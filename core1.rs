//! Core 1: uses no peripherals. It posts its messages to the mailbox in the shared block, and
//! core 0 prints them
#![no_std]
#![no_main]

mod common;
mod memory_map;
mod mmu;

use common::{FixedBuf, Mailbox, describe_fault};
use core::fmt::Write;
use core::panic::PanicInfo;

const CORE_ID: usize = 1;

// Core Entry Point Setup
core::arch::global_asm!(include_str!("startup.S"), core_id = const CORE_ID);

// Core 1 never enables any interrupts in the GIC, but the vector table in startup.S needs this
#[unsafe(no_mangle)]
pub extern "C" fn handle_irq() {}

// Synchronous exceptions (e.g. a data abort from an unmapped address), called from sync_entry in
// startup.S, which halts this core afterwards
#[unsafe(no_mangle)]
pub extern "C" fn handle_sync(esr: u64, far: u64, elr: u64) {
    let mut buf = FixedBuf::<120>::new();
    let _ = describe_fault(&mut buf, esr, far, elr);
    Mailbox::get().post(buf.as_str());
}

#[unsafe(no_mangle)]
pub extern "C" fn main_app() -> ! {
    mmu::mmu_init();
    let mailbox = Mailbox::get();
    mailbox.post("core 1: MMU on\r\n");

    loop {
        mailbox.post("Hello from core 1\r\n");
        #[cfg(not(debug_assertions))]
        common::sleep_ms(3000);
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Core 1 has no UART, so the panic message goes through the mailbox (truncated to fit)
    let mut buf = FixedBuf::<120>::new();
    let _ = write!(buf, "core 1: PANIC: {}\r\n", info.message());
    Mailbox::get().post(buf.as_str());
    loop {}
}
