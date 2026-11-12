// Memory map, used by both cores' images and by build.rs (which passes it to the linker).
// Each region is one 2 MB block in the MMU's level 2 table:
//
//   0x40000000  CORE0_BASE   Core 0: the DTB (QEMU puts it at the RAM base), core0 at +0x80000,
//                            its stack and translation tables
//   0x40200000  CORE1_BASE   Core 1: core1, its stack and translation tables
//   0x40400000  SHARED_BASE  Shared by both cores: the mailbox
//
// Each core maps only its own block and the shared block (core 0 also maps the devices), so any
// access to the other core's block is a translation fault (see mmu.rs)
#![allow(dead_code)] // Each user needs only some of these

pub const CORE0_BASE: usize = 0x4000_0000;
pub const CORE1_BASE: usize = 0x4020_0000;
pub const SHARED_BASE: usize = 0x4040_0000;
pub const REGION_SIZE: usize = 0x20_0000;

/// Where core 0's image is linked: 0x80000 into its block, after the DTB
pub const CORE0_IMAGE: usize = CORE0_BASE + 0x8_0000;
