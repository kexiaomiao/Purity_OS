//! 8259 PIC remap and EOI handling.

use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::instructions::port::Port;

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
    // Mask every IRQ except the ones we actually service. The ATA disk
    // (IRQ14) asserts its line even in PIO polling mode; without a handler and
    // EOI it corrupts interrupt delivery the moment interrupts are enabled,
    // producing a double fault. Blocking it (and all other unserviced IRQs)
    // keeps the interrupt controller quiet until we install real handlers.
    unsafe {
        // PIC1 data (0x21): enable IRQ0 (timer), IRQ1 (keyboard), IRQ2
        // (cascade). Mask IRQ3..IRQ7 (serial etc.).
        Port::new(0x21u16).write(0xF8u8);
        // PIC2 data (0xA1): enable IRQ12 (mouse). Mask IRQ8..11, IRQ13..15,
        // including IRQ14 (ATA) and IRQ15 (secondary ATA).
        Port::new(0xA1u16).write(0xEFu8);
    }
}

/// Send end-of-interrupt to the PIC.
pub fn notify_eoi(index: InterruptIndex) {
    unsafe { PICS.lock().notify_end_of_interrupt(index.as_u8()) };
}
