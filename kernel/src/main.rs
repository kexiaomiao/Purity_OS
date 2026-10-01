//! PurityOS kernel entry point.
//!
//! A from-scratch, pure-Rust x86_64 kernel. The `bootloader` crate hands us
//! control in 64-bit long mode with paging already enabled; we then set up
//! the GDT/IDT/PIC, drivers, heap, GUI (framebuffer + window manager), and
//! finally the multitasking shell.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

#[macro_use]
extern crate alloc;

mod drivers;
mod fs;
mod gui;
mod interrupts;
mod lampgl;
mod mem;
mod shell;
mod task;
mod user;
mod utf8;

use bootloader_api::entry_point;

/// Bootloader configuration: request a framebuffer for the GUI desktop.
/// (The `frame_buffer` field is deprecated in 0.11.1 in favour of disk-image
/// BootConfig, but the entry-point config still works and keeps the kernel
/// self-contained.)
#[allow(deprecated)]
const BOOT_CONFIG: bootloader_api::BootloaderConfig = {
    use bootloader_api::config::Mapping;
    let mut config = bootloader_api::BootloaderConfig::new_default();
    config.frame_buffer.minimum_framebuffer_width = Some(1280);
    config.frame_buffer.minimum_framebuffer_height = Some(720);
    // Map all physical memory into the higher half so the kernel can build
    // page tables (BootInfo::physical_memory_offset would be None otherwise).
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOT_CONFIG);

/// A dedicated kernel stack is not needed: the bootloader's boot stack is
/// fine, but its top sits flush against an unmapped page and generated code
/// writes at positive RSP offsets near the top. We map writable headroom above
/// it right after paging is set up (see `mem::map_headroom_writable`).

fn kernel_main(boot_info: &'static mut bootloader_api::BootInfo) -> ! {
    // Earliest output paths.
    drivers::uart::init();
    // NOTE: do NOT touch the VGA text buffer at 0xb8000 yet — the bootloader
    // may not have mapped it into kernel space, and GDT/IDT aren't installed
    // yet, so a page fault here would triple-fault with no handler. The GUI
    // framebuffer takes over later; VGA output is only used as a fallback.
    interrupts::gdt::init();
    interrupts::idt::init();
    interrupts::pic::init();
    drivers::timer::init();
    drivers::rtc::init();
    crate::klog!("PurityOS kernel starting...\n");

    // Memory + heap (must come before anything that allocates). We first pull
    // the framebuffer out of the boot info so both subsystems can be set up.
    let fbuf = boot_info.framebuffer.take();
    mem::init(boot_info);
    // The boot stack top sits flush against an unmapped page; map writable
    // headroom above it so kernel_main's positive-RSP-offset writes survive.
    mem::map_headroom_writable(0x1000_0000_0000, 256 * 1024);

    // Restore PurityFS from the ATA disk. This must run BEFORE the scheduler
    // starts (task::init), while interrupts are still off and the PIT has no
    // chance to switch stacks. The reader is bounded: it probes LBA 100 for the
    // PFS1 superblock magic and reads at most 32 sectors (16 KiB), and returns
    // immediately on an empty disk without allocating.
    fs::load_from_disk();

    // Keyboard + timer interrupts now live.
    x86_64::instructions::interrupts::enable();

    // GUI: framebuffer + mouse + window manager + desktop.
    let gui_on = gui::start(fbuf);
    if gui_on {
        crate::klog!("GUI: framebuffer initialized, desktop ready.\n");
    } else {
        crate::klog!("GUI: no framebuffer, falling back to text mode.\n");
    }

    crate::println!("PurityOS kernel ready.");
    crate::klog!("PurityOS kernel ready.\n");

    // --- Automated Ring-3 link test: preload keyboard input BEFORE the
    // scheduler starts, so the user shell can read it without real hardware.
    // "echo test" then "fork" (execs /bin/forktest which calls SYS_GETPID and
    // SYS_FORK and prints the parent/child branches).
    crate::drivers::keyboard::preload(b"echo test\nfork\n");

    // Start the scheduler; the shell runs as task #1. This never returns.
    task::init(shell::run);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Lock-free: print PANIC straight to the UART port, bypassing every lock.
    unsafe {
        drivers::uart::early_init();
        drivers::uart::early_print("\n\n*** PANIC! (early) ***\n");
        drivers::uart::early_print_fmt(format_args!("{}\n", info));
    }
    // Rapid alarm beep before we die.
    crate::drivers::speaker::panic_alarm();
    // Serial-only: a panic can fire while FB/WM/VGA locks are held, so taking
    // any of those locks here would deadlock the kernel.
    crate::klog!("\n\n*** KERNEL PANIC: {} ***\n", info);
    crate::klog!("*** System halted ***\n");

    loop {
        x86_64::instructions::hlt();
    }
}
