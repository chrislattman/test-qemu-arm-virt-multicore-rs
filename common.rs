//! Code shared by both cores' images. Each image compiles its own copy, with its own CORE_ID
#![allow(dead_code)] // Each image uses only some of this

use crate::CORE_ID;
use crate::memory_map::*;
use core::arch::asm;
use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU32, Ordering};

pub const OWN_BASE: usize = if CORE_ID == 0 { CORE0_BASE } else { CORE1_BASE };

pub fn sleep_ms(ms: u32) {
    let frq: u64;
    let start: u64;
    let mut current: u64;

    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) frq);
        asm!("mrs {}, cntpct_el0", out(reg) start);
    }

    let ticks_needed = (frq / 1000) * ms as u64;

    loop {
        unsafe {
            asm!("mrs {}, cntpct_el0", out(reg) current);
        }
        if (current - start) >= ticks_needed {
            break;
        }
    }
}

// --- Mailbox ---

const MAILBOX_TEXT_SIZE: usize = 120;

/// Only core 0 touches the UART, so core 1 posts its messages here and core 0 prints them.
/// It holds one message at a time: core 1 waits until core 0 has printed the previous one
#[repr(C)]
pub struct Mailbox {
    full: AtomicU32, // 1 while text holds a message core 0 hasn't printed yet
    len: UnsafeCell<u32>,
    text: UnsafeCell<[u8; MAILBOX_TEXT_SIZE]>,
}

// SAFETY: len and text are only written by core 1 while full == 0, and only read by core 0 while
// full == 1. The acquire/release pair on full orders those accesses between the cores
unsafe impl Sync for Mailbox {}

impl Mailbox {
    /// The mailbox at the start of the shared block
    pub fn get() -> &'static Mailbox {
        // SAFETY: SHARED_BASE is mapped as normal memory on both cores, nothing else uses it,
        // and any bytes there make a valid Mailbox (from_utf8 in take checks the text)
        unsafe { &*(SHARED_BASE as *const Mailbox) }
    }

    /// Empties the mailbox (core 0 does this before starting core 1)
    pub fn clear(&self) {
        self.full.store(0, Ordering::Release);
    }

    /// Waits until the mailbox is empty, then posts s (truncated to fit) for core 0 to print
    pub fn post(&self, s: &str) {
        while self.full.load(Ordering::Acquire) != 0 {}

        let mut n = s.len().min(MAILBOX_TEXT_SIZE);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        // SAFETY: full == 0, so core 0 isn't reading these
        unsafe {
            let text = &mut *self.text.get();
            text[..n].copy_from_slice(&s.as_bytes()[..n]);
            *self.len.get() = n as u32;
        }

        self.full.store(1, Ordering::Release);
    }

    /// Passes the waiting message, if there is one, to f, then empties the mailbox
    pub fn take(&self, f: impl FnOnce(&str)) {
        if self.full.load(Ordering::Acquire) != 0 {
            // SAFETY: full == 1, so core 1 isn't writing these
            let text = unsafe {
                let len = (*self.len.get() as usize).min(MAILBOX_TEXT_SIZE);
                let text = &*self.text.get();
                &text[..len]
            };
            f(core::str::from_utf8(text).unwrap_or("<invalid message>\r\n"));
            self.full.store(0, Ordering::Release);
        }
    }
}

/// A fixed-size string buffer for formatting messages without a heap. Text that doesn't fit is
/// dropped
pub struct FixedBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedBuf<N> {
    pub const fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }

    pub fn as_str(&self) -> &str {
        // Only whole &strs (or char-boundary prefixes of them) are ever copied in
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> Write for FixedBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut n = s.len().min(N - self.len);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// Describes a synchronous exception from ESR_EL1, FAR_EL1 and ELR_EL1, e.g.
/// "core 1: data abort writing 0x40000000 (translation fault, level 2), ESR 0x96000046, PC 0x40201234, halting"
pub fn describe_fault(w: &mut impl Write, esr: u64, far: u64, elr: u64) -> fmt::Result {
    let ec = (esr >> 26) & 0x3F; // Exception class
    let fsc = esr & 0x3F; // Fault status code (for aborts)

    write!(w, "core {CORE_ID}: ")?;
    match ec {
        // Data / instruction abort taken without a change in EL
        0x25 | 0x21 => {
            if ec == 0x21 {
                write!(w, "instruction abort fetching {far:#X}")?;
            } else {
                let access = if esr & (1 << 6) != 0 { "writing" } else { "reading" }; // WnR bit
                write!(w, "data abort {access} {far:#X}")?;
            }
            match fsc & 0x3C {
                0x04 => write!(w, " (translation fault, level {})", fsc & 3)?,
                0x0C => write!(w, " (permission fault, level {})", fsc & 3)?,
                _ => {}
            }
        }
        _ => w.write_str("synchronous exception")?,
    }
    write!(w, ", ESR {esr:#X}, PC {elr:#X}, halting\r\n")
}
