//! Output routing: once the GUI is active, `print!`/`println!` go to the
//! focused terminal window; otherwise they fall back to the VGA text screen.
//!
//! Lock-order safety: the print path must NEVER nest locks. It reads the
//! framebuffer dimensions lock-free (atomics), then *tries* the WM lock. If
//! the WM lock is already held (e.g. we're inside the window manager, or an
//! exception fired mid-draw), we fall back to VGA instead of spinning — this
//! is the difference between a dropped frame and a deadlocked kernel.

use core::fmt;

/// Route formatted text to the GUI terminal (when active) or VGA.
pub fn _print(args: fmt::Arguments) {
    let (w, _) = crate::gui::fb::dims();
    if w > 0 {
        if let Some(mut wm) = crate::gui::wm::try_lock() {
            let s = alloc::format!("{}", args);
            wm.term_write(&s);
            return;
        }
        // Could not take the WM lock right now — fall through to VGA.
    }
    crate::drivers::vga::_print(args);
}
