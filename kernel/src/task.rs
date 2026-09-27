//! Kernel threads, context switching, and a round-robin scheduler.
//!
//! The PIT tick drives preemption; `spawn` creates background kernel tasks,
//! `sleep_ms` blocks the current task without spinning, and `ps` shows them.

use alloc::vec::Vec;
use spin::Mutex;

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
}

impl Task {
    const fn empty() -> Task {
        Task {
            saved_rsp: 0, state: TaskState::Ready, wake_tick: None, entry: None, priority: 1,
        }
    }
}

/// Per-task static kernel stacks.
static mut STACKS: [[u8; STACK_SIZE]; MAX_TASKS] = [[0; STACK_SIZE]; MAX_TASKS];

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
        };
        s.current = 0;
        s.task_count = 1;
    }
    spawn_inner(shell_entry, 1);

    // First context switch: boot task saves its context, shell runs.
    switch_to_next();

    // When we resume here, we are the idle task.
    loop {
        x86_64::instructions::hlt();
    }
}

fn idle() -> ! {
    // Load the on-disk PurityFS exactly once, from a fully-scheduled context.
    // Doing this before the scheduler was up produced double faults on the
    // first PIT IRQ after the ATA poll.
    use core::sync::atomic::{AtomicBool, Ordering};
    static FS_LOADED: AtomicBool = AtomicBool::new(false);
    if !FS_LOADED.swap(true, Ordering::SeqCst) {
        crate::fs::load_from_disk();
    }
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
    };
}

/// Pick the next runnable task and context-switch to it.
pub fn switch_to_next() {
    let (old_ptr, new_rsp) = {
        let mut s = lock();
        let old = s.current;
        let next = s.pick_next(old);
        if next == old {
            return;
        }
        s.tasks[old].state = TaskState::Ready;
        s.tasks[next].state = TaskState::Running;
        s.current = next;
        let old_ptr = &mut s.tasks[old].saved_rsp as *mut u64;
        let new_rsp = s.tasks[next].saved_rsp;
        (old_ptr, new_rsp)
    };
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
