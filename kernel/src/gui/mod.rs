//! PurityOS GUI subsystem: framebuffer, fonts, mouse, window manager and
//! the built-in desktop applications.

pub mod apps;
pub mod fb;
pub mod font;
pub mod output;
pub mod wm;

/// Initialize the GUI (framebuffer + mouse + window manager + desktop).
/// Returns true when a framebuffer was available.
pub fn start(fbuf: Option<bootloader_api::info::FrameBuffer>) -> bool {
    if !fb::init(fbuf) {
        return false;
    }
    crate::drivers::mouse::init();
    wm::init();
    true
}

/// One GUI tick (mouse handling + full redraw). No-op without framebuffer.
pub fn tick_if_active() {
    let (w, _) = fb::dims();
    if w > 0 {
        wm::tick();
    }
}
