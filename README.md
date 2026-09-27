# PurityOS v0.9.0

A from-scratch, **single-threaded-but-preemptive** x86_64 operating system written in
**pure Rust**, booted by the `bootloader` crate. This release is the first build
that has been verified to boot end-to-end under QEMU all the way to a graphical
desktop.

![status](https://img.shields.io/badge/status-boots%20to%20GUI-brightgreen)
![arch](https://img.shields.io/badge/arch-x86__64-blue)
![lang](https://img.shields.io/badge/lang-Rust-orange)

---

## What works in this build

### Boot & CPU
- BIOS boot via `bootloader_api` 0.11 (higher-half kernel, recursive paging)
- `BOOT_CONFIG` enables `mappings.physical_memory = Dynamic` so the kernel can
  walk its own page tables
- GDT (kernel + user 3/3, TSS with RSP0), IDT with full exception handlers
  that dump every register on faults
- PIC remap, PIT @ 100 Hz, CMOS/RTC real-time clock, PC speaker
- Lock-free early serial (`outb 0x3F8`) that works before the heap, paging,
  or locks are up — used to trace the boot sequence and debug panics

### Memory
- Offset page table on top of the bootloader's mappings
- Boot-info frame allocator (monotonic, never double-allocates a physical frame)
- 256 KiB bump/linked-list heap
- `VirtAlloc` — automatic user virtual-address range carve-out, no hard-coded
  user addrs (fixes the "地址乱堆" requirement)
- User pages mapped `USER_ACCESSIBLE`; kernel pages are never user-accessible

### Multitasking
- Kernel threads with `PurityOS_switch` (naked asm context switch)
- Round-robin + priority scheduler, preempted by the PIT IRQ
- `HeldSched` guard that records *and restores* the prior interrupt state
  (fixes the "interrupts re-enabled inside an IRQ" deadlock)
- `sleep_ms` blocks a task instead of busy-spinning

### Filesystem — PurityFS
- In-memory hierarchical FS (file / dir / symlink / device nodes)
- `resolve` / `..` / symlink following, `mv`, `rm -r`, `chmod`-style mode bits
- **ATA PIO disk persistence**: superblock at LBA 100, magic `PFS1`, bounded
  read (≤ 32 sectors = 16 KiB) — no more 4 MiB blind read against a 256 KiB heap
- `flush_to_disk()` / `load_from_disk()`

### Userspace (Ring 3)
- `int 0x80` syscall gate (DPL=3): `SYS_WRITE`, `SYS_READ`, `SYS_EXIT`,
  `SYS_SLEEP`, `SYS_OPEN/CLOSE/READ_FILE/WRITE_FILE/STAT`, `SYS_GETPID`,
  `SYS_KILL`
- GDT user segments + TSS RSP0, user stack via `alloc_user_region`
- ELF loader for the embedded user program

### GUI
- Linear framebuffer driver, software window manager (title bars, drag,
  focus, minimize, close, right-click menus)
- Built-in apps: terminal, file manager, text editor, calculator, paint,
  system monitor, hardware info, LampGL 3D demo
- Mouse (PS/2), keyboard routing to the focused window, Shift support
- **LampGL** software rasterizer (triangle fill via barycentric coordinates,
  reused z-buffer) — built-in 3D demo

---

## Running it

You need `qemu-system-x86_64`. Then:

```bash
# the prebuilt image
cp PurityOS-v0.9.0.img /tmp/purityos.img
qemu-system-x86_64 \
  -drive format=raw,file=/tmp/purityos.img \
  -serial stdio -m 512M -display none -no-reboot
```

You should see (on the serial console):

```
[kernel] entry reached
[kernel] stage: uart::init
[kernel] stage: gdt init
[kernel] stage: idt init
[kernel] stage: pic init
[kernel] stage: timer/rtc init
[kernel] stage: mem init
PurityOS kernel starting...
[kernel] stage: interrupts enable
GUI: framebuffer initialized, desktop ready.
PurityOS kernel ready.
```

A graphical desktop comes up on the framebuffer (QEMU window). Use `-nographic`
*only* if you also drop `-serial stdio` — they conflict.

## Building from source

```bash
rustup target add x86_64-unknown-none
cargo run -p xtask -- build
# image lands at target/purityos-bios.img
```

Workspace: `kernel/` + `xtask/`.

## Layout

```
kernel/src/
  main.rs         entry, boot config, panic handler, boot stages
  mem/            paging, frame allocator, heap, VirtAlloc
  interrupts/     gdt, idt, pic, syscall gate
  drivers/        uart, vga, pit, rtc, keyboard, mouse, ata, speaker
  fs/             PurityFS + ATA persistence
  task.rs         scheduler + context switch
  user.rs         Ring 3 / syscalls / ELF
  gui/            fb, window manager, apps
  lampgl.rs       software 3D rasterizer
  shell/          psh command-line
  utf8.rs         UTF-8 decoding
```

## Notes / known limitations
- Heap is 256 KiB; the 3D demo keeps its z-buffer small to fit.
- Persistence loads *lazily* from the idle task (not from `kernel_main`), because
  doing ATA PIO before the scheduler was live caused double-faults.
- Network / SMP / AHCI / USB / ELF dynamic loading are future work.

## License
MIT
