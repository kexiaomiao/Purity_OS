//! Ring 3 userland: ELF loader, `iretq` transition, and `int 0x80` dispatch.
//!
//! The user program is a small no_std ELF baked into the kernel image by
//! `build.rs` (it lives at `OUT_DIR/user_hello.elf`).

use x86_64::VirtAddr;
use x86_64::structures::idt::InterruptStackFrame;

use crate::interrupts::gdt;
use crate::mem;

/// The embedded user program ELF.
static USER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/user_hello.elf"));

/// User stack size; its virtual address is chosen automatically by
/// `mem::alloc_user_region` (no hard-coded addresses).
const USER_STACK_SIZE: usize = 64 * 1024;

/// Kernel stack used to resume after the user program calls `exit`.
#[repr(align(16))]
#[allow(dead_code)] // storage only; accessed via `&raw const`
struct RetStack([u8; 16384]);
static mut RET_STACK: RetStack = RetStack([0; 16384]);

// ---- ELF helpers ----

fn rd_u16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn rd_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn rd_u64(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([
        d[o], d[o + 1], d[o + 2], d[o + 3], d[o + 4], d[o + 5], d[o + 6], d[o + 7],
    ])
}

/// Load the embedded ELF into user pages and jump to it (never returns).
pub fn run_user_program() -> Result<(), &'static str> {
    let elf: &[u8] = USER_ELF;
    if elf.len() < 64 {
        return Err("ELF file too small");
    }
    if &elf[0..4] != b"\x7fELF" {
        return Err("not an ELF file");
    }
    if elf[4] != 2 {
        return Err("not ELF64");
    }
    if rd_u16(elf, 16) != 2 {
        return Err("not an executable (ET_EXEC)");
    }
    if rd_u16(elf, 18) != 62 {
        return Err("not x86_64");
    }
    let e_entry = rd_u64(elf, 24);
    let e_phoff = rd_u64(elf, 32);
    let e_phentsize = rd_u16(elf, 54);
    let e_phnum = rd_u16(elf, 56);

    for i in 0..e_phnum {
        let off = (e_phoff + i as u64 * e_phentsize as u64) as usize;
        if off + 56 > elf.len() {
            return Err("program header out of bounds");
        }
        let p_type = rd_u32(elf, off);
        if p_type != 1 {
            continue; // only PT_LOAD segments are mapped
        }
        let p_flags = rd_u32(elf, off + 4);
        let p_offset = rd_u64(elf, off + 8);
        let p_vaddr = rd_u64(elf, off + 16);
        let p_filesz = rd_u64(elf, off + 32);
        let p_memsz = rd_u64(elf, off + 40);

        // Page-aligned window covering [p_vaddr, p_vaddr + p_memsz).
        let page_start = (p_vaddr & !0xFFF) as usize;
        let seg_end = p_vaddr.checked_add(p_memsz).ok_or("segment overflow")?;
        let page_end = ((seg_end + 0xFFF) & !0xFFF) as usize;
        let map_len = page_end - page_start;

        let writable = p_flags & 2 != 0; // PF_W
        mem::map_user_pages(page_start, map_len, writable)?;
        // Frames may hold stale data: clear the whole window first.
        mem::zero_pages(page_start, map_len);
        // Copy file-backed bytes at their intra-page offset.
        let dst = page_start + (p_vaddr as usize & 0xFFF);
        if (p_offset as usize) + (p_filesz as usize) <= elf.len() {
            let src = &elf[p_offset as usize..(p_offset + p_filesz) as usize];
            mem::copy_to(dst, src);
        }
    }

    // User stack: the virtual address is auto-allocated — never hard-coded.
    let stack_base = mem::alloc_user_region(USER_STACK_SIZE / 4096, true)
        .ok_or("no user stack available")?;
    mem::zero_pages(stack_base, USER_STACK_SIZE);

    crate::println!("[kernel] loading user program: entry={:#x}, stack={:#x}",
        e_entry, stack_base + USER_STACK_SIZE);
    crate::klog!("[kernel] jumping to Ring 3, entry {:#x}\n", e_entry);

    // irretq into user mode — never returns.
    unsafe { jump_user(e_entry, (stack_base + USER_STACK_SIZE) as u64) }
}

/// Construct an interrupt-return frame on the stack and execute `iretq`.
///
/// SS/RSP/RFLAGS/CS/RIP are pushed so the CPU lands in Ring 3 at `entry`
/// with the user stack.
unsafe fn jump_user(entry: u64, stack_top: u64) -> ! {
    let user_cs = (gdt::user_code().0 | 3) as u64;
    let user_ss = (gdt::user_data().0 | 3) as u64;
    core::arch::asm!(
        "mov rax, {ss}",
        "push rax",
        "mov rax, {rsp}",
        "push rax",
        "mov rax, {flags}",
        "push rax",
        "mov rax, {cs}",
        "push rax",
        "mov rax, {rip}",
        "push rax",
        "iretq",
        ss = in(reg) user_ss,
        rsp = in(reg) stack_top,
        flags = in(reg) 0x202u64, // IF=1
        cs = in(reg) user_cs,
        rip = in(reg) entry,
        options(noreturn),
    );
}

/// Rewrite the interrupt frame so the next `iretq` lands back in kernel mode
/// at `kernel_resume` (used by SYS_EXIT and by user-fault recovery).
pub(crate) fn recover_user_fault(stack: &mut InterruptStackFrame, what: &str) {
    // Serial-only: we run inside an exception handler where FB/WM/VGA locks
    // may already be held by the interrupted kernel code.
    crate::klog!("[user] fault '{}', recovering to shell\n", what);
    let ret_stack_top = {
        let base = VirtAddr::from_ptr(&raw const RET_STACK as *const _);
        (base + 16384).as_u64()
    };
    // copy-modify-write the frame through the volatile wrapper
    let mut fv = unsafe { stack.as_mut().read() };
    fv.instruction_pointer = VirtAddr::new(kernel_resume as *const () as u64);
    fv.code_segment = gdt::kernel_code();
    fv.stack_segment = gdt::kernel_data();
    fv.stack_pointer = VirtAddr::new(ret_stack_top);
    // cpu_flags left as-is (IF already set)
    unsafe { stack.as_mut().write(fv) };
}

/// Resume point after the user program exits; re-enters the shell loop.
extern "C" fn kernel_resume() -> ! {
    crate::println!("\n[user program exited — back in kernel mode]");
    crate::klog!("[user] program exited, returning to shell\n");
    crate::shell::main_loop();
}

// ---- System call dispatch ----

pub mod syscall {
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;
    use x86_64::structures::idt::InterruptStackFrame;

    use crate::interrupts::idt::syscall_numbers::{
        SYS_CLOSE, SYS_EXIT, SYS_GETPID, SYS_KILL, SYS_OPEN, SYS_READ, SYS_SLEEP, SYS_STAT,
        SYS_WRITE, SYS_WRITE_FD,
    };

    // User ELF is linked at 0x400000; user heap/stack live below 0x70000000.
    const USER_LO: u64 = 0x0040_0000;
    const USER_HI: u64 = 0x7000_0000;
    const MAX_IO: usize = 512;
    const MAX_PATH: usize = 256;

    fn in_user_range(a: u64, len: usize) -> bool {
        a >= USER_LO && a.saturating_add(len as u64) <= USER_HI
    }

    /// Copy bytes from a user pointer into the kernel.
    fn copy_in(a: u64, len: usize) -> Option<Vec<u8>> {
        let len = len.min(MAX_IO);
        if !in_user_range(a, len) {
            crate::klog!("[syscall] user ptr {:#x} out of range\n", a);
            return None;
        }
        let mut out = Vec::with_capacity(len);
        let ptr = a as *const u8;
        for i in 0..len {
            out.push(unsafe { *ptr.add(i) });
        }
        Some(out)
    }

    /// Copy kernel bytes out to a user buffer.
    fn copy_out(a: u64, data: &[u8]) -> bool {
        if !in_user_range(a, data.len()) {
            return false;
        }
        let ptr = a as *mut u8;
        for (i, b) in data.iter().enumerate() {
            unsafe { *ptr.add(i) = *b };
        }
        true
    }

    /// Read a NUL-terminated path string from user memory.
    fn read_user_path(a: u64) -> Option<String> {
        if !in_user_range(a, 1) {
            return None;
        }
        let ptr = a as *const u8;
        let mut bytes = Vec::new();
        for i in 0..MAX_PATH {
            let c = unsafe { *ptr.add(i) };
            if c == 0 {
                break;
            }
            bytes.push(c);
        }
        String::from_utf8(bytes).ok()
    }

    /// An open file description for a process.
    struct OpenFile {
        path: String,
        pos: u64,
        write: bool,
    }

    fn fd_table() -> spin::MutexGuard<'static, BTreeMap<u64, OpenFile>> {
        use spin::{Lazy, Mutex};
        static FDS: Lazy<Mutex<BTreeMap<u64, OpenFile>>> =
            Lazy::new(|| Mutex::new(BTreeMap::new()));
        FDS.lock()
    }

    /// Dispatch an `int 0x80` call. `a`/`b`/`c` are the user's rdi/rsi/rdx.
    pub fn dispatch(
        n: u64,
        a: u64,
        b: u64,
        c: u64,
        stack: &mut InterruptStackFrame,
    ) -> u64 {
        match n {
            SYS_WRITE => {
                // Print `b` bytes from user pointer `a` to the console.
                let len = b as usize;
                let Some(bytes) = copy_in(a, len) else { return 0 };
                let s = core::str::from_utf8(&bytes).unwrap_or("<non-utf8>");
                crate::print!("{}", s);
                bytes.len() as u64
            }
            SYS_READ => {
                // a = fd, b = user buffer, c = count. fd 0 = keyboard (stdin).
                let fd = a;
                let count = (c as usize).min(MAX_IO);
                let mut tmp = [0u8; MAX_IO];
                let got = if fd == 0 {
                    // Block (hlt) until at least one key is available.
                    while crate::drivers::keyboard::available() == 0 {
                        x86_64::instructions::hlt();
                    }
                    let mut n = 0;
                    while n < count {
                        match crate::drivers::keyboard::read() {
                            Some(k) => {
                                tmp[n] = k;
                                n += 1;
                            }
                            None => break,
                        }
                    }
                    n
                } else {
                    let mut table = fd_table();
                    let Some(f) = table.get_mut(&fd) else {
                        crate::klog!("[syscall] read: bad fd {}\n", fd);
                        return 0;
                    };
                    let data = crate::fs::with(|vfs| vfs.read_file(&f.path));
                    let Some(data) = data else { return 0 };
                    let start = f.pos as usize;
                    if start >= data.len() {
                        0
                    } else {
                        let end = (start + count).min(data.len());
                        let n = end - start;
                        tmp[..n].copy_from_slice(&data[start..end]);
                        f.pos += n as u64;
                        n
                    }
                };
                if copy_out(b, &tmp[..got]) {
                    got as u64
                } else {
                    0
                }
            }
            SYS_OPEN => {
                // a = path ptr, b = flags (0 = read, 1 = write). Returns fd.
                let Some(path) = read_user_path(a) else { return u64::MAX };
                let real = crate::fs::with(|vfs| vfs.follow(&vfs.resolve(&path)));
                let exists = crate::fs::with(|vfs| vfs.get(&real).is_some());
                if !exists {
                    crate::klog!("[syscall] open: {} not found\n", real);
                    return u64::MAX;
                }
                let mut table = fd_table();
                let mut id = 3u64;
                while table.contains_key(&id) {
                    id += 1;
                }
                table.insert(id, OpenFile { path: real, pos: 0, write: b != 0 });
                id
            }
            SYS_CLOSE => {
                fd_table().remove(&a);
                0
            }
            SYS_WRITE_FD => {
                // a = fd, b = user buffer, c = count. Overwrites the file.
                let Some(bytes) = copy_in(b, c as usize) else { return 0 };
                let table = fd_table();
                let Some(f) = table.get(&a) else { return 0 };
                let path = f.path.clone();
                drop(table);
                match crate::fs::with(|vfs| vfs.write_file_full(&path, &bytes)) {
                    Ok(n) => n,
                    Err(_) => 0,
                }
            }
            SYS_STAT => {
                // a = path ptr, b = out buffer (16 bytes: size u64, kind u32, mode u32).
                let Some(path) = read_user_path(a) else { return 0 };
                let info = crate::fs::with(|vfs| {
                    let real = vfs.follow(&vfs.resolve(&path));
                    vfs.get(&real).map(|n| (n.size, n.kind as u32, n.mode as u32))
                });
                let Some((size, kind, mode)) = info else { return 0 };
                let mut out = [0u8; 16];
                out[0..8].copy_from_slice(&size.to_le_bytes());
                out[8..12].copy_from_slice(&kind.to_le_bytes());
                out[12..16].copy_from_slice(&mode.to_le_bytes());
                if copy_out(b, &out) { 1 } else { 0 }
            }
            SYS_GETPID => 1,
            SYS_KILL => {
                // a = pid to kill. Killing pid 0 (idle) is refused.
                if a == 0 {
                    0
                } else if crate::task::kill(a as usize) {
                    1
                } else {
                    0
                }
            }
            SYS_EXIT => {
                super::recover_user_fault(stack, "exit");
                0
            }
            SYS_SLEEP => {
                // Block this task and yield the CPU; other tasks keep running.
                crate::task::sleep_ms(a);
                0
            }
            _ => {
                crate::klog!("[syscall] unknown syscall number {}\n", n);
                0
            }
        }
    }
}
