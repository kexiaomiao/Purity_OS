//! ATA PIO disk driver (primary bus, ports 0x1F0-0x1F7).

use x86_64::instructions::port::Port;

const DATA: u16 = 0x1F0;
const ERROR: u16 = 0x1F1;
const SECTOR_COUNT: u16 = 0x1F2;
const LBA_LOW: u16 = 0x1F3;
const LBA_MID: u16 = 0x1F4;
const LBA_HIGH: u16 = 0x1F5;
const DRIVE_HEAD: u16 = 0x1F6;
const STATUS_CMD: u16 = 0x1F7;

const STATUS_BSY: u8 = 0x80;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

fn wait_bsy() -> Result<(), &'static str> {
    unsafe {
        let mut status = Port::<u8>::new(STATUS_CMD);
        for _ in 0..100_000 {
            if status.read() & STATUS_BSY == 0 {
                return Ok(());
            }
            x86_64::instructions::nop();
        }
    }
    Err("ATA timeout waiting for BSY")
}

fn wait_drq() -> Result<(), &'static str> {
    unsafe {
        let mut status = Port::<u8>::new(STATUS_CMD);
        for _ in 0..100_000 {
            let s = status.read();
            if s & STATUS_ERR != 0 {
                return Err("ATA error while waiting DRQ");
            }
            if s & STATUS_DRQ != 0 {
                return Ok(());
            }
            x86_64::instructions::nop();
        }
    }
    Err("ATA timeout waiting for DRQ")
}

/// Read one 512-byte LBA28 sector into `buf`.
pub fn read_sector(lba: u32, buf: &mut [u8; 512]) -> Result<(), &'static str> {
    if lba >= 0x0FFF_FFFF {
        return Err("LBA out of range");
    }
    // ATA PIO polling is a short, time-sensitive sequence. Mask interrupts for
    // its duration so the PIT IRQ cannot race the DRQ/BSY handshake (which was
    // producing double faults mid-poll).
    let was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    let result = unsafe {
        let mut drive_head = Port::new(DRIVE_HEAD);
        let mut sec_count = Port::new(SECTOR_COUNT);
        let mut lba_low = Port::new(LBA_LOW);
        let mut lba_mid = Port::new(LBA_MID);
        let mut lba_high = Port::new(LBA_HIGH);
        let mut cmd = Port::new(STATUS_CMD);
        let mut data = Port::new(DATA);

        wait_bsy()?;
        // Select master drive, LBA mode.
        drive_head.write(0xE0 | ((lba >> 24) & 0x0F) as u8);
        sec_count.write(1u8);
        lba_low.write((lba & 0xFF) as u8);
        lba_mid.write(((lba >> 8) & 0xFF) as u8);
        lba_high.write(((lba >> 16) & 0xFF) as u8);
        cmd.write(0x20u8); // READ SECTORS (PIO)

        // Poll status with a bounded loop.
        for _ in 0..100_000 {
            let status: u8 = cmd.read();
            if status & STATUS_ERR != 0 {
                return Err("ATA read error");
            }
            if status & STATUS_BSY == 0 {
                break;
            }
            x86_64::instructions::nop();
        }
        wait_drq()?;

        // Read 256 words.
        for chunk in buf.chunks_mut(2) {
            let word: u16 = data.read();
            chunk[0] = word as u8;
            chunk[1] = (word >> 8) as u8;
        }
        Ok(())
    };
    if was_enabled {
        unsafe { x86_64::instructions::interrupts::enable(); }
    }
    result
}

/// Write one 512-byte LBA28 sector from `buf`.
pub fn write_sector(lba: u32, buf: &[u8; 512]) -> Result<(), &'static str> {
    if lba >= 0x0FFF_FFFF {
        return Err("LBA out of range");
    }
    unsafe {
        let mut drive_head = Port::new(DRIVE_HEAD);
        let mut sec_count = Port::new(SECTOR_COUNT);
        let mut lba_low = Port::new(LBA_LOW);
        let mut lba_mid = Port::new(LBA_MID);
        let mut lba_high = Port::new(LBA_HIGH);
        let mut cmd = Port::new(STATUS_CMD);
        let mut data = Port::new(DATA);

        wait_bsy()?;
        drive_head.write(0xE0 | ((lba >> 24) & 0x0F) as u8);
        sec_count.write(1u8);
        lba_low.write((lba & 0xFF) as u8);
        lba_mid.write(((lba >> 8) & 0xFF) as u8);
        lba_high.write(((lba >> 16) & 0xFF) as u8);
        cmd.write(0x30u8); // WRITE SECTORS (PIO)

        wait_drq()?;
        for chunk in buf.chunks(2) {
            let word = chunk[0] as u16 | ((chunk[1] as u16) << 8);
            data.write(word);
        }
        wait_bsy()?;
        // Cache flush.
        cmd.write(0xE7u8);
        wait_bsy()?;
    }
    Ok(())
}
