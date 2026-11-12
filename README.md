# no_std Rust example for QEMU ARM "virt" machine with two Cortex-A53 cores

Each core runs its own image from its own private 2 MB block of RAM, and has its own MMU translation tables that map only its own block, a shared block and (core 0 only) the devices. If either core touches the other core's block, it gets a translation fault.

- Core 0 owns the UART and the GIC (through the `arm-pl011-uart` and `arm-gic` crates, as in the single-core version). It runs the same Hello/World loop and SGI cascade, starts core 1, and prints core 1's messages.
- Core 1 uses no peripherals. It posts a message to a mailbox in the shared block every 3 seconds, and core 0 prints it after each loop iteration (every 2 seconds), so core 1's messages can appear up to 2 seconds late.

Run `sudo apt install qemu-system-arm gdb-multiarch`

Build and run (`.cargo/config.toml` runs QEMU through `qemu.sh`):

```
cargo qemu        # release build; in another shell: telnet localhost 5678
cargo qemu-debug  # debug build; QEMU halts and waits for GDB on localhost:1234
```

The debug build skips the interrupts and sleeps like the single-core version. The VS Code "Rust: Remote Attach" configuration loads both images' symbols (`add-symbol-file` for `core1`), and each core shows up as a thread. It also sets `scheduler-locking step`, so stepping one core keeps the other halted. Otherwise GDB resumes both cores for every step, and a breakpoint on the other core can cut the step short. Continue still resumes both. QEMU has no non-stop mode, so whenever one core is stopped, the other is too. `.cargo/config.toml` has the equivalent GDB commands.

If either core touches the other core's block, it reports the fault and halts, for example:

```
core 1: data abort writing 0x40000000 (translation fault, level 2), ESR 0x96000046, PC 0x40201870, halting
```

## Memory map

| Block | Address | Contents | Mapped by |
|---|---|---|---|
| Core 0 | 0x40000000 | DTB (QEMU puts it at the RAM base), `core0` at +0x80000, its stack and translation tables | Core 0 |
| Core 1 | 0x40200000 | `core1`, its stack and translation tables | Core 1 |
| Shared | 0x40400000 | The mailbox | Both, never executable |
| Devices | 0x00000000-0x3FFFFFFF | GIC at 0x08000000, UART at 0x09000000 | Core 0, as Device-nGnRnE, never executable |

The addresses are defined once, in `memory_map.rs`. Both images use it as a module, and `build.rs` includes it too, to pass each image's addresses to the linker (`--defsym`, with `cargo::rustc-link-arg-bin`). `linker.ld` asserts that each image, including its stack, fits in its block. Everything else is unmapped on both cores.

## How it works

- **Two binaries.** `core0.rs` and `core1.rs` are separate crate roots (`[[bin]]` targets), and each includes `common.rs`, `memory_map.rs` and `mmu.rs` as modules with its own `CORE_ID` constant. Each image has its own copy of every static, its own stack and its own translation tables, all inside its own block. `startup.S` is shared too: `global_asm!` substitutes `{core_id}`.
- **Building and running both.** `cargo run` only builds the binary it runs (`core0`, the `default-run`), so the runner, `qemu.sh`, builds `core1` with the same profile before starting QEMU. QEMU loads `core0` with `-kernel` and `core1` with `-device loader,file=...`, which only copies it into memory.
- **Starting core 1.** With `-kernel` and no firmware, QEMU starts core 0 only. Core 1 is powered off until core 0 calls PSCI `CPU_ON` (`psci_cpu_on` in `core0.rs`, an `hvc #0` call, which QEMU handles itself since there's no EL2 or EL3).
- **One MMU per core.** Each core has its own `TTBR0_EL1`, `TCR_EL1`, `MAIR_EL1` and `SCTLR_EL1`, and each core points `TTBR0_EL1` at its own tables in its own block (`mmu.rs`). Every table entry starts out invalid, and each core fills in only what it uses, so the protection is a whitelist: the other core's block is simply never mapped.
- **The mailbox** (`Mailbox` in `common.rs`) sits at the start of the shared block, which is normal write-back cacheable, inner shareable memory on both cores, so it's coherent between them. Its `full` flag is an `AtomicU32` with acquire/release ordering, so the text is always written before the flag is set, and read after it's seen. Messages are `&str`s; `FixedBuf` formats messages with `write!` without a heap.
- **Faults** go to `sync_entry` in `startup.S`, which passes `ESR_EL1`, `FAR_EL1` and `ELR_EL1` to `handle_sync`. Core 0 prints the fault itself; core 1 posts it to the mailbox. Core 1's panic handler also reports through the mailbox.

## Limitations

- **The protection is cooperative.** Both cores run at EL1, which owns its own translation tables, so a core could map the other core's block again by rewriting its own tables. This catches stray pointers and buffer overruns, not deliberate code. To enforce it, a higher exception level has to own the tables: with `-M virt,virtualization=on`, an EL2 hypervisor could give each core stage 2 translation tables that EL1 can't change (QEMU supports this, but this project doesn't do it).
- **Rust's guarantees stop at each image.** The compiler checks each binary on its own, so it can't see that both cores share the mailbox. That's why `Mailbox` uses atomics and `UnsafeCell`, with the cross-core rules in its `SAFETY` comments.
- **The MMU only checks the CPU's own accesses.** DMA from devices isn't checked (the virt machine has an SMMUv3 for that, which isn't used here).
- **QEMU doesn't model caches**, so it can't catch a wrong cacheability attribute. Before turning the MMU and caches on, real hardware also needs its caches invalidated, which QEMU doesn't need and this project doesn't do.
- **The debugger sees each core's own view of memory.** GDB reads memory through the selected thread's MMU, so with core 0 selected, reading 0x40200000 fails with "Cannot access memory". Switch to core 1's thread (`thread 2`) to read core 1's block. Breakpoints work on either core, since QEMU handles them itself.
- **Debug builds spin.** Core 1 waits in `Mailbox::post` for core 0 to empty the mailbox, and `sleep_ms` busy-waits, so each QEMU vCPU uses a full host core.

## Notes

- The `-softfloat` target keeps the compiler from using the FP/SIMD registers, like `-mgeneral-regs-only` in C. The FPU is disabled at EL1, and `irq_entry` doesn't save those registers
- Serial output is lost until a telnet client connects; change `wait=off` to `wait=on` in `qemu.sh` to make QEMU wait for the connection before starting
- `core0` is passed straight to `-kernel` as an ELF, so QEMU loads it at its linked address (0x40080000, after the device tree at the RAM base) and jumps to `_start`
