//! Build script: compile the Ring 3 "hello" user program into an ELF
//! (`user_hello.elf`) that gets embedded into the kernel via `include_bytes!`.
//!
//! The user program is a freestanding C file (no libc, no CRT) compiled with
//! the host `cc`, then linked with its own linker script into a bare ET_EXEC
//! ELF64 the kernel's ELF loader can map directly.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let c_src = src_dir.join("userspace/hello.c");
    let ld_script = src_dir.join("userspace/user.ld");
    let obj = out.join("user_hello.o");
    let elf = out.join("user_hello.elf");

    // 1) Compile the freestanding user program.
    let status = Command::new("cc")
        .arg("-c")
        .arg("-ffreestanding")
        .arg("-fno-stack-protector")
        .arg("-fno-pic")
        .arg("-mno-red-zone")
        .arg("-mcmodel=large")
        .arg("-O1")
        .arg("-o")
        .arg(&obj)
        .arg(&c_src)
        .status()
        .expect("failed to invoke cc");
    assert!(status.success(), "compiling userspace/hello.c failed");

    // 2) Link into a bare ET_EXEC ELF64.
    let status = Command::new("cc")
        .arg("-nostdlib")
        .arg("-no-pie")
        .arg("-T")
        .arg(&ld_script)
        .arg("-o")
        .arg(&elf)
        .arg(&obj)
        .status()
        .expect("failed to invoke linker");
    assert!(status.success(), "linking user_hello.elf failed");

    // Rebuild when the user program changes.
    println!("cargo:rerun-if-changed={}", c_src.display());
    println!("cargo:rerun-if-changed={}", ld_script.display());
}
