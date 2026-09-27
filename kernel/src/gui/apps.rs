//! Built-in GUI applications: Files, Settings, Clock, Calculator, Paint,
//! System Monitor, Player, plus desktop icons and the start menu.

use alloc::string::String;
use crate::drivers::rtc;
use crate::drivers::speaker;
use crate::drivers::timer;
use crate::fs;
use crate::gui::fb::{self, Color};
use crate::gui::font;
use crate::gui::wm::{AppKind, AppState};

/// Palette used by the Paint app (index -> color).
pub const PAINT_PALETTE: [Color; 8] = [
    fb::BLACK, fb::WHITE, fb::RED, fb::ORANGE, fb::YELLOW, fb::GREEN, fb::ACCENT, fb::PURPLE,
];

const PAINT_W: usize = 240;
const PAINT_H: usize = 180;

/// Result of handling an in-app click: an action the window manager should
/// perform (it owns the window list and global theme).
#[derive(Clone)]
pub enum ClickAction {
    None,
    Theme(usize),
    /// Open the given file path in the text editor.
    OpenEditor(alloc::string::String),
}

// ---------------------------------------------------------------------------
// Desktop icons
// ---------------------------------------------------------------------------

fn app_color(kind: AppKind) -> Color {
    match kind {
        AppKind::Terminal => 0x2d6cdf,
        AppKind::Files => 0x1f8a4c,
        AppKind::Settings => 0x5a5a66,
        AppKind::Clock => 0x9b59b6,
        AppKind::Calc => 0x16a085,
        AppKind::Paint => 0xe67e22,
        AppKind::Monitor => 0x2980b9,
        AppKind::Player => 0xc0392b,
        AppKind::Editor => 0x27ae60,
        AppKind::HardwareInfo => 0x8e44ad,
        AppKind::LampDemo => 0x1abc9c,
        AppKind::Snake => 0x2ecc71,
    }
}

/// Draw a desktop icon (rounded tile + glyph + label).
pub fn draw_icon(f: &mut fb::Fb, kind: AppKind, x: usize, y: usize) {
    let c = app_color(kind);
    // Tile with a soft border.
    f.fill_round_rect(x + 4, y + 4, 44, 44, 10, 0x000000aa);
    f.fill_round_rect(x, y, 44, 44, 10, c);
    f.rect(x, y, 44, 44, 0xffffff30);
    // Glyph inside the tile.
    let gx = x + 8;
    let gy = y + 14;
    match kind {
        AppKind::Terminal => {
            f.fill_round_rect(gx, gy, 28, 20, 3, 0x0f1420);
            f.text(gx + 4, gy + 3, ">_", fb::GREEN, None);
        }
        AppKind::Files => {
            f.fill_round_rect(gx + 2, gy + 4, 24, 16, 2, 0xfff6d8);
            f.fill_round_rect(gx + 2, gy + 4, 16, 4, 2, 0xffffff);
            f.fill_round_rect(gx + 8, gy + 9, 12, 6, 2, 0x1f8a4c);
        }
        AppKind::Settings => {
            f.fill_round_rect(gx + 10, gy + 2, 8, 16, 3, 0xd0d0d8);
            f.fill_round_rect(gx + 6, gy + 6, 16, 8, 3, 0xd0d0d8);
            f.fill_round_rect(gx + 10, gy + 9, 8, 3, 2, 0x5a5a66);
        }
        AppKind::Clock => {
            f.fill_round_rect(gx + 2, gy + 2, 24, 24, 12, 0xf5f5f5);
            f.text(gx + 9, gy + 6, ":", fb::PURPLE, None);
            f.hline(gx + 14, gy + 12, 6, fb::PURPLE);
        }
        AppKind::Calc => {
            f.fill_round_rect(gx, gy, 28, 20, 3, 0xe8f8f4);
            f.hline(gx + 3, gy + 3, 22, 0x16a085);
            f.fill_round_rect(gx + 3, gy + 7, 22, 10, 2, 0x16a085);
        }
        AppKind::Paint => {
            f.fill_round_rect(gx, gy + 6, 26, 12, 2, 0xf0f0f0);
            f.fill_round_rect(gx + 18, gy, 6, 16, 2, fb::RED);
            f.fill_round_rect(gx + 3, gy + 8, 14, 4, 2, fb::ORANGE);
        }
        AppKind::Monitor => {
            f.fill_round_rect(gx, gy, 28, 16, 2, 0x0f1420);
            f.fill_round_rect(gx + 4, gy + 3, 20, 10, 2, 0x2980b9);
            f.hline(gx + 10, gy + 17, 8, 0x0f1420);
            f.fill_rect(gx + 8, gy + 17, 12, 3, 0x0f1420);
        }
        AppKind::Player => {
            f.fill_round_rect(gx + 2, gy, 24, 18, 3, 0xffd9c0);
            f.fill_rect(gx + 20, gy + 6, 4, 8, 0xc0392b);
        }
        AppKind::Editor => {
            f.fill_round_rect(gx + 4, gy - 2, 20, 22, 2, 0xffffff);
            f.fill_round_rect(gx + 16, gy - 2, 8, 8, 0, 0x27ae60);
            f.hline(gx + 7, gy + 4, 14, 0x999999);
            f.hline(gx + 7, gy + 8, 14, 0x999999);
            f.hline(gx + 7, gy + 12, 10, 0x999999);
        }
        AppKind::HardwareInfo => {
            f.fill_round_rect(gx + 4, gy + 2, 20, 14, 2, 0xd0d0d8);
            f.fill_rect(gx + 8, gy + 6, 12, 6, 0x8e44ad);
            f.fill_rect(gx + 2, gy + 16, 4, 4, 0x8e44ad);
            f.fill_rect(gx + 22, gy + 16, 4, 4, 0x8e44ad);
        }
        AppKind::LampDemo => {
            f.fill_round_rect(gx + 6, gy - 2, 16, 14, 2, 0x1abc9c);
            f.fill_rect(gx + 12, gy + 2, 16, 14, 0x16a085);
            f.hline(gx + 6, gy + 12, 10, fb::WHITE);
        }
        AppKind::Snake => {
            // Three green dots + red food dot.
            f.fill_round_rect(gx + 4, gy + 6, 8, 8, 2, 0x2ed573);
            f.fill_round_rect(gx + 14, gy + 6, 8, 8, 2, 0x2ed573);
            f.fill_round_rect(gx + 24, gy + 6, 8, 8, 2, 0x7bed9f);
            f.fill_round_rect(gx + 38, gy + 8, 6, 6, 2, 0xe74c3c);
        }
    }
    // Label.
    f.text(x + 6, y + 50, kind.title(), fb::WHITE, None);
}

// ---------------------------------------------------------------------------
// Start menu
// ---------------------------------------------------------------------------

pub fn start_menu_entries() -> [AppKind; 12] {
    [
        AppKind::Terminal,
        AppKind::Files,
        AppKind::Settings,
        AppKind::Clock,
        AppKind::Calc,
        AppKind::Paint,
        AppKind::Monitor,
        AppKind::Player,
        AppKind::Editor,
        AppKind::HardwareInfo,
        AppKind::LampDemo,
        AppKind::Snake,
    ]
}

pub fn draw_start_menu(f: &mut fb::Fb, theme: usize, mx: usize, my: usize) {
    let (_sw, sh) = (f.width, f.height);
    let entries = start_menu_entries();
    let menu_h = entries.len() * 34 + 14;
    let top = sh - crate::gui::wm::TASKBAR_H - menu_h - 8;
    f.fill_round_rect(8, top, 220, menu_h, 12, 0x23232b);
    f.rect(8, top, 220, menu_h, 0x3a3a46);
    f.text(20, top + 8, "PurityOS", fb::WHITE, None);
    let accent = crate::gui::wm::accent_with(theme);
    for (i, k) in entries.iter().enumerate() {
        let ey = top + 26 + i * 34;
        // Highlight the entry under the mouse instead of always the first.
        let hover = mx >= crate::gui::wm::START_MENU_X
            && mx <= crate::gui::wm::START_MENU_X + crate::gui::wm::START_MENU_W
            && my >= ey && my <= ey + 30;
        f.fill_round_rect(
            crate::gui::wm::START_MENU_X, ey, crate::gui::wm::START_MENU_W, 30, 6,
            if hover { accent } else { 0x2a2a34 });
        f.fill_round_rect(crate::gui::wm::START_MENU_X + 6, ey + 5, 20, 20, 6, app_color(*k));
        f.text(crate::gui::wm::START_MENU_X + 34, ey + 8, k.title(), fb::WHITE, None);
    }
}

// ---------------------------------------------------------------------------
// Per-app drawing
// ---------------------------------------------------------------------------

pub fn draw_app(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize, hist: &[u64; 64], idx: usize) {
    match s.kind {
        AppKind::Terminal => {}
        AppKind::Files => draw_files(f, s, x, y, w, h),
        AppKind::Settings => draw_settings(f, s, x, y, w, h),
        AppKind::Clock => draw_clock(f, s, x, y, w, h),
        AppKind::Calc => draw_calc(f, s, x, y, w, h),
        AppKind::Paint => draw_paint(f, s, x, y, w, h),
        AppKind::Monitor => draw_monitor(f, s, x, y, w, h, hist, idx),
        AppKind::Player => draw_player(f, s, x, y, w, h),
        AppKind::Editor => draw_editor(f, s, x, y, w, h),
        AppKind::HardwareInfo => draw_hwinfo(f, s, x, y, w, h),
        AppKind::LampDemo => {
            app_header(f, x, y, w, "LampGL — software rasterizer");
            let t = crate::drivers::timer::ticks();
            let angle = (t as f32) * 0.02;
            crate::lampgl::draw_cube(f, x + 6, y + 32, w - 12, h - 44, angle);
        }
        AppKind::Snake => draw_snake(f, s, x, y, w, h),
    }
}

/// Draw the Snake game board.
fn draw_snake(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Snake  —  arrow keys / WASD, R to restart");
    let g = &s.snake;
    // Board is a square; fit it below the header, centered horizontally.
    let avail_h = h.saturating_sub(40);
    let board = w.min(avail_h).saturating_sub(8);
    let cell = board / crate::gui::wm::SnakeGame::GRID as usize;
    if cell == 0 {
        f.text(x + 8, y + 36, "window too small", fb::GRAY, None);
        return;
    }
    let board = cell * crate::gui::wm::SnakeGame::GRID as usize;
    let bx = x + (w.saturating_sub(board)) / 2;
    let by = y + 36;
    // Board background + grid border.
    f.fill_rect(bx, by, board, board, 0x10141c);
    f.rect(bx, by, board, board, 0x2a3344);
    // Food (red).
    let (fx, fy) = g.food;
    f.fill_rect(bx + fx as usize * cell + 1, by + fy as usize * cell + 1, cell - 2, cell - 2, 0xe74c3c);
    // Snake (head brighter).
    for (i, &(sx, sy)) in g.body.iter().enumerate() {
        let c = if i == 0 { 0x7bed9f } else { 0x2ed573 };
        f.fill_rect(bx + sx as usize * cell + 1, by + sy as usize * cell + 1, cell - 2, cell - 2, c);
    }
    // Score / status line.
    let status = if g.dead {
        alloc::format!("Score: {}  —  GAME OVER, press R", g.score)
    } else {
        alloc::format!("Score: {}", g.score)
    };
    f.text(x + 10, by + board + 6, &status, fb::WHITE, None);
}

/// Top strip of an app: app name + hint.
fn app_header(f: &mut fb::Fb, x: usize, y: usize, w: usize, title: &str) {    f.hline(x, y + 22, w, 0x33333c);
    f.text(x + 4, y + 4, title, fb::LIGHT_GRAY, None);
}

fn draw_files(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "PurityFS File Manager  —  click to open, top button to go up");
    let mut top = y + 30;
    // Address bar.
    let addr = alloc::format!("  {}", s.files_cwd);
    f.fill_round_rect(x + 4, top, w - 8, 24, 6, 0x181820);
    f.text(x + 8, top + 5, &addr, fb::CYAN, None);
    // Up button.
    f.fill_round_rect(x + 4, top, 40, 24, 6, 0x2a2a34);
    f.text(x + 12, top + 5, "^ up", fb::WHITE, None);
    top += 32;

    let entries = fs::with(|fsys| {
        let cwd = s.files_cwd.clone();
        fsys.list(&cwd)
    });
    let cols = (w / 170).max(1);
    let mut idx = 0;
    for (name, kind, size) in entries {
        let cx = x + 8 + (idx % cols) * 170;
        let cy = top + (idx / cols) * 34;
        if cy + 30 > y + h {
            break;
        }
        let selected = idx == s.files_sel;
        let bg = if selected { 0x3d4b66 } else { 0x23232b };
        f.fill_round_rect(cx, cy, 160, 28, 6, bg);
        let icon_c = match kind {
            fs::FileKind::Dir => 0x1f8a4c,
            fs::FileKind::File => 0x2d6cdf,
            fs::FileKind::SymLink => 0xf39c12,
            fs::FileKind::Device => 0x9b59b6,
        };
        f.fill_round_rect(cx + 4, cy + 6, 16, 16, 4, icon_c);
        let label = if name.len() > 18 { &name[..18] } else { &name };
        f.text(cx + 26, cy + 7, label, fb::WHITE, None);
        let size_str = match kind {
            fs::FileKind::Dir => String::from("dir"),
            fs::FileKind::File => alloc::format!("{}B", size),
            fs::FileKind::SymLink => String::from("link"),
            fs::FileKind::Device => String::from("dev"),
        };
        f.text(cx + 26, cy + 16, &size_str, fb::LIGHT_GRAY, None);
        idx += 1;
    }
    if idx == 0 {
        f.text(x + 10, top + 6, "(empty directory)", fb::GRAY, None);
    }

    // Bottom action buttons (same geometry as click_files).
    const BTN_H: usize = 30;
    if h >= BTN_H + 4 {
        let by = h - BTN_H - 4;
        let buttons: [(usize, usize, &str); 3] = [
            (4, 96, "New File"),
            (104, 104, "New Folder"),
            (212, 84, "Delete"),
        ];
        for (bx, bw, label) in buttons {
            f.fill_round_rect(x + bx, by, bw, BTN_H, 6, 0x2d6cdf);
            f.text(x + bx + 8, by + 9, label, fb::WHITE, None);
        }
    }
}

fn draw_settings(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Settings  —  themes & info");
    let mut ty = y + 34;
    f.text(x + 8, ty, "Theme:", fb::WHITE, None);
    ty += 26;
    let themes = [(0x2d6cdf, "Blue"), (0x9b59b6, "Purple"), (0x2ecc71, "Green"), (0xe91e63, "Pink")];
    for (i, (c, name)) in themes.iter().enumerate() {
        let bx = x + 8 + i * 110;
        f.fill_round_rect(bx, ty, 96, 34, 8, *c);
        f.text(bx + 14, ty + 10, name, fb::WHITE, None);
        if i == s.settings_tab {
            f.rect(bx, ty, 96, 34, fb::WHITE);
        }
    }
    ty += 50;
    f.text(x + 8, ty, "About PurityOS", fb::WHITE, None);
    ty += 22;
    for line in [
        "PurityOS v0.4.0 — a from-scratch Rust OS",
        "kernel : x86_64, no_std, pure Rust",
        "shell  : psh (Tab complete, history)",
        "user   : Ring 3 via int 0x80 + ELF loader",
        "fs     : PurityFS v2 (file/dir/link/dev)",
        "mem    : paging + heap + VirtAlloc",
    ] {
        f.text(x + 10, ty, line, fb::LIGHT_GRAY, None);
        ty += 18;
    }
}

fn draw_clock(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Clock & Calendar");
    let now = rtc::now();
    let cx = x + w / 2;
    let time = alloc::format!("{}", now);
    let time_px = time.len() * font::CHAR_W;
    let tx = cx.saturating_sub(time_px / 2).max(x);
    f.text(tx, y + 40, &time, fb::WHITE, None);
    f.hline(x + 20, y + 64, w - 40, 0x33333c);

    // Simple month grid (fixed 31 days, weeks aligned by weekday).
    let (_, month, day) = rtc::date_parts();
    let ty = y + 78;
    let weekday0 = rtc::first_weekday_of_month();
    let label = alloc::format!("Month {} — click a day", month);
    f.text(x + 10, ty, &label, fb::CYAN, None);
    let mut cx = x + 12;
    let mut cy = ty + 24;
    for d in 1..=31u32 {
        let bg = if d == day { 0x2d6cdf } else { 0x2a2a34 };
        f.fill_round_rect(cx, cy, 26, 24, 5, bg);
        let ds = alloc::format!("{}", d);
        f.text(cx + 8, cy + 5, &ds, fb::WHITE, None);
        cx += 32;
        if (d + weekday0) % 7 == 0 {
            cx = x + 12;
            cy += 30;
            if cy + 24 > y + h {
                break;
            }
        }
    }
    let _ = s;
}

fn draw_calc(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Calculator");
    // Display.
    f.fill_round_rect(x + 6, y + 30, w - 12, 40, 8, 0x101016);
    f.text(x + 12, y + 38, &s.calc_expr, fb::LIGHT_GRAY, None);
    f.text(x + 12, y + 48, &s.calc_result, fb::WHITE, None);
    // Buttons: 4 columns x 5 rows.
    let keys: [&str; 20] = [
        "C", "(", ")", "/",
        "7", "8", "9", "*",
        "4", "5", "6", "-",
        "1", "2", "3", "+",
        "0", ".", "=", "B",
    ];
    let bw = (w - 24) / 4;
    let bh = 44;
    let mut i = 0;
    for k in keys {
        let col = i % 4;
        let row = i / 4;
        let bx = x + 8 + col * bw;
        let by = y + 80 + row * bh;
        let is_op = matches!(k, "+" | "-" | "*" | "/" | "=" | "C" | "B");
        let bg = if k == "=" { 0x2d6cdf } else if is_op { 0x33333c } else { 0x23232b };
        f.fill_round_rect(bx, by, bw - 6, bh - 6, 6, bg);
        f.text(bx + (bw - 6) / 2 - 4, by + (bh - 6) / 2 - 6, k, fb::WHITE, None);
        i += 1;
    }
}

fn draw_paint(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Paint  —  drag to draw, palette below");
    if h < 100 {
        f.text(x + 8, y + 30, "window too small", fb::GRAY, None);
        return;
    }
    if s.paint_canvas.is_empty() {
        return;
    }
    // Canvas: scale 240x180 logical pixels to fit the window.
    let cw = (w - 20).min(480);
    let ch = (cw * PAINT_H / PAINT_W).min(h - 90);
    let scale_x = cw / PAINT_W;
    let scale_y = ch / PAINT_H;
    for py in 0..PAINT_H {
        for px in 0..PAINT_W {
            let idx = s.paint_canvas[py * PAINT_W + px] as usize;
            let c = PAINT_PALETTE[idx.min(PAINT_PALETTE.len() - 1)];
            let bx = x + 8 + px * scale_x;
            let by = y + 30 + py * scale_y;
            f.fill_rect(bx, by, scale_x, scale_y, c);
        }
    }
    f.rect(x + 8, y + 30, cw, ch, 0x444450);
    // Palette.
    let py = y + 30 + ch + 14;
    for (i, c) in PAINT_PALETTE.iter().enumerate() {
        let bx = x + 8 + i * 40;
        f.fill_round_rect(bx, py, 34, 26, 6, *c);
        if s.paint_color == *c {
            f.rect(bx, py, 34, 26, fb::WHITE);
        }
    }
}

fn draw_monitor(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize, hist: &[u64; 64], idx: usize) {
    app_header(f, x, y, w, "System Monitor");
    let (heap_used, heap_total) = crate::mem::heap_stats();
    let (user_used, user_total) = crate::mem::user_alloc_stats();
    let phys = crate::mem::phys_stats();
    let uptime = timer::uptime_seconds();

    let mut ty = y + 34;
    f.text(x + 10, ty, &alloc::format!("Uptime   : {:.1} s", uptime), fb::WHITE, None);
    ty += 20;
    f.text(x + 10, ty, &alloc::format!("Heap     : {} / {} KiB", heap_used / 1024, heap_total / 1024), fb::WHITE, None);
    ty += 20;
    f.text(x + 10, ty, &alloc::format!("User VA   : {} / {} MiB", user_used >> 20, user_total >> 20), fb::WHITE, None);
    ty += 20;
    f.text(x + 10, ty, &alloc::format!("Physical : {} MiB free", phys >> 20), fb::WHITE, None);
    ty += 26;

    // Heap usage bar.
    let frac = if heap_total > 0 { heap_used as f64 / heap_total as f64 } else { 0.0 };
    f.text(x + 10, ty, "Heap usage", fb::LIGHT_GRAY, None);
    ty += 18;
    f.fill_round_rect(x + 10, ty, w - 40, 18, 6, 0x181820);
    f.fill_round_rect(x + 10, ty, ((w - 40) as f64 * frac.clamp(0.0, 1.0)) as usize, 18, 6, fb::GREEN);
    ty += 30;

    // Real-time heap history line chart (64 samples).
    f.text(x + 10, ty, "Heap history (last 64 ticks)", fb::LIGHT_GRAY, None);
    ty += 18;
    let chart_h = 60;
    f.fill_round_rect(x + 10, ty, w - 40, chart_h, 4, 0x10141c);
    let maxv = hist.iter().max().copied().unwrap_or(1).max(1) as f64;
    let plot_w = w - 60;
    for i in 0..63 {
        let v = hist[(idx + i) % 64] as f64 / maxv;
        let px = x + 12 + i;
        let py = ty + chart_h - 2 - (v * (chart_h - 6) as f64) as usize;
        if px < x + 10 + plot_w {
            f.pixel(px, py, fb::CYAN);
            f.pixel(px, py + 1, fb::CYAN);
        }
    }
    ty += chart_h + 10;

    // Task list.
    f.text(x + 10, ty, "Tasks", fb::LIGHT_GRAY, None);
    ty += 18;
    for t in crate::task::snapshot() {
        f.text(x + 14, ty, &alloc::format!("#{} {}", t.pid, t.state), fb::WHITE, None);
        ty += 16;
        if ty > y + h - 10 {
            break;
        }
    }
    let _ = s;
}

fn draw_player(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Player  —  PC speaker melodies");
    let tunes = ["do re mi", "twinkle", "beep test"];
    f.text(x + 10, y + 40, "Melodies:", fb::WHITE, None);
    for (i, t) in tunes.iter().enumerate() {
        let by = y + 60 + i * 40;
        let bg = if i == s.player_idx { 0x3d4b66 } else { 0x23232b };
        f.fill_round_rect(x + 10, by, w - 30, 30, 8, bg);
        f.text(x + 20, by + 8, t, fb::WHITE, None);
    }
    // Play/Stop button.
    let btn = if s.player_playing { "■ Stop" } else { "▶ Play" };
    let bx = x + 10;
    let by = y + h - 56;
    f.fill_round_rect(bx, by, 120, 36, 8, if s.player_playing { fb::RED } else { 0x2d6cdf });
    f.text(bx + 30, by + 10, btn, fb::WHITE, None);
}

/// Draw the simple text editor. Keypresses are drained by wm::tick.
fn draw_editor(f: &mut fb::Fb, s: &AppState, x: usize, y: usize, w: usize, h: usize) {
    app_header(f, x, y, w, "Text Editor  —  type to edit");
    if h < 60 {
        f.text(x + 8, y + 30, "window too small", fb::GRAY, None);
        return;
    }
    // White editing area.
    f.fill_rect(x + 6, y + 30, w - 12, h - 40, 0xf4f6f0);
    let line_h = font::CHAR_H + 2;
    let max_lines = (h - 50) / line_h;
    let scroll = s.editor_cy.saturating_sub(max_lines.saturating_sub(2));
    for i in 0..max_lines.min(s.editor_lines.len()) {
        let li = scroll + i;
        if li < s.editor_lines.len() {
            f.text(x + 12, y + 38 + i * line_h, &s.editor_lines[li], fb::BLACK, None);
        }
    }
    // Cursor block.
    let shown = s.editor_cy.saturating_sub(scroll);
    if shown < max_lines {
        let cy = y + 38 + shown * line_h;
        let cx = x + 12 + s.editor_cx * font::CHAR_W;
        f.fill_rect(cx, cy, font::CHAR_W, font::CHAR_H, fb::ACCENT);
    }
}

/// Feed one keypress into the editor state.
pub fn editor_key(s: &mut AppState, b: u8) {
    // Ctrl+S = save to PurityFS and flush to disk.
    if b == 0x13 {
        let text = s.editor_lines.join("\n");
        crate::fs::with(|f| {
            let _ = f.remove(&s.editor_path);
            f.append(&s.editor_path, text.as_bytes());
        });
        crate::fs::flush_to_disk();
        return;
    }
    match b {
        b'\n' => {
            s.editor_cy += 1;
            s.editor_cx = 0;
            while s.editor_lines.len() <= s.editor_cy {
                s.editor_lines.push(alloc::string::String::new());
            }
        }
        0x08 => {
            if s.editor_cx > 0 {
                s.editor_cx -= 1;
                s.editor_lines[s.editor_cy].remove(s.editor_cx);
            } else if s.editor_cy > 0 {
                s.editor_cy -= 1;
                s.editor_cx = s.editor_lines[s.editor_cy].len();
            }
        }
        c if c >= 0x20 => {
            while s.editor_lines.len() <= s.editor_cy {
                s.editor_lines.push(alloc::string::String::new());
            }
            s.editor_lines[s.editor_cy].insert(s.editor_cx, c as char);
            s.editor_cx += 1;
        }
        _ => {}
    }
}

/// Click on editor: just focus (keyboard comes via drain_kbd).
fn click_editor(_s: &mut AppState, _x: usize, _y: usize, _w: usize, _h: usize) {}

/// Hardware info panel: CPU model, memory, PCI count.
fn draw_hwinfo(f: &mut fb::Fb, _s: &AppState, x: usize, y: usize, w: usize, _h: usize) {
    app_header(f, x, y, w, "Hardware Info");
    let mut ty = y + 38;
    // CPU model via CPUID leaf 0x80000002..4.
    let mut model = [0u8; 48];
    for i in 0..3u32 {
        let leaf = 0x80000002 + i;
        let r = unsafe { core::arch::x86_64::__cpuid(leaf) };
        let regs = [r.eax, r.ebx, r.ecx, r.edx];
        for (j, r) in regs.iter().enumerate() {
            let off = (i as usize) * 16 + j * 4;
            model[off] = (r & 0xff) as u8;
            model[off + 1] = ((r >> 8) & 0xff) as u8;
            model[off + 2] = ((r >> 16) & 0xff) as u8;
            model[off + 3] = ((r >> 24) & 0xff) as u8;
        }
    }
    let model_str = core::str::from_utf8(&model).unwrap_or("Unknown").trim();
    f.text(x + 10, ty, &alloc::format!("CPU: {}", model_str), fb::WHITE, None);
    ty += 22;
    let (_, heap_total) = crate::mem::heap_stats();
    let (_, user_total) = crate::mem::user_alloc_stats();
    let phys = crate::mem::phys_stats();
    f.text(x + 10, ty, &alloc::format!("Kernel heap: {} KiB", heap_total / 1024), fb::WHITE, None);
    ty += 20;
    f.text(x + 10, ty, &alloc::format!("User VA space: {} MiB", user_total >> 20), fb::WHITE, None);
    ty += 20;
    f.text(x + 10, ty, &alloc::format!("Physical free: {} MiB", phys >> 20), fb::WHITE, None);
    ty += 22;
    f.text(x + 10, ty, "PCI devices:", fb::LIGHT_GRAY, None);
    ty += 18;
    crate::drivers::pci::enumerate(|dev| {
        if ty < y + _h.saturating_sub(20) {
            f.text(x + 14, ty, &alloc::format!("  bus={:02x} dev={:02x} ven={:04x} dev={:04x} class={:02x}", dev.bus, dev.device, dev.vendor, dev.device_id, dev.class), fb::WHITE, None);
            ty += 16;
        }
    });
}

// ---------------------------------------------------------------------------
// Per-app clicks
// ---------------------------------------------------------------------------

/// Returns the action (if any) the window manager should perform.
pub fn click_app_ex(s: &mut AppState, x: usize, y: usize, w: usize, h: usize, double: bool, pending_out: &mut alloc::vec::Vec<u8>) -> ClickAction {
    match s.kind {
        AppKind::Terminal => ClickAction::None,
        AppKind::Files => click_files(s, x, y, w, h, double, pending_out),
        AppKind::Settings => match click_settings(s, x, y, w, h) {
            Some(t) => ClickAction::Theme(t),
            None => ClickAction::None,
        },
        AppKind::Clock => ClickAction::None,
        AppKind::Calc => {
            click_calc(s, x, y, w, h);
            ClickAction::None
        }
        AppKind::Paint => {
            click_paint(s, x, y, w, h);
            ClickAction::None
        }
        AppKind::Monitor => ClickAction::None,
        AppKind::Player => {
            click_player(s, x, y, w, h);
            ClickAction::None
        }
        AppKind::Editor => ClickAction::None,
        AppKind::HardwareInfo => ClickAction::None,
        AppKind::LampDemo => ClickAction::None,
        AppKind::Snake => ClickAction::None,
    }
}

fn click_files(s: &mut AppState, x: usize, y: usize, w: usize, h: usize, double: bool, pending_out: &mut alloc::vec::Vec<u8>) -> ClickAction {
    // Up button.
    if y >= 30 && y <= 54 && x >= 4 && x <= 44 {
        if s.files_cwd != "/" {
            let cwd = s.files_cwd.clone();
            s.files_cwd = fs::parent_of(&cwd);
            s.files_sel = 0;
        }
        return ClickAction::None;
    }

    // Bottom action buttons: New File / New Folder / Delete.
    const BTN_H: usize = 30;
    if h >= BTN_H + 4 {
        let by = h - BTN_H - 4;
        let buttons: [(usize, usize, &str); 3] = [
            (4, 96, "New File"),
            (104, 104, "New Folder"),
            (212, 84, "Delete"),
        ];
        for (bx, bw, label) in buttons {
            if x >= bx && x <= bx + bw && y >= by && y <= by + BTN_H {
                let cwd = s.files_cwd.clone();
                match label {
                    "New File" => {
                        fs::with(|fsys| {
                            let mut n = 1;
                            loop {
                                let p = fsys.resolve(&format!("{}/file{}.txt", cwd, n));
                                if fsys.create_file(&p) {
                                    break;
                                }
                                n += 1;
                            }
                        });
                    }
                    "New Folder" => {
                        fs::with(|fsys| {
                            let mut n = 1;
                            loop {
                                let p = fsys.resolve(&format!("{}/folder{}", cwd, n));
                                if fsys.create_dir(&p) {
                                    break;
                                }
                                n += 1;
                            }
                        });
                    }
                    "Delete" => {
                        // Delete the selected entry (recursively).
                        let entries = fs::with(|fsys| fsys.list(&cwd));
                        if let Some((name, _, _)) = entries.get(s.files_sel) {
                            let path = fs::with(|fsys| fsys.resolve(&format!("{}/{}", cwd, name)));
                            let _ = fs::with(|fsys| fsys.remove_recursive(&path));
                            s.files_sel = 0;
                        }
                    }
                    _ => {}
                }
                return ClickAction::None;
            }
        }
    }

    // Entries grid.
    let top = 62;
    let cols = (w / 170).max(1);
    let mut idx = 0;
    let entries = fs::with(|fsys| fsys.list(&s.files_cwd.clone()));
    for (name, kind, _) in entries {
        let cx = 8 + (idx % cols) * 170;
        let cy = top + (idx / cols) * 34;
        if x >= cx && x <= cx + 160 && y >= cy && y <= cy + 28 {
            s.files_sel = idx;
            if !double {
                return ClickAction::None; // single click = select only
            }
            // Double click = open.
            let cwd = s.files_cwd.clone();
            let action = fs::with(|fsys| {
                let path = fsys.resolve(&alloc::format!("{}/{}", cwd, name));
                if kind == fs::FileKind::Dir {
                    s.files_cwd = path;
                    s.files_sel = 0;
                    ClickAction::None
                } else if kind == fs::FileKind::SymLink {
                    let real = fsys.follow(&path);
                    s.files_cwd = fs::parent_of(&real);
                    ClickAction::None
                } else if kind == fs::FileKind::File {
                    // File association: open text-like files in the editor.
                    let is_text = name.ends_with(".txt") || name.ends_with(".md")
                        || name.ends_with(".log") || name.ends_with(".rs");
                    if is_text {
                        ClickAction::OpenEditor(path)
                    } else if let Some(data) = fsys.read_file(&path) {
                        pending_out.extend_from_slice(
                            alloc::format!("\n--- {} ({}B) ---\n", name, data.len()).as_bytes());
                        let text = core::str::from_utf8(&data).unwrap_or("<binary>");
                        pending_out.extend_from_slice(text.as_bytes());
                        pending_out.extend_from_slice(b"\n--- end ---\n");
                        ClickAction::None
                    } else {
                        ClickAction::None
                    }
                } else {
                    ClickAction::None
                }
            });
            return action;
        }
        idx += 1;
    }
    ClickAction::None
}

fn click_settings(s: &mut AppState, x: usize, y: usize, w: usize, _h: usize) -> Option<usize> {
    // Theme tiles: same layout as draw_settings.
    if y >= 60 && y <= 94 {
        for i in 0..4 {
            let bx = 8 + i * 110;
            if x >= bx && x <= bx + 96 {
                s.settings_tab = i;
                // Return the requested theme; on_click applies it (we cannot
                // lock WM here because on_click already holds it).
                return Some(i);
            }
        }
    }
    None
}

fn click_calc(s: &mut AppState, x: usize, y: usize, w: usize, _h: usize) {
    let bw = (w - 24) / 4;
    let bh = 44;
    let keys: [&str; 20] = [
        "C", "(", ")", "/",
        "7", "8", "9", "*",
        "4", "5", "6", "-",
        "1", "2", "3", "+",
        "0", ".", "=", "B",
    ];
    for (i, k) in keys.iter().enumerate() {
        let col = i % 4;
        let row = i / 4;
        let bx = 8 + col * bw;
        let by = 80 + row * bh;
        if x >= bx && x <= bx + bw - 6 && y >= by && y <= by + bh - 6 {
            match *k {
                "C" => {
                    s.calc_expr.clear();
                    s.calc_result = String::from("0");
                }
                "B" => {
                    s.calc_expr.pop();
                }
                "=" => {
                    s.calc_result = crate::gui::apps::calc_eval(&s.calc_expr);
                    s.calc_expr.clear();
                }
                other => s.calc_expr.push_str(other),
            }
            return;
        }
    }
}

fn click_paint(s: &mut AppState, x: usize, y: usize, w: usize, h: usize) {
    if s.paint_canvas.is_empty() {
        return;
    }
    // Palette row.
    let cw = (w - 20).min(480);
    let ch = (cw * PAINT_H / PAINT_W).min(h - 90);
    let py = 30 + ch + 14;
    if y >= py && y <= py + 26 {
        for (i, c) in PAINT_PALETTE.iter().enumerate() {
            let bx = 8 + i * 40;
            if x >= bx && x <= bx + 34 {
                s.paint_color = *c;
                return;
            }
        }
    }
    // Canvas: start a stroke.
    if y >= 30 && y <= 30 + ch && x >= 8 && x <= 8 + cw {
        s.paint_down = true;
        paint_stroke(s, x - 8, y - 30, cw, ch);
    }
}

/// Paint a stroke point at window-relative canvas coords. `cw`/`ch` are the
/// actual pixel size of the canvas region drawn by `draw_paint`.
pub fn paint_stroke(s: &mut AppState, x: usize, y: usize, cw: usize, ch: usize) {
    if cw == 0 || ch == 0 {
        return;
    }
    let sx = x * PAINT_W / cw;
    let sy = y * PAINT_H / ch;
    if sx < PAINT_W && sy < PAINT_H {
        paint_at(s, sx, sy);
    }
}

fn paint_at(s: &mut AppState, sx: usize, sy: usize) {
    if sx < PAINT_W && sy < PAINT_H {
        let idx = PAINT_PALETTE.iter().position(|&c| c == s.paint_color).unwrap_or(0);
        s.paint_canvas[sy * PAINT_W + sx] = idx as u8;
    }
}

fn click_player(s: &mut AppState, x: usize, y: usize, w: usize, h: usize) {
    for (i, t) in ["do re mi", "twinkle", "beep test"].iter().enumerate() {
        let by = 60 + i * 40;
        if x >= 10 && x <= w - 30 && y >= by && y <= by + 30 {
            s.player_idx = i;
            return;
        }
    }
    let by = h - 56;
    if x >= 10 && x <= 130 && y >= by && y <= by + 36 {
        if s.player_playing {
            speaker::stop();
            s.player_playing = false;
        } else {
            speaker::play_melody(s.player_idx);
            s.player_playing = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Calculator evaluation (small recursive-descent parser)
// ---------------------------------------------------------------------------

pub fn calc_eval(expr: &str) -> String {
    let mut p = Parser { s: expr, pos: 0 };
    match p.parse_expr() {
        Some(v) if p.pos == expr.len() => {
            if v == (v as i64) as f64 && v.abs() < 1e15 {
                alloc::format!("{}", v as i64)
            } else {
                alloc::format!("{:.6}", v)
            }
        }
        _ => String::from("error"),
    }
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn parse_expr(&mut self) -> Option<f64> {
        let mut v = self.parse_term()?;
        loop {
            self.skip();
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    let r = self.parse_term()?;
                    v += r;
                }
                Some('-') => {
                    self.pos += 1;
                    let r = self.parse_term()?;
                    v -= r;
                }
                _ => return Some(v),
            }
        }
    }

    fn parse_term(&mut self) -> Option<f64> {
        let mut v = self.parse_factor()?;
        loop {
            self.skip();
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    let r = self.parse_factor()?;
                    v *= r;
                }
                Some('/') => {
                    self.pos += 1;
                    let r = self.parse_factor()?;
                    if r == 0.0 {
                        return None;
                    }
                    v /= r;
                }
                _ => return Some(v),
            }
        }
    }

    fn parse_factor(&mut self) -> Option<f64> {
        self.skip();
        match self.peek() {
            Some('(') => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.skip();
                if self.peek() == Some(')') {
                    self.pos += 1;
                }
                Some(v)
            }
            _ => {
                let start = self.pos;
                while self.pos < self.s.len() {
                    let c = self.s.as_bytes()[self.pos];
                    if c.is_ascii_digit() || c == b'.' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                if self.pos == start {
                    return None;
                }
                self.s[start..self.pos].parse::<f64>().ok()
            }
        }
    }

    fn skip(&mut self) {
        while self.pos < self.s.len() && self.s.as_bytes()[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }
}
