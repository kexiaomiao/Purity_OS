//! VGA text-mode driver: 80x25 color text buffer at physical 0xb8000.

use spin::{Lazy, Mutex};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ColorCode(pub u8);

impl ColorCode {
    pub const fn new(fg: Color, bg: Color) -> ColorCode {
        ColorCode((bg as u8) << 4 | (fg as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii: u8,
    color: ColorCode,
}

/// A single cell in VGA memory. Reads/writes are volatile so the compiler
/// cannot elide or reorder them.
#[repr(transparent)]
struct Cell(ScreenChar);

impl Cell {
    #[inline]
    fn read(&self) -> ScreenChar {
        unsafe { core::ptr::read_volatile(&self.0) }
    }
    #[inline]
    fn write(&mut self, c: ScreenChar) {
        unsafe { core::ptr::write_volatile(&mut self.0, c) }
    }
}

pub const BUFFER_HEIGHT: usize = 25;
pub const BUFFER_WIDTH: usize = 80;

#[repr(transparent)]
struct Buffer {
    chars: [[Cell; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

pub struct Writer {
    col: usize,
    row: usize,
    color: ColorCode,
    buffer: &'static mut Buffer,
}

impl Writer {
    pub fn set_color(&mut self, fg: Color, bg: Color) {
        self.color = ColorCode::new(fg, bg);
    }

    pub fn color(&self) -> ColorCode {
        self.color
    }

    /// Clear the whole screen to the current background color.
    pub fn clear(&mut self) {
        let blank = ScreenChar { ascii: b' ', color: self.color };
        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                self.buffer.chars[row][col].write(blank);
            }
        }
        self.col = 0;
        self.row = 0;
    }

    fn new_line(&mut self) {
        if self.row < BUFFER_HEIGHT - 1 {
            self.row += 1;
        } else {
            self.scroll_up();
        }
        self.col = 0;
    }

    /// Move every row one line up, blank the bottom row.
    fn scroll_up(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let c = self.buffer.chars[row][col].read();
                self.buffer.chars[row - 1][col].write(c);
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar { ascii: b' ', color: self.color };
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            b'\r' => self.col = 0,
            0x08 => {
                if self.col > 0 {
                    self.col -= 1;
                    self.clear_current();
                }
            }
            b => {
                if self.col >= BUFFER_WIDTH {
                    self.new_line();
                }
                let row = self.row;
                let col = self.col;
                self.buffer.chars[row][col].write(ScreenChar { ascii: b, color: self.color });
                self.col += 1;
            }
        }
    }

    fn clear_current(&mut self) {
        let row = self.row;
        let col = self.col;
        self.buffer.chars[row][col].write(ScreenChar { ascii: b' ', color: self.color });
    }

    pub fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.write_byte(b);
        }
    }
}

impl core::fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        Writer::write_str(self, s);
        Ok(())
    }
}

static WRITER: Lazy<Mutex<Writer>> = Lazy::new(|| Mutex::new(Writer {
    col: 0,
    row: 0,
    color: ColorCode::new(Color::LightGray, Color::Black),
    buffer: unsafe { &mut *(0xb8000 as *mut Buffer) },
}));

/// Run a closure with the global VGA writer.
pub fn with_writer<F: FnOnce(&mut Writer)>(f: F) {
    f(&mut WRITER.lock());
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    let mut w = WRITER.lock();
    let _ = w.write_fmt(args);
}

/// Print to the current output: GUI terminal window (when the desktop is
/// active) or the VGA text screen.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::gui::output::_print(format_args!($($arg)*)) };
}

/// Print with a newline to the VGA screen.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}
