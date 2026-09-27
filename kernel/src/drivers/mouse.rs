//! PS/2 mouse driver (port 0x60/0x64 + IRQ12).
//!
//! Standard init sequence: enable the aux channel, set default settings,
//! enable data reporting. Packets are 3 bytes: [flags, dx, dy].

use spin::Mutex;
use x86_64::instructions::port::Port;

use crate::interrupts::pic::InterruptIndex;

/// Mouse state: absolute cursor position and button states.
pub struct Mouse {
    pub x: usize,
    pub y: usize,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    dx: i16,
    dy: i16,
    last_left_press_tick: u64,
    last_left_press_x: usize,
    last_left_press_y: usize,
    pub double_click: bool,
}

static MOUSE: Mutex<Mouse> = Mutex::new(Mouse {
    x: 320,
    y: 240,
    left: false,
    right: false,
    middle: false,
    dx: 0,
    dy: 0,
    last_left_press_tick: 0,
    last_left_press_x: 0,
    last_left_press_y: 0,
    double_click: false,
});

fn wait_output() {
    let mut status: x86_64::instructions::port::Port<u8> = Port::new(0x64);
    while unsafe { status.read() } & 1u8 == 0 {
        x86_64::instructions::hlt();
    }
}

fn wait_input() {
    let mut status: x86_64::instructions::port::Port<u8> = Port::new(0x64);
    while unsafe { status.read() } & 2u8 != 0 {
        x86_64::instructions::hlt();
    }
}

fn write_aux(data: u8) {
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0xD4u8) }; // send to aux device
    wait_input();
    let mut data_port = Port::new(0x60);
    unsafe { data_port.write(data) };
    // wait for ACK
    wait_output();
    let mut r = Port::new(0x60);
    let _ack: u8 = unsafe { r.read() };
}

/// Initialize the PS/2 mouse.
pub fn init() {
    // Disable devices first.
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0xADu8) }; // disable keyboard
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0xA7u8) }; // disable mouse

    // Read command byte and enable IRQ12 + aux.
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0x20u8) };
    wait_output();
    let mut r = Port::new(0x60);
    let mut config: u8 = unsafe { r.read() };
    config |= 0x02; // enable aux IRQ (bit 1)
    config &= !0x20; // clear bit 5 (aux clock disable)
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0x60u8) };
    wait_input();
    let mut r = Port::new(0x60);
    unsafe { r.write(config) };

    // Enable aux device.
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0xA8u8) };

    // Enable keyboard again.
    wait_input();
    let mut cmd = Port::new(0x64);
    unsafe { cmd.write(0xAEu8) };

    // Mouse: default settings, then enable reporting.
    write_aux(0xF6);
    write_aux(0xF4);
}

/// Read the 3-byte packet (called from the IRQ12 handler).
pub fn handle_irq() {
    let mut r = Port::new(0x60);
    let b0: u8 = unsafe { r.read() };
    let dx: u8 = unsafe { r.read() };
    let dy: u8 = unsafe { r.read() };

    let mut m = MOUSE.lock();
    let mut dxv = dx as i16;
    if b0 & 0x10 != 0 {
        dxv |= !0xff; // sign-extend 9-bit
    }
    let mut dyv = dy as i16;
    if b0 & 0x20 != 0 {
        dyv |= !0xff;
    }
    let now_left = b0 & 0x01 != 0;
    // Detect a left-button down-edge: within 40 ticks (~400 ms) of the
    // previous press, within 10 px, this is a double-click. Use the live
    // position (base + accumulated delta) rather than the lagging m.x/m.y
    // which is only refreshed by the idle task.
    let cur_x = (m.x as i64 + m.dx as i64).clamp(0, 4096) as usize;
    let cur_y = (m.y as i64 + m.dy as i64).clamp(0, 4096) as usize;
    if now_left && !m.left {
        let tick = crate::drivers::timer::ticks();
        let near = (cur_x as i64 - m.last_left_press_x as i64).abs() < 10
            && (cur_y as i64 - m.last_left_press_y as i64).abs() < 10;
        m.double_click = tick - m.last_left_press_tick < 40 && near;
        m.last_left_press_tick = tick;
        m.last_left_press_x = cur_x;
        m.last_left_press_y = cur_y;
    } else if !now_left {
        m.double_click = false;
    }
    m.left = now_left;
    m.right = b0 & 0x02 != 0;
    m.middle = b0 & 0x04 != 0;
    m.dx += dxv;
    m.dy -= dyv; // y is flipped in screen space
}

/// Whether the most recent left-button edge was a double-click.
/// Consumes the flag (clears it on read).
pub fn consume_double_click() -> bool {
    let mut m = MOUSE.lock();
    let d = m.double_click;
    m.double_click = false;
    d
}

/// Apply accumulated deltas to the cursor and return the new position.
pub fn update_position(screen_w: usize, screen_h: usize) {
    let mut m = MOUSE.lock();
    m.x = (m.x as i64 + m.dx as i64).clamp(0, screen_w as i64 - 1) as usize;
    m.y = (m.y as i64 + m.dy as i64).clamp(0, screen_h as i64 - 1) as usize;
    m.dx = 0;
    m.dy = 0;
}

/// Current cursor state (position + buttons).
pub fn state() -> (usize, usize, bool, bool, bool) {
    let m = MOUSE.lock();
    (m.x, m.y, m.left, m.right, m.middle)
}

/// Whether the IRQ12 line should be remapped (used by pic init).
pub fn irq_index() -> InterruptIndex {
    InterruptIndex::Mouse
}
