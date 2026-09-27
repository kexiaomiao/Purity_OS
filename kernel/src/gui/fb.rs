//! Framebuffer driver and drawing primitives for the GUI.
//!
//! The bootloader hands us a linear framebuffer (see `BootloaderConfig` in
//! `main.rs`). All pixel work goes through this module; the VGA text mode is
//! only a fallback for the serial-less early boot.

use bootloader_api::info::{FrameBuffer, PixelFormat};
use spin::Mutex;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::gui::font;

/// Truecolor packed as 0x00RRGGBB.
pub type Color = u32;

pub const BLACK: Color = 0x000000;
pub const WHITE: Color = 0xffffff;
pub const LIGHT_GRAY: Color = 0xd4d4d4;
pub const GRAY: Color = 0x808080;
pub const DARK: Color = 0x1e1e1e;
pub const DARKER: Color = 0x141414;
pub const ACCENT: Color = 0x2d6cdf; // Windows-ish blue
pub const ACCENT_LIGHT: Color = 0x5b8ff5;
pub const GREEN: Color = 0x2ecc71;
pub const RED: Color = 0xe74c3c;
pub const ORANGE: Color = 0xf39c12;
pub const CYAN: Color = 0x17a2b8;
pub const PURPLE: Color = 0x9b59b6;
pub const PINK: Color = 0xe91e63;
pub const YELLOW: Color = 0xf1c40f;

pub struct Fb {
    buf: &'static mut [u8],
    pub width: usize,
    pub height: usize,
    bpp: usize,
    format: PixelFormat,
}

static FB: Mutex<Option<Fb>> = Mutex::new(None);

/// Screen dimensions as plain atomics — readable without taking the FB lock,
/// so the print path can check "is the GUI up?" without risking a lock-order
/// inversion with WM.
static FB_W: AtomicUsize = AtomicUsize::new(0);
static FB_H: AtomicUsize = AtomicUsize::new(0);

/// Take the framebuffer out of the boot info. Returns `false` if the
/// bootloader did not set one up.
pub fn init(fbuf: Option<FrameBuffer>) -> bool {
    match fbuf {
        Some(f) => {
            let info = f.info();
            let width = info.width;
            let height = info.height;
            let bpp = info.bytes_per_pixel;
            let format = info.pixel_format;
            let buf = f.into_buffer();
            FB_W.store(width, Ordering::Relaxed);
            FB_H.store(height, Ordering::Relaxed);
            *FB.lock() = Some(Fb { buf, width, height, bpp, format });
            true
        }
        None => false,
    }
}

/// Run `f` with the framebuffer. If no framebuffer is available, does nothing.
pub fn with<F: FnOnce(&mut Fb)>(f: F) {
    if let Some(fb) = FB.lock().as_mut() {
        f(fb);
    }
}

impl Fb {
    fn offset(&self, x: usize, y: usize) -> usize {
        (y * self.width + x) * self.bpp
    }

    fn put_pixel_raw(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let o = self.offset(x, y);
        match self.format {
            PixelFormat::Bgr => {
                self.buf[o] = b;
                self.buf[o + 1] = g;
                self.buf[o + 2] = r;
            }
            _ => {
                self.buf[o] = r;
                self.buf[o + 1] = g;
                self.buf[o + 2] = b;
            }
        }
    }

    pub fn pixel(&mut self, x: usize, y: usize, c: Color) {
        let r = ((c >> 16) & 0xff) as u8;
        let g = ((c >> 8) & 0xff) as u8;
        let b = (c & 0xff) as u8;
        self.put_pixel_raw(x, y, r, g, b);
    }

    /// Clear the whole screen.
    pub fn clear(&mut self, c: Color) {
        let r = ((c >> 16) & 0xff) as u8;
        let g = ((c >> 8) & 0xff) as u8;
        let b = (c & 0xff) as u8;
        match self.format {
            PixelFormat::Bgr => {
                for px in self.buf.chunks_mut(self.bpp) {
                    if px.len() >= 3 {
                        px[0] = b;
                        px[1] = g;
                        px[2] = r;
                    }
                }
            }
            _ => {
                for px in self.buf.chunks_mut(self.bpp) {
                    if px.len() >= 3 {
                        px[0] = r;
                        px[1] = g;
                        px[2] = b;
                    }
                }
            }
        }
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: Color) {
        let x2 = (x + w).min(self.width);
        let y2 = (y + h).min(self.height);
        for yy in y..y2 {
            for xx in x..x2 {
                self.pixel(xx, yy, c);
            }
        }
    }

    pub fn hline(&mut self, x: usize, y: usize, w: usize, c: Color) {
        self.fill_rect(x, y, w, 1, c);
    }

    pub fn vline(&mut self, x: usize, y: usize, h: usize, c: Color) {
        self.fill_rect(x, y, 1, h, c);
    }

    /// Rectangle outline.
    pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: Color) {
        self.hline(x, y, w, c);
        self.hline(x, y + h - 1, w, c);
        self.vline(x, y, h, c);
        self.vline(x + w - 1, y, h, c);
    }

    /// Rounded rectangle with filled interior.
    pub fn fill_round_rect(&mut self, x: usize, y: usize, w: usize, h: usize, r: usize, c: Color) {
        for yy in y..y + h {
            for xx in x..x + w {
                // Corner check: skip pixels outside the rounded corners.
                let dx = if xx < x + r { x + r - 1 - xx } else if xx >= x + w - r { xx - (x + w - r) } else { 0 };
                let dy = if yy < y + r { y + r - 1 - yy } else if yy >= y + h - r { yy - (y + h - r) } else { 0 };
                if dx * dx + dy * dy <= (r * r) as usize {
                    self.pixel(xx, yy, c);
                }
            }
        }
    }

    /// Draw a single glyph at pixel (x, y) with foreground `fg` and
    /// background `bg` (bg may be None for transparency).
    pub fn glyph(&mut self, x: usize, y: usize, ch: u8, fg: Color, bg: Option<Color>) {
        if ch < 0x20 || ch > 0x7e {
            return;
        }
        let g = &font::GLYPHS[(ch - 0x20) as usize];
        for (row, bits) in g.iter().enumerate() {
            for col in 0..8 {
                let on = bits & (1 << (7 - col)) != 0;
                if on {
                    self.pixel(x + col, y + row, fg);
                } else if let Some(b) = bg {
                    self.pixel(x + col, y + row, b);
                }
            }
        }
    }

    /// Draw a string at pixel (x, y).
    pub fn text(&mut self, x: usize, y: usize, s: &str, fg: Color, bg: Option<Color>) {
        let mut cx = x;
        for b in s.bytes() {
            if b == b'\n' {
                continue; // caller handles line breaks
            }
            if b >= 0x20 && b <= 0x7e {
                self.glyph(cx, y, b, fg, bg);
                cx += font::CHAR_W;
            } else {
                cx += font::CHAR_W; // skip unknown
            }
        }
    }

    /// Draw a horizontal gradient rectangle (top color -> bottom color).
    pub fn vgradient(&mut self, x: usize, y: usize, w: usize, h: usize, top: Color, bot: Color) {
        let tr = (top >> 16) & 0xff; let tg = (top >> 8) & 0xff; let tb = top & 0xff;
        let br = (bot >> 16) & 0xff; let bg = (bot >> 8) & 0xff; let bb = bot & 0xff;
        for i in 0..h {
            let t = i as i64 * 100 / h.max(1) as i64;
            let mix = |a: u32, b: u32| -> u32 {
                (a as i64 + (b as i64 - a as i64) * t / 100) as u32
            };
            let r = mix(tr, br) as u8;
            let g = mix(tg, bg) as u8;
            let b = mix(tb, bb) as u8;
            let c = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
            self.hline(x, y + i, w, c);
        }
    }
}

/// Screen dimensions (0,0 if no framebuffer). Lock-free: the values are set
/// once at init and never change afterwards, so no ordering with FB/WM locks
/// can ever deadlock the print path.
pub fn dims() -> (usize, usize) {
    (FB_W.load(Ordering::Relaxed), FB_H.load(Ordering::Relaxed))
}
