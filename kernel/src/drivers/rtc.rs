//! RTC (CMOS) real-time clock driver, ports 0x70/0x71.

use x86_64::instructions::port::Port;

/// A wall-clock timestamp.
#[derive(Debug, Clone, Copy)]
pub struct DateTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        let mut addr = Port::new(0x70);
        let mut data = Port::new(0x71);
        addr.write(reg);
        data.read()
    }
}

fn cmos_write(reg: u8, value: u8) {
    unsafe {
        let mut addr = Port::new(0x70);
        let mut data = Port::new(0x71);
        addr.write(reg);
        data.write(value);
    }
}

/// Wait until an RTC update has finished.
fn wait_update() {
    while cmos_read(0x0A) & 0x80 != 0 {}
}

fn bcd_to_binary(b: u8) -> u8 {
    (b & 0x0F) + ((b >> 4) * 10)
}

/// Read the current wall-clock time.
pub fn now() -> DateTime {
    // Register B tells us whether values are BCD and whether 24h format is used.
    wait_update();
    let status_b = cmos_read(0x0B);
    let bcd = status_b & 0x04 == 0;
    let hour_12 = status_b & 0x02 == 0;

    let second = cmos_read(0x00);
    let minute = cmos_read(0x02);
    let hour = cmos_read(0x04);
    let day = cmos_read(0x07);
    let month = cmos_read(0x08);
    let year = cmos_read(0x09);

    let convert = |v: u8| -> u8 { if bcd { bcd_to_binary(v) } else { v } };

    // PM bit (bit 7 of raw hour register) must be checked BEFORE BCD conversion.
    let pm = hour_12 && (hour & 0x80 != 0);
    let hour_raw = hour & 0x7F;

    let second = convert(second) as u32;
    let minute = convert(minute) as u32;
    let mut hour = convert(hour_raw) as u32;
    let day = convert(day) as u32;
    let month = convert(month) as u32;
    let year = convert(year) as u32;

    if pm {
        hour += 12;
    }

    // Assume the 21st century (QEMU RTC defaults to 20xx).
    let year = 2000 + year;

    DateTime { year, month, day, hour, minute, second }
}

impl core::fmt::Display for DateTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Test the CMOS is readable (used by the `time` command path).
pub fn init() {
    // Reading the status register warms the driver; nothing else to do.
    let _ = cmos_read(0x0B);
}

/// (year, month, day) triple.
pub fn date_parts() -> (u32, u32, u32) {
    let d = now();
    (d.year, d.month, d.day)
}

/// Weekday of the 1st of the current month (0 = Sunday). Zeller's congruence.
pub fn first_weekday_of_month() -> u32 {
    let (y, m, _d) = date_parts();
    let (mut y, mut m) = (y as i64, m as i64);
    if m < 3 {
        m += 12;
        y -= 1;
    }
    let k = y % 100;
    let j = y / 100;
    let h = (1 + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // h: 0=Sat,1=Sun,...,6=Fri -> convert to 0=Sun.
    ((h + 6) % 7) as u32
}
