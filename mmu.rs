//! Per-core translation tables. Each core's image has its own copy in its own private block, and
//! each core's TTBR0_EL1 points to its own copy, so the two cores see different memory maps.
//! 4 KB granule with 39-bit virtual addresses: level 1 entries map 1 GB, level 2 entries map 2 MB.
//! Virtual addresses equal physical addresses (identity mapping)

use crate::CORE_ID;
use crate::common::OWN_BASE;
use crate::memory_map::*;
use core::arch::asm;

#[repr(C, align(4096))]
struct Table([u64; 512]);

static mut L1_TABLE: Table = Table([0; 512]); // 0x00_0000_0000-0x7F_FFFF_FFFF
static mut L2_RAM: Table = Table([0; 512]); // 0x4000_0000-0x7FFF_FFFF (RAM)

// Descriptor bits
const DESC_VALID: u64 = 1 << 0;
const DESC_TABLE: u64 = 1 << 1; // Level 1: points to a level 2 table (clear = 1 GB block)
const fn desc_attr(index: u64) -> u64 {
    index << 2 // Index into MAIR_EL1
}
const DESC_SH_INNER: u64 = 3 << 8; // Inner shareable: coherent between the cores
const DESC_AF: u64 = 1 << 10; // Access flag (if clear, the first access faults)
const DESC_PXN: u64 = 1 << 53; // Not executable at EL1
const DESC_UXN: u64 = 1 << 54; // Not executable at EL0

// MAIR_EL1 attribute indexes
const ATTR_NORMAL: u64 = 0; // Normal memory, write-back cacheable
const ATTR_DEVICE: u64 = 1; // Device-nGnRnE
const MAIR_VALUE: u64 = (0xFF << (8 * ATTR_NORMAL)) | (0x00 << (8 * ATTR_DEVICE));

const NORMAL_BLOCK: u64 = DESC_VALID | desc_attr(ATTR_NORMAL) | DESC_SH_INNER | DESC_AF;
const DEVICE_BLOCK: u64 = DESC_VALID | desc_attr(ATTR_DEVICE) | DESC_AF | DESC_PXN | DESC_UXN;

const RAM_BASE: usize = 0x4000_0000;
const BLOCK_SIZE: usize = 0x20_0000;
const fn l2_index(addr: usize) -> usize {
    (addr - RAM_BASE) / BLOCK_SIZE
}

const _: () = assert!(REGION_SIZE == BLOCK_SIZE, "each region must be exactly one level 2 block");

// TCR_EL1
const TCR_T0SZ: u64 = 64 - 39; // 39-bit virtual addresses through TTBR0_EL1
const TCR_IRGN0_WB: u64 = 1 << 8; // Table walks: inner write-back cacheable
const TCR_ORGN0_WB: u64 = 1 << 10; // Table walks: outer write-back cacheable
const TCR_SH0_INNER: u64 = 3 << 12; // Table walks: inner shareable
const TCR_EPD1: u64 = 1 << 23; // No TTBR1_EL1 walks (the upper half of the address space is unused)
const TCR_IPS_40BIT: u64 = 2 << 32; // 40-bit physical addresses (the Cortex-A53's maximum)
const TCR_VALUE: u64 =
    TCR_T0SZ | TCR_IRGN0_WB | TCR_ORGN0_WB | TCR_SH0_INNER | TCR_EPD1 | TCR_IPS_40BIT;

// SCTLR_EL1
const SCTLR_M: u64 = 1 << 0; // MMU enable
const SCTLR_C: u64 = 1 << 2; // Data cache enable
const SCTLR_I: u64 = 1 << 12; // Instruction cache enable

/// Builds this core's tables and turns on the MMU and caches. Call once, first thing in main_app
pub fn mmu_init() {
    // SAFETY: Only this core can reach these tables (they're in its own block), and nothing else
    // uses them until the MMU is on
    let (l1, l2) = unsafe { (&mut (*&raw mut L1_TABLE).0, &mut (*&raw mut L2_RAM).0) };

    // Every entry starts out invalid (.bss is zeroed), so any address not mapped here faults.
    // This is a whitelist: the other core's block is simply never mapped
    if CORE_ID == 0 {
        // 0-1 GB: devices (GIC at 0x08000000, UART at 0x09000000), never executable. Only core 0
        // uses them
        l1[0] = DEVICE_BLOCK;
    }
    // 1-2 GB: RAM, mapped in 2 MB blocks by the level 2 table
    l1[1] = l2.as_ptr() as u64 | DESC_TABLE | DESC_VALID;

    l2[l2_index(OWN_BASE)] = OWN_BASE as u64 | NORMAL_BLOCK;
    l2[l2_index(SHARED_BASE)] = SHARED_BASE as u64 | NORMAL_BLOCK | DESC_PXN | DESC_UXN;

    // No `nomem` option, so these also act as compiler memory barriers (the table writes above
    // must happen first)
    unsafe {
        asm!(
            "msr  mair_el1, {mair}",
            "msr  tcr_el1, {tcr}",
            "msr  ttbr0_el1, {ttbr}",
            "isb",
            "tlbi vmalle1", // Discard any stale TLB entries
            "dsb  nsh",
            "isb",
            mair = in(reg) MAIR_VALUE,
            tcr = in(reg) TCR_VALUE,
            ttbr = in(reg) l1.as_ptr(),
            options(nostack),
        );

        let mut sctlr: u64;
        asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nostack));
        sctlr |= SCTLR_M | SCTLR_C | SCTLR_I;
        asm!("msr sctlr_el1, {}", "isb", in(reg) sctlr, options(nostack));
    }
}
