//! Serial (COM1) output — used as the kernel log / QEMU serial backend.

use spin::Mutex;
use uart_16550::SerialPort;

/// Global COM1 serial port, initialized in [`init`].
static SERIAL: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3F8) });

/// Initialize the serial port. Must be called once early.
pub fn init() {
    SERIAL.lock().init();
}

/// Write a single byte to COM1.
pub fn byte(b: u8) {
    SERIAL.lock().send(b);
}

/// Write a string to COM1.
pub fn write_str(s: &str) {
    let mut serial = SERIAL.lock();
    for b in s.bytes() {
        serial.send(b);
    }
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let mut serial = SERIAL.lock();
    let _ = serial.write_fmt(args);
}

/// Kernel logging macro (goes to serial only).
#[macro_export]
macro_rules! klog {
    ($($arg:tt)*) => {
        $crate::drivers::uart::_print(format_args!($($arg)*))
    }
}

// ---------------------------------------------------------------------------
// Lock-free, no-global early serial output.
//
// These functions talk to the 16550 UART at 0x3F8 directly with `outb`/`inb`,
// taking no spinlock and reading no global state. They are safe to call before
// the heap, paging, or the SERIAL static are up — which is exactly what we
// need to diagnose a crash before kernel_main fully runs.
// ---------------------------------------------------------------------------

/// Program COM1 to 115200 8N1. Idempotent; safe to call repeatedly.
pub unsafe fn early_init() {
    use x86_64::instructions::port::Port;
    Port::<u8>::new(0x3F9).write(0x00); // disable interrupts
    Port::<u8>::new(0x3FB).write(0x80); // DLAB on
    Port::<u8>::new(0x3F8).write(0x01); // divisor low  (115200)
    Port::<u8>::new(0x3F9).write(0x00); // divisor high
    Port::<u8>::new(0x3FB).write(0x03); // 8 bits, no parity, 1 stop
    Port::<u8>::new(0x3FC).write(0xC7); // enable FIFO, clear, 14-byte threshold
}

/// Spin until the UART TX FIFO has room, then send one byte.
pub unsafe fn early_byte(b: u8) {
    use x86_64::instructions::port::Port;
    // Transmitter Holding Register Empty (line status bit 5).
    while Port::<u8>::new(0x3FD).read() & 0x20 == 0 {}
    Port::<u8>::new(0x3F8).write(b);
}

/// Write a string directly to COM1, bypassing every lock/global.
pub unsafe fn early_print(s: &str) {
    for b in s.bytes() {
        // Translate LF to CRLF so terminals show newlines correctly.
        if b == b'\n' {
            early_byte(b'\r');
        }
        early_byte(b);
    }
}

/// A tiny `core::fmt::Write` wrapper over [`early_print`], for formatting
/// panic messages with no locks/heap.
struct EarlyWriter;
impl core::fmt::Write for EarlyWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        unsafe { early_print(s) };
        Ok(())
    }
}

/// Format and write a fmt::Arguments to COM1 directly.
pub unsafe fn early_print_fmt(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let _ = EarlyWriter.write_fmt(args);
}
