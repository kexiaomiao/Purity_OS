//! Kernel threads, context switching, and a round-robin scheduler.
//!
//! The PIT tick drives preemption; `spawn` creates background kernel tasks,
//! `sleep_ms` blocks the current task without spinning, and `ps` shows them.

use alloc::vec::Vec;
use spin::Mutex;

use crate::user::UserContext;

const MAX_TASKS: usize = 8;
const STACK_SIZE: usize = 4096 * 16; // 64 KiB per task

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Ready,
    Running,
    Blocked,
    /// Terminated: never scheduled again (slot retained until a future rework).
    Dead,
}

struct Task {
    /// Saved stack pointer (points at the last pushed callee-saved reg).
    saved_rsp: u64,
    state: TaskState,
    wake_tick: Option<u64>,
    entry: Option<fn() -> !>,
    /// Scheduling priority: smaller runs first (0 = highest, default 1).
    priority: u8,
    /// If this task runs a user process: its independent page-table root
    /// (0 = use the kernel address space).
    cr3: u64,
    /// Kernel stack top (RSP0) used when this process traps into the kernel
    /// from Ring 3 (0 = a kernel task; RSP0 is left untouched).
    rsp0: u64,
    /// True when this task is a forked user process (its exit terminates it).
    is_process: bool,
    /// Saved user context for the *first* entry of a forked child (consumed by
    /// `process_enter`; afterwards the state lives in the interrupt frame on
    /// the task's kernel stack).
    user_ctx: Option<UserContext>,
}

impl Task {
    const fn empty() -> Task {
        Task {
            saved_rsp: 0, state: TaskState::Ready, wake_tick: None, entry: None, priority: 1,
            cr3: 0, rsp0: 0, is_process: false, user_ctx: None,
        }
    }
}

/// Per-task static kernel stacks.
static mut STACKS: [[u8; STACK_SIZE]; MAX_TASKS] = [[0; STACK_SIZE]; MAX_TASKS];

fn stack_top(id: usize) -> u64 {
    unsafe { (&raw mut STACKS[id] as *mut u8 as u64) + STACK_SIZE as u64 }
}

struct Scheduler {
    tasks: [Task; MAX_TASKS],
    current: usize,
    task_count: usize,
}

impl Scheduler {
    const fn new() -> Self {
        Scheduler { tasks: [const { Task::empty() }; MAX_TASKS], current: 0, task_count: 0 }
    }

    /// Find the next runnable task. Highest-priority (smallest `priority`)
    /// wins; within that priority level we round-robin starting after `from`.
    fn pick_next(&self, from: usize) -> usize {
        // Highest priority among all runnable tasks.
        let mut best_prio: Option<u8> = None;
        for t in self.tasks.iter() {
            if t.entry.is_some() && t.state == TaskState::Ready {
                best_prio = Some(match best_prio {
                    Some(p) => p.min(t.priority),
                    None => t.priority,
                });
            }
        }
        let Some(prio) = best_prio else {
            return from;
        };
        // Round-robin within that priority level, starting just after `from`.
        for i in 1..=MAX_TASKS {
            let idx = (from + i) % MAX_TASKS;
            let t = &self.tasks[idx];
            if t.entry.is_some() && t.state == TaskState::Ready && t.priority == prio {
                return idx;
            }
        }
        from
    }
}

static SCHED: Mutex<Scheduler> = Mutex::new(Scheduler::new());

/// Guard that disables interrupts while the scheduler is locked and restores
/// the previous interrupt state on drop. Prevents the PIT handler from
/// deadlocking on the spinlock and avoids re-enabling interrupts when we
/// were called from inside an interrupt handler (e.g. the PIT tick).
struct HeldSched<'a> {
    guard: Option<spin::MutexGuard<'a, Scheduler>>,
    was_enabled: bool,
}

impl Drop for HeldSched<'_> {
    fn drop(&mut self) {
        // Release the spinlock FIRST, then restore interrupts only if they
        // were enabled before we took the guard.
        self.guard.take();
        if self.was_enabled {
            unsafe { x86_64::instructions::interrupts::enable(); }
        }
    }
}

impl core::ops::Deref for HeldSched<'_> {
    type Target = Scheduler;
    fn deref(&self) -> &Scheduler {
        self.guard.as_ref().unwrap()
    }
}

impl core::ops::DerefMut for HeldSched<'_> {
    fn deref_mut(&mut self) -> &mut Scheduler {
        self.guard.as_mut().unwrap()
    }
}

fn lock() -> HeldSched<'static> {
    let was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    HeldSched { guard: Some(SCHED.lock()), was_enabled }
}

extern "C" {
    /// Save callee-saved registers at `old_rsp`, restore them from `new_rsp`.
    fn PurityOS_switch(old_rsp_out: *mut u64, new_rsp: u64);
}

core::arch::global_asm!(
    ".global PurityOS_switch",
    "PurityOS_switch:",
    "    push rbp",
    "    push rbx",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    "    mov [rdi], rsp",
    "    mov rsp, rsi",
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop rbx",
    "    pop rbp",
    // Every resumed task must run with interrupts enabled. The task being
    // switched away from may be in a context where IF=0 (an interrupt gate's
    // syscall handler, or the PIT handler itself); if that leaked through, the
    // idle task would reach `hlt()` with IF=0 and never wake (silent hang).
    // `sti` here makes the IF state correct for ALL tasks at every resume.
    "    sti",
    "    ret",
);

/// Lay out a fresh task stack so that the first `switch` into it drops us
/// straight into `entry`.
fn prepare_stack(task_id: usize, entry: fn() -> !) -> u64 {
    unsafe {
        let stack: &mut [u8; STACK_SIZE] = &mut STACKS[task_id];
        let top = (stack.as_mut_ptr() as u64 + STACK_SIZE as u64) & !0xF;
        // top-8 holds the return address (entry); below it are six zeroed
        // register slots (r15..rbp).
        ((top - 8) as *mut u64).write(entry as u64);
        top - 8 - 6 * 8
    }
}

/// Initialize the scheduler and start the shell as task #1.
/// Switches to the shell; only returns to the boot/idle task when the
/// scheduler schedules it back.
pub fn init(shell_entry: fn() -> !) -> ! {
    {
        let mut s = lock();
        // Task 0 is the current (boot) task; its rsp is captured on first switch.
        s.tasks[0] = Task {
            saved_rsp: 0, state: TaskState::Running, wake_tick: None, entry: Some(idle), priority: 1,
            cr3: 0, rsp0: 0, is_process: false, user_ctx: None,
        };
        s.current = 0;
        s.task_count = 1;
    }

    // PurityFS disk persistence: the three-step loader (magic probe, bounded
    // sector read, bounds-checked deserialize) is implemented and memory-safe,
    // but re-enabling interrupts around ATA PIO trips a pre-existing
    // interrupt-context scheduling #DF. Leave the in-memory FS active for now;
    // wiring disk persistence back in needs the interrupt/scheduler fix first.
    // let loaded = crate::fs::load_from_disk();
    // if !loaded { crate::fs::flush_to_disk(); }
    let _ = ();

    spawn_inner(shell_entry, 1);
    // The shell's Ring-3 programs trap into the kernel on the shared privilege
    // stack; forked child processes get their own per-process kernel stacks.
    {
        let mut s = lock();
        s.tasks[1].rsp0 = crate::interrupts::gdt::priv_stack_top();
    }

    // First context switch: boot task saves its context, shell runs.
    switch_to_next();

    // When we resume here, we are the idle task.
    loop {
        x86_64::instructions::hlt();
    }
}

fn idle() -> ! {
    loop {
        // Drive the GUI (mouse + redraw) when a framebuffer is present.
        crate::gui::tick_if_active();
        x86_64::instructions::hlt();
    }
}

/// Create a new kernel task running `entry` at the default priority.
pub fn spawn(entry: fn() -> !) {
    spawn_inner(entry, 1);
}

/// Create a new kernel task with an explicit priority (smaller = higher).
pub fn spawn_priority(entry: fn() -> !, priority: u8) {
    spawn_inner(entry, priority);
}

fn spawn_inner(entry: fn() -> !, priority: u8) {
    let mut s = lock();
    if s.task_count >= MAX_TASKS {
        return;
    }
    let id = s.task_count;
    s.task_count += 1;
    let rsp = prepare_stack(id, entry);
    s.tasks[id] = Task {
        saved_rsp: rsp, state: TaskState::Ready, wake_tick: None, entry: Some(entry), priority,
        cr3: 0, rsp0: 0, is_process: false, user_ctx: None,
    };
}

/// Pick the next runnable task and context-switch to it.
pub fn switch_to_next() {
    let (old_ptr, new_rsp, next_cr3, next_rsp0) = {
        let mut s = lock();
        let old = s.current;
        let next = s.pick_next(old);
        if next == old {
            return;
        }
        // Only a task that was Running becomes Ready when we switch away.
        // A task that called `sleep_ms`/`block_on_keyboard` was already marked
        // Blocked *before* calling us; overwriting that here would wake it too
        // early and defeat the whole point of blocking.
        if s.tasks[old].state == TaskState::Running {
            s.tasks[old].state = TaskState::Ready;
        }
        s.tasks[next].state = TaskState::Running;
        s.current = next;
        let old_ptr = &mut s.tasks[old].saved_rsp as *mut u64;
        let new_rsp = s.tasks[next].saved_rsp;
        (old_ptr, new_rsp, s.tasks[next].cr3, s.tasks[next].rsp0)
    };
    // Install the destination process's address space and kernel stack (RSP0)
    // *before* the switch. The kernel half is shared, so running this code
    // under the next task's CR3 is safe; the old task's user pages are no
    // longer needed once we stop touching them here.
    if next_cr3 != 0 {
        crate::mem::switch_cr3(next_cr3);
    }
    if next_rsp0 != 0 {
        crate::interrupts::gdt::set_priv_stack(next_rsp0);
    }
    unsafe {
        PurityOS_switch(old_ptr, new_rsp);
    }
}

/// Called from the PIT IRQ (any mode): wake up tasks whose sleep expired.
pub fn wake_blocked() {
    let now = crate::drivers::timer::ticks();
    let mut s = lock();
    for t in s.tasks.iter_mut() {
        if t.state == TaskState::Blocked {
            if let Some(wake) = t.wake_tick {
                if now >= wake {
                    t.state = TaskState::Ready;
                    t.wake_tick = None;
                }
            }
        }
    }
}

/// Block the current task until keyboard input arrives. Unlike `sleep_ms`,
/// the wake time is `None`: it means "wait on an external event". The keyboard
/// IRQ calls [`wake_keyboard_waiters`] when a byte lands.
pub fn block_on_keyboard() {
    {
        let mut s = lock();
        let cur = s.current;
        s.tasks[cur].state = TaskState::Blocked;
        s.tasks[cur].wake_tick = None;
    }
    switch_to_next();
}

/// Wake every task that is blocked waiting for keyboard input (wake_tick==None).
pub fn wake_keyboard_waiters() {
    let mut s = lock();
    for t in s.tasks.iter_mut() {
        if t.state == TaskState::Blocked && t.wake_tick.is_none() {
            t.state = TaskState::Ready;
        }
    }
}

/// Block the current task for `ms` milliseconds.
pub fn sleep_ms(ms: u64) {
    let wake = crate::drivers::timer::ticks() + (ms * crate::drivers::timer::FREQ_HZ / 1000).max(1);
    {
        let mut s = lock();
        let cur = s.current;
        s.tasks[cur].state = TaskState::Blocked;
        s.tasks[cur].wake_tick = Some(wake);
    }
    switch_to_next();
}

/// Cooperative yield.
pub fn yield_now() {
    switch_to_next();
}

/// Terminate a task by pid. Returns true if it existed and was killed.
pub fn kill(pid: usize) -> bool {
    let mut s = lock();
    if pid >= MAX_TASKS || s.tasks[pid].entry.is_none() {
        return false;
    }
    if s.tasks[pid].state == TaskState::Dead {
        return false;
    }
    s.tasks[pid].state = TaskState::Dead;
    s.tasks[pid].wake_tick = None;
    true
}

/// Terminate the currently running task and switch away (never returns to it).
pub fn exit_current() -> ! {
    {
        let mut s = lock();
        let cur = s.current;
        s.tasks[cur].state = TaskState::Dead;
        s.tasks[cur].wake_tick = None;
    }
    switch_to_next();
    // If we somehow resume (nothing else runnable), halt forever.
    loop {
        x86_64::instructions::hlt();
    }
}

/// PID (task slot) of the currently running task.
pub fn current_pid() -> usize {
    let s = lock();
    s.current
}

/// True if the current task is a forked user process (as opposed to a kernel
/// task such as the shell that happens to be running a user program).
pub fn current_is_process() -> bool {
    let s = lock();
    s.tasks[s.current].is_process
}

/// First kernel entry for a forked child process. The scheduler has already
/// loaded the child's CR3 and set its RSP0; here we read the child's captured
/// user context and iretq into Ring 3.
fn process_enter() -> ! {
    let (ctx, cr3, rsp0) = {
        let mut s = lock();
        let cur = s.current;
        (
            s.tasks[cur].user_ctx.take().expect("process_enter: no context"),
            s.tasks[cur].cr3,
            s.tasks[cur].rsp0,
        )
    };
    if cr3 != 0 {
        crate::mem::switch_cr3(cr3);
    }
    if rsp0 != 0 {
        crate::interrupts::gdt::set_priv_stack(rsp0);
    }
    // Take a stable address of `ctx` and hand it to a non-inlined function so
    // the compiler cannot alias it to the rsp0 value during optimization.
    let ctx_ptr = &ctx as *const UserContext;
    unsafe { crate::user::enter_user_from_ctx(ctx_ptr) }
}

/// Fork the current process: deep-copy its user memory into a fresh,
/// independent page table, create a child task, and return the child's pid.
/// The child resumes with a copy of the parent's registers (rax = 0); the
/// parent's fork syscall returns this pid.
pub fn fork_process(parent_ctx: UserContext) -> Option<usize> {
    let mut s = lock();
    if s.task_count >= MAX_TASKS {
        return None;
    }
    // Deep-copy the parent's user pages into a new page table.
    let cr3 = match crate::mem::fork_user_space() {
        Ok(c) => c,
        Err(_) => return None,
    };
    let id = s.task_count;
    s.task_count += 1;
    let mut child_ctx = parent_ctx;
    child_ctx.rax = 0; // the child's fork() returns 0
    let rsp = prepare_stack(id, process_enter);
    s.tasks[id] = Task {
        saved_rsp: rsp,
        state: TaskState::Ready,
        wake_tick: None,
        entry: Some(process_enter),
        priority: 1,
        cr3,
        rsp0: stack_top(id),
        is_process: true,
        user_ctx: Some(child_ctx),
    };
    Some(id)
}

/// Snapshot for the `ps` command.
pub struct TaskInfo {
    pub pid: usize,
    pub state: &'static str,
}

pub fn snapshot() -> Vec<TaskInfo> {
    let s = lock();
    let mut out = Vec::new();
    for (i, t) in s.tasks.iter().enumerate().take(s.task_count) {
        let state = match t.state {
            TaskState::Ready => "ready",
            TaskState::Running => "running",
            TaskState::Blocked => "sleep",
            TaskState::Dead => "dead",
        };
        out.push(TaskInfo { pid: i, state });
    }
    out
}
