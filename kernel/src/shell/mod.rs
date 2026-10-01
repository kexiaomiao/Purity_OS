//! PurityOS interactive command-line shell (`psh`).

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;
use x86_64::instructions::port::Port;

use crate::drivers::keyboard;
use crate::drivers::rtc;
use crate::drivers::speaker;
use crate::drivers::timer;
use crate::drivers::vga::Color;
use crate::fs;
use crate::mem;
use crate::print;
use crate::println;
use crate::task;

static THEME: Mutex<Theme> = Mutex::new(Theme { fg: Color::LightGray, bg: Color::Black });

#[derive(Clone, Copy)]
struct Theme {
    fg: Color,
    bg: Color,
}

fn apply_theme() {
    let t = THEME.lock();
    crate::drivers::vga::with_writer(|w| w.set_color(t.fg, t.bg));
}

/// All known commands, used for tab-completion.
const COMMANDS: &[&str] = &[
    "help", "echo", "clear", "reboot", "shutdown", "version", "about", "uname",
    "uptime", "mem", "free", "top", "ps", "ls", "dir", "pwd", "cd", "mkdir",
    "touch", "cat", "rm", "write", "color", "calc", "hexdump", "rand", "banner",
    "date", "time", "beep", "play", "lspci", "disk", "spawn", "sleep", "run",
    "ln", "dev", "fsinfo", "panic",
];

/// Command history ring buffer.
struct History {
    entries: Vec<String>,
    /// Position while browsing; `len()` means "current being typed".
    pos: usize,
}

impl History {
    fn push(&mut self, cmd: &str) {
        self.entries.push(String::from(cmd));
        if self.entries.len() > 32 {
            self.entries.remove(0);
        }
        self.pos = self.entries.len();
    }
}

static HISTORY: Mutex<History> = Mutex::new(History { entries: Vec::new(), pos: 0 });

fn prompt_text() -> String {
    let cwd = fs::with(|f| f.cwd.clone());
    let mut s = String::from("purityos:");
    s.push_str(&cwd);
    s.push_str("$ ");
    s
}

/// Entry point: run the shell forever (runs as a kernel task).
pub fn run() -> ! {
    apply_theme();
    print_banner();
    println!();
    println!("Welcome to PurityOS. Type `help` for a list of commands.");
    println!("Tab completes commands; Up/Down recall history.\n");

    // Automated boot test: drop into the user-mode Ring 3 shell, which reads
    // the preloaded keyboard bytes ("echo test", "hi").
    crate::klog!("[test] launching user shell\n");
    match crate::user::run_user_shell() {
        Ok(()) => crate::klog!("[test] user shell returned\n"),
        Err(e) => crate::klog!("[test] user shell failed: {}\n", e),
    }

    main_loop()
}

/// The shell command loop. Re-entered after a user program exits.
pub fn main_loop() -> ! {
    loop {
        print!("{}", prompt_text());
        let line = read_line();
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            HISTORY.lock().push(trimmed);
        }
        if trimmed.is_empty() {
            continue;
        }
        execute(trimmed);
    }
}

/// Read a line from the keyboard with editing, history and tab completion.
fn read_line() -> String {
    let mut line = String::new();
    let mut pending: Vec<u8> = Vec::new();

    loop {
        keyboard::drain_into(&mut pending);

        for &b in pending.iter() {
            match b {
                b'\n' => {
                    println!();
                    return line;
                }
                0x08 => {
                    if !line.is_empty() {
                        line.pop();
                        crate::print!("\x08 \x08");
                    }
                }
                b'\t' => {
                    tab_complete(&mut line);
                }
                keyboard::KEY_UP => recall_history(&mut line, true),
                keyboard::KEY_DOWN => recall_history(&mut line, false),
                c if c >= 0x20 && c < 0x7f => {
                    line.push(c as char);
                    crate::print!("{}", c as char);
                }
                _ => {}
            }
        }
        pending.clear();
        x86_64::instructions::hlt();
    }
}

fn repaint(line: &str) {
    let prompt = prompt_text();
    // Carriage return, rewrite prompt + line, pad to clear leftovers.
    crate::print!("\r{}{}", prompt, line);
    let used = prompt.len() + line.len();
    for _ in used..80 {
        crate::print!(" ");
    }
    // Move cursor back to end of line.
    for _ in used..80 {
        crate::print!("\x08");
    }
}

fn recall_history(line: &mut String, up: bool) {
    let mut h = HISTORY.lock();
    if h.entries.is_empty() {
        return;
    }
    if up {
        if h.pos > 0 {
            h.pos -= 1;
        }
    } else {
        if h.pos < h.entries.len() {
            h.pos += 1;
        }
    }
    let new = if h.pos < h.entries.len() {
        h.entries[h.pos].clone()
    } else {
        String::new()
    };
    *line = new;
    repaint(line);
}

fn tab_complete(line: &mut String) {
    let prefix: String = line.chars().take_while(|c| !c.is_whitespace()).collect();
    if prefix.is_empty() {
        return;
    }
    let matches: Vec<&&str> = COMMANDS.iter().filter(|c| c.starts_with(&prefix)).collect();
    if matches.is_empty() {
        return;
    }
    if matches.len() == 1 {
        line.clear();
        line.push_str(matches[0]);
        crate::print!("{} ", matches[0].trim_start_matches(""));
    } else {
        // Show candidates on a new line.
        println!();
        for m in &matches {
            print!("{}  ", m);
        }
        println!();
        repaint(line);
    }
}

fn print_banner() {
    crate::drivers::vga::with_writer(|w| w.set_color(Color::LightCyan, Color::Black));
    println!(" ____  _                   _    ___  ____");
    println!("|  _ \\(_)_ __  _   ___  __| |_ / _ \\/ ___|");
    println!("| |_) | | '_ \\| | | \\ \\/ / __| | | | \\___ \\");
    println!("|  __/| | | | | |_| |>  <| |_| |_| |___) |");
    println!("|_|   |_|_| |_|\\__,_/_/\\_\\\\__|\\___/|____/");
    println!("Pure Rust · from-scratch kernel · v0.4.0");
    apply_theme();
}

fn split_args(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn execute(line: &str) {
    let args = split_args(line);
    let cmd = args[0];
    match cmd {
        "help" => cmd_help(),
        "echo" => println!("{}", args.get(1..).map(|a| a.join(" ")).unwrap_or_default()),
        "clear" => crate::drivers::vga::with_writer(|w| w.clear()),
        "reboot" => {
            crate::fs::flush_to_disk();
            unsafe { Port::new(0x64).write(0xFEu8) };
        }
        "shutdown" => {
            crate::fs::flush_to_disk();
            unsafe { Port::new(0x604).write(0x2000u16) };
        }
        "version" | "about" | "uname" => cmd_version(),
        "uptime" => {
            let s = timer::uptime_seconds();
            println!("up {:.1} seconds ({} ticks @ {} Hz)", s, timer::ticks(), timer::FREQ_HZ);
        }
        "date" | "time" => println!("{}", rtc::now()),
        "beep" => {
            let freq: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(880);
            let ms: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
            speaker::beep(freq, ms);
        }
        "play" => speaker::play(),
        "lspci" => cmd_lspci(),
        "disk" => cmd_disk(&args[1..]),
        "spawn" => cmd_spawn(args.get(1)),
        "run" => cmd_run(),
        "sleep" => {
            let ms: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
            println!("sleeping {} ms...", ms);
            task::sleep_ms(ms);
            println!("awake.");
        }
        "mem" | "free" => cmd_free(),
        "top" => cmd_top(),
        "ps" => cmd_ps(),
        "ls" | "dir" => cmd_ls(args.get(1)),
        "pwd" => fs::with(|f| println!("{}", f.cwd)),
        "cd" => cmd_cd(args.get(1)),
        "mkdir" => cmd_mkdir(args.get(1)),
        "touch" => cmd_touch(args.get(1)),
        "cat" => cmd_cat(args.get(1)),
        "rm" => cmd_rm(args.get(1)),
        "ln" => cmd_ln(args.get(1), args.get(2)),
        "dev" => cmd_dev(args.get(1)),
        "fsinfo" => cmd_fsinfo(),
        "write" => cmd_write(&args),
        "color" => cmd_color(args.get(1)),
        "calc" => cmd_calc(&args[1..]),
        "hexdump" => cmd_hexdump(args.get(1)),
        "rand" => cmd_rand(),
        "banner" => print_banner(),
        "panic" => panic!("PurityOS: intentional panic requested by user"),
        _ => println!("purityos: command not found: {} (try `help`)", cmd),
    }
}

fn cmd_help() {
    println!("PurityOS commands:");
    println!("  Files:   ls pwd cd mkdir touch cat rm write hexdump ln dev fsinfo");
    println!("  Info:    help echo clear version uptime date time mem free top ps");
    println!("  Sound:   beep <hz> <ms>  play");
    println!("  Sys:     lspci disk read <lba>  spawn <name>  sleep <ms>");
    println!("  Ring 3:  run   (load & execute the user program)");
    println!("  Utils:   color <name>  calc <expr>  rand  banner");
    println!("  Power:   reboot  shutdown  panic");
}

fn cmd_version() {
    println!("PurityOS 0.5.0 (pure Rust, from-scratch kernel)");
    println!("  arch    : x86_64");
    println!("  kernel  : purity-kernel 0.5.0");
    println!("  shell   : psh 0.5.0");
    println!("  uptime  : {:.1} s", timer::uptime_seconds());
    println!("  rtc     : {}", rtc::now());
}

fn cmd_free() {
    let (used, total) = mem::heap_stats();
    println!("Heap: {} KiB used / {} KiB total ({} KiB free)",
        used / 1024, total / 1024, (total - used) / 1024);
}

fn cmd_top() {
    let (used, total) = mem::heap_stats();
    println!("--- PurityOS top ---");
    println!("uptime: {:.1}s   ticks: {}", timer::uptime_seconds(), timer::ticks());
    println!("heap:  {} KiB used / {} KiB total", used / 1024, total / 1024);
    println!("tasks:");
    for info in task::snapshot() {
        println!("  pid {}  {}", info.pid, info.state);
    }
}

fn cmd_ps() {
    println!("PID  STATE");
    for info in task::snapshot() {
        println!("  {}  {}", info.pid, info.state);
    }
}

fn cmd_lspci() {
    println!("BUS:DEV.FN  VID:PID  CLASS");
    crate::drivers::pci::enumerate(|d| {
        println!(
            "{:02X}:{:02X}.{}  {:04X}:{:04X}  {}",
            d.bus, d.device, d.function, d.vendor, d.device_id,
            crate::drivers::pci::class_name(d.class, d.subclass)
        );
    });
}

fn cmd_disk(args: &[&str]) {
    if args.len() < 2 || args[0] != "read" {
        println!("usage: disk read <lba>");
        return;
    }
    let lba: u32 = match args[1].parse() {
        Ok(v) => v,
        Err(_) => {
            println!("disk: bad LBA '{}'", args[1]);
            return;
        }
    };
    let mut buf = [0u8; 512];
    match crate::drivers::ata::read_sector(lba, &mut buf) {
        Ok(()) => {
            println!("Sector LBA {} (512 bytes):", lba);
            for (i, chunk) in buf.chunks(16).enumerate() {
                print!("{:08x}  ", i * 16);
                for b in chunk { print!("{:02x} ", b); }
                print!(" |");
                for b in chunk {
                    let c = if *b >= 0x20 && *b < 0x7f { *b } else { b'.' };
                    print!("{}", c as char);
                }
                println!("|");
            }
            let sig = (buf[510] as u16) | ((buf[511] as u16) << 8);
            println!("Boot signature: {:#06x} (expect 0x55aa for MBR)", sig);
        }
        Err(e) => println!("disk read failed: {}", e),
    }
}

fn cmd_spawn(name: Option<&&str>) {
    let n = name.unwrap_or(&"demo");
    match *n {
        "dots" => task::spawn(dots_task),
        "letters" => task::spawn(letters_task),
        _ => {
            println!("known tasks: dots, letters");
            println!("spawning 'dots' by default...");
            task::spawn(dots_task);
        }
    }
}

/// Load the embedded user program into Ring 3 and run it.
fn cmd_run() {
    println!("[shell] launching user program (Ring 3)...");
    match crate::user::run_user_program() {
        Ok(()) => unreachable!(), // jump_user never returns
        Err(e) => println!("[shell] failed to load user program: {}", e),
    }
}

/// Demo background task: print a dot every 200ms.
fn dots_task() -> ! {
    loop {
        crate::print!(".");
        task::sleep_ms(200);
    }
}

/// Demo background task: print 'A' every 300ms.
fn letters_task() -> ! {
    loop {
        crate::print!("A");
        task::sleep_ms(300);
    }
}

fn cmd_ls(path: Option<&&str>) {
    fs::with(|f| {
        let resolved = f.resolve(path.unwrap_or(&"."));
        match f.get(&resolved) {
            Some(node) if node.is_dir() => {
                let entries = f.list(&resolved);
                if entries.is_empty() {
                    println!("(empty)");
                } else {
                    for (name, kind, size) in entries {
                        match kind {
                            crate::fs::FileKind::Dir => println!("{}/", name),
                            crate::fs::FileKind::SymLink => println!("{} -> (link)", name),
                            crate::fs::FileKind::Device => println!("{} [dev]", name),
                            _ => println!("{} ({})", name, size),
                        }
                    }
                }
            }
            Some(_) => println!("{}", resolved),
            None => println!("ls: cannot access '{}': no such file or directory", resolved),
        }
    });
}

fn cmd_cd(path: Option<&&str>) {
    let Some(p) = path else {
        fs::with(|f| f.cwd = "/home".into());
        return;
    };
    fs::with(|f| {
        let resolved = f.resolve(p);
        match f.get(&resolved) {
            Some(node) if node.is_dir() => f.cwd = resolved,
            Some(_) => println!("cd: not a directory: {}", resolved),
            None => println!("cd: no such file or directory: {}", resolved),
        }
    });
}

fn cmd_mkdir(path: Option<&&str>) {
    let Some(p) = path else { println!("mkdir: missing operand"); return; };
    fs::with(|f| {
        let resolved = f.resolve(p);
        if !f.create_dir(&resolved) {
            println!("mkdir: cannot create '{}': already exists", resolved);
        }
    });
}

fn cmd_touch(path: Option<&&str>) {
    let Some(p) = path else { println!("touch: missing operand"); return; };
    fs::with(|f| {
        let resolved = f.resolve(p);
        if !f.create_file(&resolved) {
            println!("touch: cannot create '{}': already exists", resolved);
        }
    });
}

fn cmd_cat(path: Option<&&str>) {
    let Some(p) = path else { println!("cat: missing operand"); return; };
    fs::with(|f| {
        let resolved = f.resolve(p);
        match f.get(&resolved) {
            Some(node) if node.kind == crate::fs::FileKind::File || node.kind == crate::fs::FileKind::SymLink => {
                let text = core::str::from_utf8(&node.content).unwrap_or("<binary>");
                print!("{}", text);
                if !text.ends_with('\n') { println!(); }
            }
            Some(_) => println!("cat: {}: is a directory", resolved),
            None => println!("cat: {}: no such file", resolved),
        }
    });
}

fn cmd_rm(path: Option<&&str>) {
    let Some(p) = path else { println!("rm: missing operand"); return; };
    fs::with(|f| {
        let resolved = f.resolve(p);
        if let Err(e) = f.remove(&resolved) {
            println!("rm: cannot remove '{}': {}", resolved, e);
        }
    });
}

/// `ln <target> <link>` — create a PurityFS symlink.
fn cmd_ln(target: Option<&&str>, link: Option<&&str>) {
    let (Some(t), Some(l)) = (target, link) else {
        println!("ln: usage: ln <target> <link>");
        return;
    };
    fs::with(|f| {
        let resolved = f.resolve(l);
        if f.symlink(&resolved, t) {
            println!("created link {} -> {}", resolved, t);
        } else {
            println!("ln: cannot create '{}': already exists", resolved);
        }
    });
}

/// `dev <path>` — create a device node.
fn cmd_dev(path: Option<&&str>) {
    let Some(p) = path else { println!("dev: usage: dev <path>"); return; };
    fs::with(|f| {
        let resolved = f.resolve(p);
        if f.device(&resolved) {
            println!("created device node {}", resolved);
        } else {
            println!("dev: cannot create '{}': already exists", resolved);
        }
    });
}

/// `fsinfo` — PurityFS statistics.
fn cmd_fsinfo() {
    fs::with(|f| {
        let mut files = 0;
        let mut dirs = 0;
        let mut links = 0;
        let mut devs = 0;
        for n in f.entries.values() {
            match n.kind {
                crate::fs::FileKind::File => files += 1,
                crate::fs::FileKind::Dir => dirs += 1,
                crate::fs::FileKind::SymLink => links += 1,
                crate::fs::FileKind::Device => devs += 1,
            }
        }
        println!("PurityFS v2");
        println!("  nodes   : {} total ({} dir, {} file, {} link, {} dev)",
            f.entries.len(), dirs, files, links, devs);
        println!("  data    : {} bytes", f.used_bytes());
        println!("  cwd     : {}", f.cwd);
    });
}

fn cmd_write(args: &[&str]) {
    if args.len() < 2 {
        println!("usage: write <path> <text...>");
        return;
    }
    let path = args[1];
    let text = args[2..].join(" ");
    fs::with(|f| {
        let resolved = f.resolve(path);
        if f.get(&resolved).is_none() {
            f.create_file(&resolved);
        }
        if let Some(node) = f.get_mut(&resolved) {
            if node.is_dir() {
                println!("write: {}: is a directory", resolved);
                return;
            }
            node.content.extend_from_slice(text.as_bytes());
            node.content.push(b'\n');
            node.size = node.content.len() as u64;
        }
    });
}

fn cmd_color(name: Option<&&str>) {
    let Some(n) = name else {
        println!("current fg color index: {}", THEME.lock().fg as u8);
        return;
    };
    let fg = match *n {
        "black" => Color::Black, "red" => Color::Red, "green" => Color::Green,
        "yellow" => Color::Yellow, "blue" => Color::Blue, "magenta" => Color::Magenta,
        "cyan" => Color::Cyan, "white" => Color::White, "gray" | "grey" => Color::LightGray,
        "lightred" => Color::LightRed, "lightgreen" => Color::LightGreen,
        "lightcyan" => Color::LightCyan, "lightblue" => Color::LightBlue,
        _ => { println!("color: unknown '{}'", n); return; }
    };
    THEME.lock().fg = fg;
    apply_theme();
    println!("terminal color set.");
}

fn cmd_calc(args: &[&str]) {
    if args.is_empty() {
        println!("usage: calc <a> <op> <b> [op c ...]");
        return;
    }
    let mut acc: f64 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => { println!("calc: bad number '{}'", args[0]); return; }
    };
    let mut i = 1;
    while i + 1 < args.len() + 1 {
        let op = match args.get(i) { Some(o) => *o, None => break };
        let rhs: f64 = match args.get(i + 1).and_then(|v| v.parse().ok()) {
            Some(v) => v, None => break,
        };
        match op {
            "+" => acc += rhs,
            "-" => acc -= rhs,
            "*" => acc *= rhs,
            "/" => { if rhs == 0.0 { println!("calc: division by zero"); return; } acc /= rhs; }
            _ => { println!("calc: unknown operator '{}'", op); return; }
        }
        i += 2;
    }
    println!("{}", acc);
}

fn cmd_hexdump(path: Option<&&str>) {
    let Some(p) = path else { println!("hexdump: missing operand"); return; };
    fs::with(|f| {
        let resolved = f.resolve(p);
        match f.get(&resolved) {
            Some(node) if node.kind == crate::fs::FileKind::File || node.kind == crate::fs::FileKind::SymLink => {
                for (i, chunk) in node.content.chunks(16).enumerate() {
                    print!("{:08x}  ", i * 16);
                    for b in chunk { print!("{:02x} ", b); }
                    for _ in chunk.len()..16 { print!("   "); }
                    print!(" |");
                    for b in chunk {
                        let c = if *b >= 0x20 && *b < 0x7f { *b } else { b'.' };
                        print!("{}", c as char);
                    }
                    println!("|");
                }
            }
            Some(_) => println!("hexdump: {}: is a directory", resolved),
            None => println!("hexdump: {}: no such file", resolved),
        }
    });
}

fn cmd_rand() {
    let mut state = (timer::ticks() as u32) | 1;
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    println!("{}", state);
}
