//! PC speaker (PIT channel 2 + port 0x61 gate).

use x86_64::instructions::port::Port;

/// Start playing the given frequency (Hz). Call [`stop`] to silence.
pub fn start(freq_hz: u32) {
    if freq_hz == 0 {
        return;
    }
    let divisor = 1_193_180 / freq_hz;
    unsafe {
        let mut pit_cmd = Port::new(0x43);
        let mut pit_ch2 = Port::new(0x42);
        let mut gate = Port::new(0x61);

        // Channel 2, lobyte/hibyte, mode 3 (square wave).
        pit_cmd.write(0xB6u8);
        pit_ch2.write((divisor & 0xff) as u8);
        pit_ch2.write(((divisor >> 8) & 0xff) as u8);

        // Gate bits 0 (timer 2 output to speaker) and 1 (speaker data).
        let current: u8 = gate.read();
        gate.write(current | 0x03);
    }
}

/// Stop the speaker.
pub fn stop() {
    unsafe {
        let mut gate = Port::new(0x61);
        let current: u8 = gate.read();
        gate.write(current & 0xFC);
    }
}

/// Play a single tone for `ms` milliseconds (blocking, tick-based).
pub fn beep(freq_hz: u32, ms: u64) {
    start(freq_hz);
    crate::drivers::timer::sleep_ms(ms);
    stop();
}

/// Play a little melody (Do Re Mi ...).
pub fn play() {
    // C4 D4 E4 F4 G4 A4 B4 C5
    let notes: [(u32, u64); 8] = [
        (262, 200), (294, 200), (330, 200), (349, 200),
        (392, 200), (440, 200), (494, 200), (523, 400),
    ];
    for (freq, dur) in notes {
        beep(freq, dur);
        crate::drivers::timer::sleep_ms(60);
    }
}

/// A rapid error beep used on kernel panic.
pub fn panic_alarm() {
    for _ in 0..3 {
        beep(880, 120);
        crate::drivers::timer::sleep_ms(80);
    }
}

/// Melodies for the Player app.
pub fn play_melody(idx: usize) {
    match idx {
        1 => {
            // Twinkle Twinkle (C C G G A A G ...)
            let notes: [(u32, u64); 14] = [
                (262, 300), (262, 300), (392, 300), (392, 300), (440, 300), (440, 300), (392, 600),
                (349, 300), (349, 300), (330, 300), (330, 300), (294, 300), (294, 300), (262, 600),
            ];
            for (f, d) in notes {
                beep(f, d);
            }
        }
        _ => play(),
    }
}
