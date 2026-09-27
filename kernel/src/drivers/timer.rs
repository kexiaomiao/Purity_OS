//! PIT (8253/8254) timer: ~100 Hz tick source.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

/// Number of ticks since boot.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Timer frequency in Hz (100 -> 10ms per tick).
pub const FREQ_HZ: u64 = 100;

/// Initialize the PIT to [`FREQ_HZ`].
pub fn init() {
    let divisor = 1193180 / FREQ_HZ;
    unsafe {
        let mut cmd = Port::new(0x43);
        let mut data = Port::new(0x40);
        // Channel 0, lobyte/hibyte access, mode 3 (square wave).
        cmd.write(0x36u8);
        data.write((divisor & 0xff) as u8);
        data.write(((divisor >> 8) & 0xff) as u8);
    }
}

/// Called from the IRQ handler.
pub fn tick() {
    TICKS.fetch_add(1, Ordering::SeqCst);
}

/// Ticks since boot.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::SeqCst)
}

/// Seconds since boot (floating).
pub fn uptime_seconds() -> f64 {
    ticks() as f64 / FREQ_HZ as f64
}

/// Busy-wait (using `hlt`) for roughly `ms` milliseconds.
/// With the scheduler present this yields to other tasks via interrupts.
pub fn sleep_ms(ms: u64) {
    let target = ticks() + (ms * FREQ_HZ / 1000).max(1);
    while ticks() < target {
        x86_64::instructions::hlt();
    }
}
