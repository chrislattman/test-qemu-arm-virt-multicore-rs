#!/bin/sh
# Cargo runner (see .cargo/config.toml). `cargo run` only builds core0 and passes its ELF as the
# last argument, so this also builds core1 with the same profile, then starts QEMU with both.
# --gdb (first argument) halts QEMU at startup and waits for GDB on localhost:1234
set -e

gdb=
if [ "$1" = "--gdb" ]; then
    gdb="-S -gdb tcp:localhost:1234"
    shift
fi

core0=$1
dir=$(dirname "$core0")
if [ "$(basename "$dir")" = release ]; then
    "${CARGO:-cargo}" build --quiet --release --bin core1
else
    "${CARGO:-cargo}" build --quiet --bin core1
fi

# -smp 2: two Cortex-A53 cores. -kernel starts core 0 at core0's entry point, and the generic
# loader device copies core1 into core 1's block without starting core 1 (core 0 does that with
# PSCI). In another shell: telnet localhost 5678
# shellcheck disable=SC2086
exec qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a53 \
    -smp 2 \
    -nographic \
    -serial telnet:localhost:5678,server=on,wait=off \
    -kernel "$core0" \
    -device loader,file="$dir/core1" \
    $gdb
