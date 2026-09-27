//! Interrupt Descriptor Table and exception/IRQ handlers.

use spin::Lazy;
use x86_64::PrivilegeLevel;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use crate::interrupts::gdt;
use crate::{println, klog};

pub use crate::interrupts::pic::InterruptIndex;

/// int 0x80 vector — our system-call gate.
pub const SYSCALL_VECTOR: u8 = 0x80;

/// Halt the CPU forever.
pub(crate) fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();

    idt.divide_error.set_handler_fn(divide_by_zero_handler);
    idt.debug.set_handler_fn(debug_handler);
    idt.non_maskable_interrupt.set_handler_fn(nmi_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.general_protection_fault.set_handler_fn(gp_fault_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);

    idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
    idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(mouse_interrupt_handler);

    // Software interrupt gate, DPL=3: user programs call `int 0x80`.
    idt[SYSCALL_VECTOR]
        .set_handler_fn(syscall_handler)
        .set_privilege_level(PrivilegeLevel::Ring3);

    idt
});

/// Load the IDT.
pub fn init() {
    IDT.load();
}

/// Is the interrupted code running in Ring 3 (user mode)?
fn in_user_mode(frame: &InterruptStackFrame) -> bool {
    frame.code_segment.rpl() == PrivilegeLevel::Ring3
}

/// Snapshot of general-purpose registers at interrupt entry.
/// `rbx`/`rbp` are reserved by LLVM (cannot be asm operands); RIP/RSP/CS/FLAGS
/// come from the interrupt frame instead.
struct GpRegs {
    rax: u64, rcx: u64, rdx: u64, rsi: u64, rdi: u64,
    r8: u64, r9: u64, r10: u64, r11: u64,
    r12: u64, r13: u64, r14: u64, r15: u64,
}

/// Read the snapshotable general-purpose registers in one asm block. They are
/// unmodified at the top of any x86-interrupt handler (LLVM's prologue only
/// pushes them, preserving their values).
fn read_gp_regs() -> GpRegs {
    let mut r = GpRegs {
        rax: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0,
        r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
    };
    unsafe {
        // Fixed-register operands cannot be referenced by placeholders; the
        // empty template just snapshots each register into the Rust variable.
        core::arch::asm!(
            "",
            lateout("rax") r.rax, lateout("rcx") r.rcx, lateout("rdx") r.rdx,
            lateout("rsi") r.rsi, lateout("rdi") r.rdi,
            lateout("r8") r.r8, lateout("r9") r.r9, lateout("r10") r.r10,
            lateout("r11") r.r11, lateout("r12") r.r12, lateout("r13") r.r13,
            lateout("r14") r.r14, lateout("r15") r.r15,
            options(nomem, nostack, preserves_flags),
        );
    }
    r
}

/// Dump a register snapshot plus interrupt frame.
/// Returns `true` when the fault happened in user mode and the interrupt frame
/// has been rewritten to resume the kernel shell — the caller MUST then return
/// normally so the x86-interrupt epilogue executes `iretq` and applies the new
/// frame. Returns `false` (kernel-mode fault) and the caller should `hlt_loop`.
///
/// Everything goes to the serial port (`klog!`), never to `println!`: a fault
/// can fire while the FB/WM/VGA locks are held, and taking any of those locks
/// from an exception handler would deadlock the kernel.
fn dump_exception(name: &str, frame: &mut InterruptStackFrame, err: Option<u64>) -> bool {
    let regs = read_gp_regs();
    let mode = if in_user_mode(frame) { "USER mode" } else { "KERNEL mode" };
    klog!("[EXCEPTION] {} — {}!\n", name, mode);
    klog!("  RIP = {:#x}   CS = {:#x}   RSP = {:#x}\n",
        frame.instruction_pointer.as_u64(),
        frame.code_segment.0,
        frame.stack_pointer.as_u64());
    if let Some(e) = err {
        klog!("  error code = {:#x}\n", e);
    }
    klog!("  RAX={:#018x} RCX={:#018x} RDX={:#018x}\n",
        regs.rax, regs.rcx, regs.rdx);
    klog!("  RSI={:#018x} RDI={:#018x} R8 ={:#018x} R9 ={:#018x}\n",
        regs.rsi, regs.rdi, regs.r8, regs.r9);
    klog!("  R10={:#018x} R11={:#018x} R12={:#018x} R13={:#018x}\n",
        regs.r10, regs.r11, regs.r12, regs.r13);
    klog!("  R14={:#018x} R15={:#018x}\n",
        regs.r14, regs.r15);

    // User-mode faults must never take down the whole OS: rewrite the frame to
    // jump back to the kernel shell, then let the caller return so `iretq` runs.
    if in_user_mode(frame) {
        crate::user::recover_user_fault(frame, name);
        return true;
    }
    false
}

// ---- Exception handlers ----

/// Shared tail: recovered (user fault) -> return so `iretq` applies the new
/// frame; not recovered (kernel fault) -> halt.
macro_rules! exception_tail {
    ($recovered:expr) => {
        if $recovered {
            // fall through: normal return -> CPU iretq to kernel_resume
        } else {
            hlt_loop()
        }
    };
}

extern "x86-interrupt" fn divide_by_zero_handler(mut stack: InterruptStackFrame) {
    exception_tail!(dump_exception("Divide by zero", &mut stack, None));
}

extern "x86-interrupt" fn debug_handler(mut stack: InterruptStackFrame) {
    exception_tail!(dump_exception("Debug exception", &mut stack, None));
}

extern "x86-interrupt" fn nmi_handler(mut stack: InterruptStackFrame) {
    exception_tail!(dump_exception("Non-maskable interrupt", &mut stack, None));
}

extern "x86-interrupt" fn breakpoint_handler(mut stack: InterruptStackFrame) {
    exception_tail!(dump_exception("Breakpoint", &mut stack, None));
}

extern "x86-interrupt" fn double_fault_handler(mut stack: InterruptStackFrame, err: u64) -> ! {
    dump_exception("DOUBLE FAULT", &mut stack, Some(err));
    hlt_loop()
}

extern "x86-interrupt" fn invalid_opcode_handler(mut stack: InterruptStackFrame) {
    exception_tail!(dump_exception("Invalid opcode", &mut stack, None));
}

extern "x86-interrupt" fn gp_fault_handler(mut stack: InterruptStackFrame, err: u64) {
    exception_tail!(dump_exception("General protection fault", &mut stack, Some(err)));
}

extern "x86-interrupt" fn page_fault_handler(mut stack: InterruptStackFrame, err: PageFaultErrorCode) {
    use x86_64::registers::control::Cr2;
    let cr2 = Cr2::read();
    // Serial-only output: a fault can land while FB/WM/VGA locks are held.
    klog!("[EXCEPTION] Page fault!\n");
    klog!("  CR2 (faulting address) = {:#x}\n", cr2.unwrap_or_else(|_| x86_64::VirtAddr::new(0)).as_u64());
    klog!("  error flags: {:?}\n", err);
    let mode = if in_user_mode(&stack) { "USER mode" } else { "KERNEL mode" };
    klog!("  mode = {}\n", mode);
    klog!("  RIP = {:#x}   RSP = {:#x}\n",
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64());
    let regs = read_gp_regs();
    klog!("  RAX={:#018x} RCX={:#018x} RDX={:#018x}\n",
        regs.rax, regs.rcx, regs.rdx);
    klog!("  RSI={:#018x} RDI={:#018x} R8 ={:#018x} R9 ={:#018x}\n",
        regs.rsi, regs.rdi, regs.r8, regs.r9);
    klog!("  R10={:#018x} R11={:#018x} R12={:#018x} R13={:#018x}\n",
        regs.r10, regs.r11, regs.r12, regs.r13);
    klog!("  R14={:#018x} R15={:#018x}\n",
        regs.r14, regs.r15);
    exception_tail!(in_user_mode(&stack) && {
        crate::user::recover_user_fault(&mut stack, "page fault");
        true
    });
}

// ---- IRQ handlers ----

extern "x86-interrupt" fn timer_interrupt_handler(stack: InterruptStackFrame) {
    crate::drivers::timer::tick();
    crate::interrupts::pic::notify_eoi(InterruptIndex::Timer);

    // Wake sleeping tasks in any mode.
    crate::task::wake_blocked();

    // Preempt only while running kernel (Ring 0) tasks. A user program owns
    // the CPU until it traps back via int 0x80 or exits.
    if !in_user_mode(&stack) {
        crate::task::switch_to_next();
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack: InterruptStackFrame) {
    crate::drivers::keyboard::handle_irq();
    crate::interrupts::pic::notify_eoi(InterruptIndex::Keyboard);
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack: InterruptStackFrame) {
    crate::drivers::mouse::handle_irq();
    crate::interrupts::pic::notify_eoi(InterruptIndex::Mouse);
}

// ---- System calls (int 0x80, callable from Ring 3) ----

/// Syscall numbers shared with the userspace crate.
pub mod syscall_numbers {
    pub const SYS_WRITE: u64 = 1;
    pub const SYS_EXIT: u64 = 2;
    pub const SYS_SLEEP: u64 = 3;
    pub const SYS_READ: u64 = 4;
    pub const SYS_OPEN: u64 = 5;
    pub const SYS_CLOSE: u64 = 6;
    pub const SYS_WRITE_FD: u64 = 7;
    pub const SYS_STAT: u64 = 8;
    pub const SYS_GETPID: u64 = 9;
    pub const SYS_KILL: u64 = 10;
}

extern "x86-interrupt" fn syscall_handler(mut stack: InterruptStackFrame) {
    // Read syscall number (rax) and args (rdi, rsi, rdx) in one asm block.
    // Each output is pinned to the register it reads so LLVM's allocator
    // cannot clobber a source before it is read (a real bug we hit: `out(reg)`
    // let LLVM chain outputs through rdi→rsi→rdx→rcx).
    let (n, a, b, c): (u64, u64, u64, u64);
    unsafe {
        // Snapshot rax/rdi/rsi/rdx (user's syscall number + args).
        core::arch::asm!(
            "",
            lateout("rax") n, lateout("rdi") a, lateout("rsi") b, lateout("rdx") c,
            options(nomem, nostack, preserves_flags),
        );
    }

    let ret = crate::user::syscall::dispatch(n, a, b, c, &mut stack);

    // Write the return value into RAX; data dependency (ret derives from the
    // register reads above) keeps the asm blocks ordered.
    unsafe {
        core::arch::asm!(
            "mov rax, {}",
            in(reg) ret,
            options(nomem, nostack, preserves_flags),
        );
    }
}
