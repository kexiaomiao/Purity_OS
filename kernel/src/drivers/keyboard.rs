//! PS/2 keyboard driver: scancode set 1 -> ASCII, buffered for the shell.
//!
//! Special keys are encoded as high bytes so the shell can distinguish them
//! from printable ASCII:
//!   0x80 = Up, 0x81 = Down, 0x82 = Left, 0x83 = Right.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::instructions::port::Port;

pub const KEY_UP: u8 = 0x80;
pub const KEY_DOWN: u8 = 0x81;
pub const KEY_LEFT: u8 = 0x82;
pub const KEY_RIGHT: u8 = 0x83;

/// Buffered ASCII input; the shell reads from here.
static BUFFER: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());
/// Pre-injected bytes used for automated boot testing (no QEMU keyboard needed).
static PRELOAD: Mutex<VecDeque<u8>> = Mutex::new(VecDeque::new());
/// Tracks the E0 prefix (arrow keys / extended keys).
static EXTENDED: Mutex<bool> = Mutex::new(false);
/// Number of shift keys (left+right) currently held, so releasing one while
/// the other is still held keeps Shift active.
static SHIFT: AtomicUsize = AtomicUsize::new(0);

/// Push bytes into the preload queue; they are consumed before real hardware.
pub fn preload(bytes: &[u8]) {
    let mut p = PRELOAD.lock();
    for &b in bytes {
        p.push_back(b);
    }
}

/// Number of bytes waiting (preload queue first, then hardware buffer).
pub fn available() -> usize {
    let p = PRELOAD.lock().len();
    if p > 0 { p } else { BUFFER.lock().len() }
}

/// Pop one byte from the keyboard buffer, if any.
pub fn read() -> Option<u8> {
    let mut p = PRELOAD.lock();
    if let Some(b) = p.pop_front() {
        return Some(b);
    }
    BUFFER.lock().pop_front()
}

/// Drain all buffered bytes into `out`.
pub fn drain_into(out: &mut Vec<u8>) {
    let mut b = BUFFER.lock();
    while let Some(c) = b.pop_front() {
        out.push(c);
    }
}

/// Called from the keyboard IRQ handler.
pub fn handle_irq() {
    let scancode: u8 = unsafe {
        let mut port = Port::new(0x60);
        port.read()
    };

    if scancode == 0xE0 {
        *EXTENDED.lock() = true;
        return;
    }
    if scancode == 0xE1 {
        *EXTENDED.lock() = false;
        return;
    }

    let extended = core::mem::replace(&mut *EXTENDED.lock(), false);
    let release = scancode & 0x80 != 0;
    let code = scancode & 0x7F;

    // Track left/right shift on both make and break using a held count.
    if code == 0x2A || code == 0x36 {
        if release {
            SHIFT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| Some(c.saturating_sub(1))).ok();
        } else {
            SHIFT.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }

    if release {
        return;
    }

    // Extended (arrow) keys first.
    if extended {
        let special = match code {
            0x48 => Some(KEY_UP),
            0x50 => Some(KEY_DOWN),
            0x4B => Some(KEY_LEFT),
            0x4D => Some(KEY_RIGHT),
            _ => None,
        };
        if let Some(s) = special {
            BUFFER.lock().push_back(s);
        }
        return;
    }

    let shifted = SHIFT.load(Ordering::Relaxed) > 0;
    if let Some(ascii) = scancode_to_ascii(code, shifted) {
        // If a GUI window (e.g. Editor) has keyboard focus, route the byte
        // to it instead of the psh shell buffer.
        if !crate::gui::wm::route_key(ascii) {
            BUFFER.lock().push_back(ascii);
        }
    }

    // Wake any task blocked on SYS_READ (keyboard stdin).
    crate::task::wake_keyboard_waiters();
}

/// Translate a set-1 make code to ASCII, honoring the Shift modifier.
fn scancode_to_ascii(sc: u8, shift: bool) -> Option<u8> {
    let b = match sc {
        0x02..=0x0B => {
            // Number row: unshifted digits, shifted symbols.
            let unshifted = b"1234567890";
            let shifted = b"!@#$%^&*()";
            let i = (sc - 0x02) as usize;
            if shift { shifted[i] } else { unshifted[i] }
        }
        0x0C => if shift { b'_' } else { b'-' },
        0x0D => if shift { b'+' } else { b'=' },
        0x0E => 0x08, // backspace
        0x0F => b'\t', // tab
        0x10..=0x19 => {
            let lower = b"qwertyuiop"[(sc - 0x10) as usize];
            if shift { lower - 32 } else { lower }
        }
        0x1A => if shift { b'{' } else { b'[' },
        0x1B => if shift { b'}' } else { b']' },
        0x1C => b'\n', // enter
        0x1E..=0x26 => {
            let lower = b"asdfghjkl"[(sc - 0x1E) as usize];
            if shift { lower - 32 } else { lower }
        }
        0x27 => if shift { b':' } else { b';' },
        0x28 => if shift { b'"' } else { b'\'' },
        0x29 => if shift { b'~' } else { b'`' },
        0x2B => if shift { b'|' } else { b'\\' },
        0x2C..=0x32 => {
            let lower = b"zxcvbnm"[(sc - 0x2C) as usize];
            if shift { lower - 32 } else { lower }
        }
        0x33 => if shift { b'<' } else { b',' },
        0x34 => if shift { b'>' } else { b'.' },
        0x35 => if shift { b'?' } else { b'/' },
        0x39 => b' ',
        0x01 => 0x1b, // esc
        _ => return None,
    };
    Some(b)
}
