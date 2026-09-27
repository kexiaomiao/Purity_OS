//! PCI configuration space enumeration via ports 0xCF8 (address) / 0xCFC (data).

use x86_64::instructions::port::Port;

/// A discovered PCI device.
#[derive(Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

fn read_config(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset & 0xFC) as u32);
    unsafe {
        let mut addr_port = Port::new(0xCF8);
        let mut data_port = Port::new(0xCFC);
        addr_port.write(address);
        data_port.read()
    }
}

/// Human-readable name for a PCI class code.
pub fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x00, _) => "Unclassified",
        (0x01, 0x01) => "IDE controller",
        (0x01, 0x06) => "SATA controller",
        (0x03, 0x00) => "VGA controller",
        (0x06, 0x00) => "Host bridge",
        (0x06, 0x01) => "ISA bridge",
        (0x06, 0x04) => "PCI bridge",
        (0x0C, 0x05) => "USB controller",
        (0x04, _) => "Multimedia device",
        (0x02, _) => "Network controller",
        (0x08, _) => "System peripheral",
        _ => "Other device",
    }
}

/// Enumerate up to `max` PCI devices. Calls `f` for each found device.
pub fn enumerate(mut f: impl FnMut(PciDevice)) {
    for bus in 0..=255u16 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let vendor_id_raw = read_config(bus as u8, device, function, 0x00);
                let vendor_id = (vendor_id_raw & 0xFFFF) as u16;
                if vendor_id == 0xFFFF {
                    if function == 0 {
                        break; // no device at function 0 -> skip this device
                    } else {
                        continue;
                    }
                }

                let device_id = (vendor_id_raw >> 16) as u16;
                let class_raw = read_config(bus as u8, device, function, 0x08);
                let class = (class_raw >> 24) as u8;
                let subclass = ((class_raw >> 16) & 0xFF) as u8;
                let prog_if = ((class_raw >> 8) & 0xFF) as u8;

                f(PciDevice {
                    bus: bus as u8,
                    device,
                    function,
                    vendor: vendor_id,
                    device_id,
                    class,
                    subclass,
                    prog_if,
                });

                // If function 0 isn't a multi-function device, stop here.
                let header_type = (read_config(bus as u8, device, 0, 0x0C) >> 16) & 0x80;
                if function == 0 && header_type == 0 {
                    break;
                }
            }
        }
    }
}
