//! 8259 PIC remap and EOI handling.

use pic8259::ChainedPics;
use spin::Mutex;

pub const PIC_1_OFFSET: u8 = 0x20;
pub const PIC_2_OFFSET: u8 = 0x28;

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// IRQ indices after remap. PS/2 mouse sits on IRQ12 -> vector 0x2C.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,      // 0x20 (IRQ0)
    Keyboard = PIC_1_OFFSET + 1, // 0x21 (IRQ1)
    Mouse = PIC_2_OFFSET + 4,  // 0x2C (IRQ12)
}

impl InterruptIndex {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    pub fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

/// Initialize (remap) the PICs.
pub fn init() {
    unsafe { PICS.lock().initialize() };
}

/// Send end-of-interrupt to the PIC.
pub fn notify_eoi(index: InterruptIndex) {
    unsafe { PICS.lock().notify_end_of_interrupt(index.as_u8()) };
}
