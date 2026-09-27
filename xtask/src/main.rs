//! xtask: build the PurityOS kernel and pack it into a bootable disk image.
//!
//! Usage:
//!   cargo run --release -p xtask -- build
//!   cargo run --release -p xtask -- run      # build + launch QEMU

use std::path::PathBuf;
use std::process::Command;

fn main() -> anyhow::Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "build".into());

    let profile = "release";
    let target = "x86_64-unknown-none";
    println!("==> Building kernel ({})", profile);
    let status = Command::new("cargo")
        .args(["build", "-p", "purity_os", "--profile", profile, "--target", target])
        .current_dir("kernel")
        .status()?;
    anyhow::ensure!(status.success(), "kernel build failed");

    let kernel_path = PathBuf::from("target")
        .join(target)
        .join(profile)
        .join("purity_os");
    anyhow::ensure!(kernel_path.exists(), "kernel binary not found: {}", kernel_path.display());

    let image_path = PathBuf::from("target/purityos-bios.img");
    println!("==> Creating BIOS disk image at {}", image_path.display());

    // Request a framebuffer (1280x720, GUI desktop) via the disk-image boot
    // config; the kernel falls back to text mode if none is provided.
    let mut boot_config = bootloader::BootConfig::default();
    boot_config.frame_buffer.minimum_framebuffer_width = Some(1280);
    boot_config.frame_buffer.minimum_framebuffer_height = Some(720);
    boot_config.frame_buffer_logging = false;
    boot_config.serial_logging = true;

    let mut builder = bootloader::DiskImageBuilder::new(kernel_path);
    builder.set_boot_config(&boot_config);
    builder.create_bios_image(&image_path)?;

    println!("==> Done. Boot with:");
    println!(
        "    qemu-system-x86_64 -drive format=raw,file={} -serial stdio",
        image_path.display()
    );

    if cmd == "run" {
        println!("==> Launching QEMU");
        let status = Command::new("qemu-system-x86_64")
            .args([
                "-drive",
                &format!("format=raw,file={}", image_path.display()),
                "-serial",
                "stdio",
                "-m",
                "512M",
            ])
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
