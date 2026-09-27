//! Window manager: desktop, taskbar, windows, icons and input dispatch.
//!
//! Style blends Windows (taskbar + title bars), macOS (top menu bar with
//! clock) and UOS (rounded corners, desktop icons).

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use crate::gui::fb::{self, Color};
use crate::gui::font;

pub const TITLE_H: usize = 26;
pub const TASKBAR_H: usize = 44;
pub const MENUBAR_H: usize = 30;
/// Start-menu entry hit/draw geometry (shared by draw_start_menu and on_click).
pub const START_MENU_X: usize = 16;
pub const START_MENU_W: usize = 204;
pub const ICON_W: usize = 72;
pub const ICON_H: usize = 86;

/// Terminal text buffer (up to 512 scrollback lines, 160 cols).
#[derive(Clone)]
pub struct TermBuf {
    lines: Vec<Vec<u8>>,
    x: usize,
    y: usize,
}

impl TermBuf {
    pub fn new() -> Self {
        let mut lines = Vec::new();
        lines.push(Vec::new());
        TermBuf { lines, x: 0, y: 0 }
    }

    fn ensure_line(&mut self) {
        if self.y >= self.lines.len() {
            self.lines.push(Vec::new());
            if self.lines.len() > 512 {
                self.lines.remove(0);
                if self.y > 0 {
                    self.y -= 1;
                }
            }
        }
    }

    pub fn put(&mut self, b: u8) {
        match b {
            b'\n' => {
                self.y += 1;
                self.x = 0;
                self.ensure_line();
            }
            0x08 => {
                if self.x > 0 {
                    self.x -= 1;
                    self.lines[self.y].pop();
                }
            }
            0x0c | 0x1b => {} // form feed / escape ignored
            c if c >= 0x20 => {
                self.ensure_line();
                if self.x >= 160 {
                    self.y += 1;
                    self.x = 0;
                    self.ensure_line();
                }
                if self.x < 160 {
                    self.lines[self.y].push(c);
                    self.x += 1;
                }
            }
            _ => {}
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(Vec::new());
        self.x = 0;
        self.y = 0;
    }

    /// Draw the visible tail of the buffer into the framebuffer.
    pub fn draw(&self, fb: &mut fb::Fb, x: usize, y: usize, w: usize, h: usize, fg: Color) {
        let cols = w / font::CHAR_W;
        let rows = h / font::CHAR_H;
        let visible = self.lines.len().min(rows.max(1));
        let start = self.lines.len().saturating_sub(visible);
        for r in 0..visible {
            let line = &self.lines[start + r];
            let mut cx = x;
            for &c in line.iter().take(cols) {
                if c >= 0x20 && c <= 0x7e {
                    fb.glyph(cx, y + r * font::CHAR_H, c, fg, None);
                }
                cx += font::CHAR_W;
            }
        }
        // Cursor: inverse block at the current write position.
        let cur_row = self.lines.len().saturating_sub(1).saturating_sub(start);
        if cur_row < visible {
            let cy = y + cur_row * font::CHAR_H;
            let cx = x + self.x.min(cols) * font::CHAR_W;
            if self.x < cols {
                fb.fill_rect(cx, cy, font::CHAR_W, font::CHAR_H, fb::GRAY);
            }
        }
    }
}

/// Application kinds available from the desktop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppKind {
    Terminal,
    Files,
    Settings,
    Clock,
    Calc,
    Paint,
    Monitor,
    Player,
    Editor,
    HardwareInfo,
    LampDemo,
    Snake,
}

impl AppKind {
    pub fn title(self) -> &'static str {
        match self {
            AppKind::Terminal => "Terminal",
            AppKind::Files => "PurityFS Files",
            AppKind::Settings => "Settings",
            AppKind::Clock => "Clock",
            AppKind::Calc => "Calculator",
            AppKind::Paint => "Paint",
            AppKind::Monitor => "System Monitor",
            AppKind::Player => "Player",
            AppKind::Editor => "Text Editor",
            AppKind::HardwareInfo => "Hardware Info",
            AppKind::LampDemo => "LampGL Demo",
            AppKind::Snake => "Snake",
        }
    }
}

/// Snake game state (grid is GRID x GRID cells). Direction: 0=up,1=right,2=down,3=left.
#[derive(Clone)]
pub struct SnakeGame {
    pub body: alloc::vec::Vec<(u8, u8)>,
    pub dir: u8,
    pub next_dir: u8,
    pub food: (u8, u8),
    pub score: u16,
    pub last_tick: u64,
    pub dead: bool,
}

impl SnakeGame {
    pub const GRID: u8 = 20;
    pub const MOVE_TICKS: u64 = 8;

    pub fn new() -> Self {
        let mut g = SnakeGame {
            body: alloc::vec![(10, 10), (9, 10), (8, 10)],
            dir: 1,
            next_dir: 1,
            food: (15, 10),
            score: 0,
            last_tick: 0,
            dead: false,
        };
        g.last_tick = crate::drivers::timer::ticks();
        g
    }

    /// Advance one step. Returns true if the state changed.
    pub fn step(&mut self) {
        // Commit queued direction unless it directly reverses.
        if (self.next_dir as i8 - self.dir as i8).abs() != 2 {
            self.dir = self.next_dir;
        }
        let (hx, hy) = self.body[0];
        let (nx, ny) = match self.dir {
            0 => (hx, hy.saturating_sub(1)),
            1 => ((hx + 1).min(Self::GRID - 1), hy),
            2 => (hx, (hy + 1).min(Self::GRID - 1)),
            _ => (hx.saturating_sub(1), hy),
        };
        // Wall collision.
        let hit_wall = match self.dir {
            0 => hy == 0,
            1 => hx == Self::GRID - 1,
            2 => hy == Self::GRID - 1,
            _ => hx == 0,
        };
        if hit_wall || self.body.contains(&(nx, ny)) {
            self.dead = true;
            return;
        }
        self.body.insert(0, (nx, ny));
        if (nx, ny) == self.food {
            self.score += 1;
            // Place new food somewhere not on the snake (simple search).
            let ncells = (Self::GRID as u16) * (Self::GRID as u16);
            let mut f: u16 = ((nx as u16) * 7 + (ny as u16) * 13 + self.score as u16 * 5 + 3) % ncells;
            let mut cell = ((f % Self::GRID as u16) as u8, (f / Self::GRID as u16) as u8);
            let mut guard = 0;
            while self.body.contains(&cell) && guard < 200 {
                f = (f + 17) % ncells;
                cell = ((f % Self::GRID as u16) as u8, (f / Self::GRID as u16) as u8);
                guard += 1;
            }
            self.food = cell;
        } else {
            self.body.pop();
        }
    }
}

/// Feed a keystroke to the Snake game (arrows, WASD, or R to restart).
pub fn snake_key(g: &mut SnakeGame, b: u8) {
    match b {
        crate::drivers::keyboard::KEY_UP | 0x77 => g.next_dir = 0, // up / w
        crate::drivers::keyboard::KEY_RIGHT | 0x64 => g.next_dir = 1, // right / d
        crate::drivers::keyboard::KEY_DOWN | 0x73 => g.next_dir = 2, // down / s
        crate::drivers::keyboard::KEY_LEFT | 0x61 => g.next_dir = 3, // left / a
        0x72 => *g = SnakeGame::new(), // R restarts
        _ => {}
    }
}

/// Per-window application state.
#[derive(Clone)]
pub struct AppState {
    pub kind: AppKind,
    pub term: TermBuf,
    pub files_cwd: String,
    pub files_sel: usize,
    pub calc_expr: String,
    pub calc_result: String,
    pub paint_color: Color,
    pub paint_down: bool,
    pub paint_canvas: alloc::vec::Vec<u8>, // 240x180 palette-indexed
    pub settings_tab: usize,
    pub player_idx: usize,
    pub player_playing: bool,
    pub monitor_idx: usize,
    pub editor_lines: alloc::vec::Vec<alloc::string::String>,
    pub editor_cx: usize,
    pub editor_cy: usize,
    pub editor_path: alloc::string::String,
    pub snake: SnakeGame,
}

impl AppState {
    pub fn new(kind: AppKind) -> Self {
        // Only allocate the 42 KiB paint canvas for actual Paint windows;
        // every other app gets an empty buffer.
        let canvas = if kind == AppKind::Paint {
            let mut v = alloc::vec::Vec::new();
            v.resize(240 * 180, 0);
            v
        } else {
            alloc::vec::Vec::new()
        };
        AppState {
            kind,
            term: TermBuf::new(),
            files_cwd: String::from("/home"),
            files_sel: 0,
            calc_expr: String::new(),
            calc_result: String::from("0"),
            paint_color: fb::ACCENT,
            paint_down: false,
            paint_canvas: canvas,
            settings_tab: 0,
            player_idx: 0,
            player_playing: false,
            monitor_idx: 0,
            editor_lines: {
                let mut v = alloc::vec::Vec::new();
                v.push(alloc::string::String::new());
                v
            },
            editor_cx: 0,
            editor_cy: 0,
            editor_path: alloc::string::String::from("/home/untitled.txt"),
            snake: SnakeGame::new(),
        }
    }
}

#[derive(Clone)]
pub struct Window {
    pub id: usize,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub state: AppState,
    pub minimized: bool,
    pub topmost: bool,
}

pub struct Wm {
    pub windows: Vec<Window>,
    pub focus: Option<usize>,
    next_id: usize,
    drag: Option<(usize, isize, isize)>, // (win, grab offsets)
    resize: Option<(usize, isize, isize)>, // (win, initial w, initial h)
    prev_left: bool,
    pub start_menu: bool,
    pub theme: usize, // 0 light-blue, 1 dark, 2 green, 3 purple
    pub wallpaper: usize, // 0 blue gradient, 1 dark, 2 green, 3 pink, 4 solid
    pub locked: bool,
    pub menu: Option<Menu>,
    pub notifications: Vec<Notification>,
    pub heap_history: [u64; 64],
    pub heap_hist_idx: usize,
    kbd_buf: alloc::collections::VecDeque<u8>,
    /// When set, a modal confirmation dialog is shown (shutdown / restart).
    pub confirm: Option<ConfirmKind>,
}

/// Modal confirmation dialog kind.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    Shutdown,
    Restart,
}

/// A popup context menu.
#[derive(Clone)]
pub struct Menu {
    pub x: usize,
    pub y: usize,
    pub items: Vec<&'static str>,
    pub target: MenuTarget,
}

/// What a context menu acts on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuTarget {
    Desktop,
    TitleBar(usize), // window id
}

/// A transient notification bubble (top-right).
#[derive(Clone)]
pub struct Notification {
    pub text: String,
    pub born_tick: u64,
}

static WM: Mutex<Wm> = Mutex::new(Wm {
    windows: Vec::new(),
    focus: None,
    next_id: 1,
    drag: None,
    resize: None,
    prev_left: false,
    start_menu: false,
    theme: 0,
    wallpaper: 0,
    locked: true, // boot into the lock screen
    menu: None,
    notifications: Vec::new(),
    heap_history: [0u64; 64],
    heap_hist_idx: 0,
    kbd_buf: alloc::collections::VecDeque::new(),
    confirm: None,
});

/// Route a keypress from the keyboard IRQ. Returns true if the GUI
/// absorbed it. Runs in interrupt context: use try_lock so we never block
/// on a lock the interrupted kernel code may already hold.
pub fn route_key(b: u8) -> bool {
    use core::sync::atomic::Ordering;
    if !KBD_GRAB.load(Ordering::Relaxed) {
        return false;
    }
    if let Some(mut wm) = WM.try_lock() {
        wm.kbd_buf.push_back(b);
        true
    } else {
        false // lock busy (e.g. mid-frame); drop the byte rather than deadlock
    }
}

static KBD_GRAB: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn set_kbd_grab(g: bool) {
    KBD_GRAB.store(g, core::sync::atomic::Ordering::Relaxed);
}

/// Open the wm for direct access.
pub fn lock() -> spin::MutexGuard<'static, Wm> {
    WM.lock()
}

/// Non-blocking open of the wm. Returns `None` when the lock is held
/// elsewhere — used by the print path so it can fall back instead of
/// deadlocking the kernel.
pub fn try_lock() -> Option<spin::MutexGuard<'static, Wm>> {
    WM.try_lock()
}

pub fn init() {
    let mut wm = WM.lock();
    wm.open(AppKind::Terminal);
}

pub fn focus_id() -> Option<usize> {
    WM.lock().focus
}

/// Theme accent color.
pub fn accent_with(theme: usize) -> Color {
    match theme {
        1 => fb::PURPLE,
        2 => fb::GREEN,
        3 => fb::PINK,
        _ => fb::ACCENT,
    }
}

pub fn background_with(theme: usize) -> Color {
    match theme {
        1 => 0x15151c,
        2 => 0x17301d,
        3 => 0x2a1b2e,
        _ => 0x1b2a41,
    }
}

impl Wm {
    /// Open a window of the given kind (or focus an existing one).
    pub fn open(&mut self, kind: AppKind) {
        if let Some(w) = self.windows.iter().find(|w| w.state.kind == kind && !w.minimized) {
            self.focus = Some(w.id);
            return;
        }
        let (sw, sh) = fb::dims();
        let (ww, wh) = default_size(kind, sw, sh);
        let n = self.windows.len();
        let x = 60 + (n % 5) * 36;
        let y = 60 + (n % 4) * 28;
        let id = self.next_id;
        self.next_id += 1;
        let win = Window {
            id,
            x: x.min(sw.saturating_sub(ww)),
            y: y.min(sh.saturating_sub(wh)),
            w: ww,
            h: wh,
            state: AppState::new(kind),
            minimized: false,
            topmost: false,
        };
        self.windows.push(win);
        self.focus = Some(id);
    }

    /// Open the text editor and load the given file's contents into it.
    pub fn open_editor_with(&mut self, path: &str) {
        let real = crate::fs::with(|vfs| vfs.follow(&vfs.resolve(path)));
        let data = crate::fs::with(|vfs| vfs.read_file(&real));
        // Reuse an existing (non-minimized) editor if present.
        let existing = self
            .windows
            .iter()
            .position(|w| w.state.kind == AppKind::Editor && !w.minimized);
        let idx = match existing {
            Some(i) => i,
            None => {
                self.open(AppKind::Editor);
                self.windows
                    .iter()
                    .position(|w| w.state.kind == AppKind::Editor)
                    .unwrap()
            }
        };
        let st = &mut self.windows[idx].state;
        st.editor_path = real.clone();
        st.editor_lines.clear();
        if let Some(bytes) = data {
            let text = alloc::string::String::from_utf8_lossy(&bytes);
            for line in text.split('\n') {
                st.editor_lines.push(alloc::string::String::from(line));
            }
        }
        if st.editor_lines.is_empty() {
            st.editor_lines.push(alloc::string::String::new());
        }
        st.editor_cy = st.editor_lines.len() - 1;
        st.editor_cx = st.editor_lines[st.editor_cy].len();
        self.focus = Some(self.windows[idx].id);
    }

    pub fn close(&mut self, id: usize) {
        self.windows.retain(|w| w.id != id);
        if self.focus == Some(id) {
            self.focus = self.windows.last().map(|w| w.id);
        }
    }

    pub fn raise(&mut self, id: usize) {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            let w = self.windows.remove(i);
            self.windows.push(w);
            self.focus = Some(id);
        }
    }

    /// Terminal output: write formatted text into the focused terminal, or
    /// any terminal window if none is focused.
    pub fn term_write(&mut self, s: &str) {
        let id = match self.focus {
            Some(i) if self.windows.iter().any(|w| w.id == i && w.state.kind == AppKind::Terminal) => i,
            _ => match self.windows.iter().find(|w| w.state.kind == AppKind::Terminal) {
                Some(w) => w.id,
                None => return,
            },
        };
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            for b in s.bytes() {
                w.state.term.put(b);
            }
        }
    }
}

fn default_size(kind: AppKind, sw: usize, sh: usize) -> (usize, usize) {
    match kind {
        AppKind::Terminal => (sw * 3 / 4, sh * 3 / 4),
        AppKind::Files => (560, 420),
        AppKind::Settings => (520, 380),
        AppKind::Clock => (360, 260),
        AppKind::Calc => (280, 360),
        AppKind::Paint => (560, 460),
        AppKind::Monitor => (480, 320),
        AppKind::Player => (420, 300),
        AppKind::Editor => (640, 420),
        AppKind::HardwareInfo => (480, 360),
        AppKind::LampDemo => (560, 460),
        AppKind::Snake => (480, 520),
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Draw the whole desktop (background, windows, taskbar, menu bar, cursor).
pub fn draw_all() {
    fb::with(|f| {
        let (sw, sh) = (f.width, f.height);
        let (wm_mx, wm_my, _, _, _) = crate::drivers::mouse::state();
        let wm = WM.lock();

        // Lock screen: cover everything until the user clicks.
        if wm.locked {
            draw_lock_screen(f, sw, sh);
            return;
        }

        // Desktop background by selected wallpaper.
        let bg = background_with(wm.theme);
        match wm.wallpaper {
            4 => f.fill_rect(0, 0, sw, sh, 0x101418), // solid
            _ => f.vgradient(0, 0, sw, sh, bg, fb::DARKER),
        }

        // Desktop icons.
        let kinds = [
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
        ];
        let icon_start_y = MENUBAR_H + 24;
        for (i, k) in kinds.iter().enumerate() {
            let cx = 28 + (i % 4) * (ICON_W + 18);
            let cy = icon_start_y + (i / 4) * (ICON_H + 16);
            crate::gui::apps::draw_icon(f, *k, cx, cy);
        }

        // Windows: non-topmost first, topmost last (so topmost wins).
        for w in wm.windows.iter().filter(|w| !w.minimized && !w.topmost) {
            draw_window(f, w, wm.focus == Some(w.id), &wm.heap_history, wm.heap_hist_idx, wm.theme);
        }
        for w in wm.windows.iter().filter(|w| !w.minimized && w.topmost) {
            draw_window(f, w, wm.focus == Some(w.id), &wm.heap_history, wm.heap_hist_idx, wm.theme);
        }

        // Top menu bar (macOS-ish).
        f.fill_round_rect(0, 0, sw, MENUBAR_H, 0, 0x2b2b33);
        f.text(14, 7, "  PurityOS", fb::WHITE, None);
        f.text(90, 7, "v0.4.0", fb::LIGHT_GRAY, None);
        let (w_, h_) = (f.width, f.height);
        f.text(w_.saturating_sub(300), 7, &alloc::format!("{}", crate::drivers::rtc::now()), fb::WHITE, None);
        let upt = crate::drivers::timer::uptime_seconds();
        let up_str = alloc::format!("up {:.1}s", upt);
        f.text(w_.saturating_sub(300) - (up_str.len() * font::CHAR_W + 24), 7, &up_str, fb::LIGHT_GRAY, None);
        // right side accents
        f.fill_round_rect(w_ - 8, 6, 4, 18, 2, fb::GREEN);

        // Bottom taskbar (Windows/UOS-ish).
        let tb_y = sh - TASKBAR_H;
        f.fill_round_rect(0, tb_y, sw, TASKBAR_H, 0, 0x26262e);
        f.hline(0, tb_y, sw, 0x3a3a44);

        // Start button.
        let start_x = 10;
        f.fill_round_rect(start_x, tb_y + 6, 96, TASKBAR_H - 12, 8, accent_with(wm.theme));
        f.text(start_x + 12, tb_y + 14, "Start", fb::WHITE, None);

        // Taskbar window buttons.
        let mut bx = start_x + 110;
        for w in &wm.windows {
            let active = wm.focus == Some(w.id) && !w.minimized;
            let bgc = if active { 0x3d4b66 } else { 0x33333c };
            f.fill_round_rect(bx, tb_y + 6, 150, TASKBAR_H - 12, 6, bgc);
            f.text(bx + 10, tb_y + 14, w.state.kind.title(), fb::WHITE, None);
            if w.minimized {
                f.fill_round_rect(bx + 138, tb_y + 18, 6, 6, 2, fb::ORANGE);
            }
            bx += 160;
            if bx > w_.saturating_sub(280) {
                break;
            }
        }

        // System tray area.
        f.fill_round_rect(w_ - 190, tb_y + 6, 180, TASKBAR_H - 12, 6, 0x2e2e36);
        let (mx, my, _, _, _) = crate::drivers::mouse::state();
        let pos = alloc::format!("{:3},{:3}", mx, my);
        f.text(w_ - 176, tb_y + 14, &pos, fb::LIGHT_GRAY, None);
        f.text(w_ - 120, tb_y + 14, &alloc::format!("{}", crate::drivers::rtc::now()), fb::WHITE, None);

        // Start menu (simple popup).
        if wm.start_menu {
            crate::gui::apps::draw_start_menu(f, wm.theme, wm_mx, wm_my);
        }

        // Notification bubbles (top-right, stack downward).
        let now = crate::drivers::timer::ticks();
        let mut ny = MENUBAR_H + 8;
        for n in &wm.notifications {
            let age = now - n.born_tick;
            if age < 200 {
                f.fill_round_rect(sw - 280, ny, 264, 44, 8, 0x233049);
                f.rect(sw - 280, ny, 264, 44, 0x3d6cdf);
                f.text(sw - 268, ny + 8, &n.text, fb::WHITE, None);
                ny += 52;
            }
        }

        // Context menu popup.
        if let Some(menu) = &wm.menu {
            let mh = menu.items.len() * 26 + 10;
            f.fill_round_rect(menu.x, menu.y, 180, mh, 8, 0x2a2a34);
            f.rect(menu.x, menu.y, 180, mh, 0x3d3d4a);
            for (i, item) in menu.items.iter().enumerate() {
                f.text(menu.x + 12, menu.y + 8 + i * 26, item, fb::WHITE, None);
            }
        }

        // Modal confirmation dialog (shutdown / restart).
        if let Some(kind) = wm.confirm {
            draw_confirm(f, sw, sh, kind);
        }

        // Resize handle hint on the focused window.
        if let Some(fid) = wm.focus {
            if let Some(w) = wm.windows.iter().find(|w| w.id == fid && !w.minimized) {
                f.text(w.x + w.w - 10, w.y + w.h - 10, "..", fb::GRAY, None);
            }
        }

        // Cursor.
        draw_cursor(f, crate::drivers::mouse::state());
    });
}

fn draw_lock_screen(f: &mut fb::Fb, sw: usize, sh: usize) {
    // Blur-ish dark overlay.
    f.vgradient(0, 0, sw, sh, 0x0a0e1a, 0x1a2340);
    // Center lock card.
    let cw = 360;
    let ch = 180;
    let cx = (sw - cw) / 2;
    let cy = (sh - ch) / 2;
    f.fill_round_rect(cx, cy, cw, ch, 16, 0x16203a);
    f.rect(cx, cy, cw, ch, 0x3d6cdf);
    f.text(cx + cw / 2 - 20, cy + 24, "PurityOS", fb::WHITE, None);
    f.text(cx + cw / 2 - 56, cy + 56, "Locked — click to unlock", fb::LIGHT_GRAY, None);
    f.text(cx + cw / 2 - 60, cy + 90, &alloc::format!("{}", crate::drivers::rtc::now()), fb::CYAN, None);
    draw_cursor(f, crate::drivers::mouse::state());
}

// Confirmation dialog geometry (shared by draw_confirm and on_click).
const CONFIRM_W: usize = 380;
const CONFIRM_H: usize = 160;
const CONFIRM_BTN_Y: usize = 104;
const CONFIRM_BTN_H: usize = 38;

/// Return the dialog's top-left and the two button rects (Yes, No).
fn confirm_layout(sw: usize, sh: usize) -> (usize, usize, (usize, usize, usize, usize), (usize, usize, usize, usize)) {
    let cx = (sw.saturating_sub(CONFIRM_W)) / 2;
    let cy = (sh.saturating_sub(CONFIRM_H)) / 2;
    let yes = (cx + 36, cy + CONFIRM_BTN_Y, 130, CONFIRM_BTN_H);
    let no = (cx + CONFIRM_W - 36 - 130, cy + CONFIRM_BTN_Y, 130, CONFIRM_BTN_H);
    (cx, cy, yes, no)
}

fn draw_confirm(f: &mut fb::Fb, sw: usize, sh: usize, kind: ConfirmKind) {
    // Dim the whole screen behind the modal.
    f.fill_rect(0, 0, sw, sh, 0x05070d);
    let (cx, cy, yes, no) = confirm_layout(sw, sh);
    f.fill_round_rect(cx, cy, CONFIRM_W, CONFIRM_H, 14, 0x1c2230);
    f.rect(cx, cy, CONFIRM_W, CONFIRM_H, 0x3d6cdf);
    let title = if kind == ConfirmKind::Shutdown { "Shut down PurityOS?" } else { "Restart PurityOS?" };
    f.text(cx + 24, cy + 28, title, fb::WHITE, None);
    f.text(cx + 24, cy + 60, "Unsaved work will be stopped. FS is flushed first.", fb::LIGHT_GRAY, None);
    f.fill_round_rect(yes.0, yes.1, yes.2, yes.3, 8, 0xc0392b);
    f.text(yes.0 + 48, yes.1 + 11, "Yes", fb::WHITE, None);
    f.fill_round_rect(no.0, no.1, no.2, no.3, 8, 0x3a3a46);
    f.text(no.0 + 50, no.1 + 11, "No", fb::WHITE, None);
}

fn draw_window(f: &mut fb::Fb, w: &Window, focused: bool, hist: &[u64; 64], hidx: usize, theme: usize) {
    let title_color = if focused { accent_with(theme) } else { 0x5a5a66 };

    // Shadow.
    f.fill_round_rect(w.x + 4, w.y + 6, w.w, w.h, 10, 0x0a0a10);

    // Body.
    let body = if focused { 0x202028 } else { 0x1a1a22 };
    f.fill_round_rect(w.x, w.y, w.w, w.h, 10, body);
    f.rect(w.x, w.y, w.w, w.h, 0x3d3d4a);

    // Title bar.
    let tb_y = w.y + 4;
    f.fill_round_rect(w.x + 2, tb_y, w.w - 4, TITLE_H, 8, title_color);
    f.text(w.x + 10, tb_y + 5, w.state.kind.title(), fb::WHITE, None);

    // Window buttons: close (red) and minimize (orange).
    let btn_x = w.x + w.w - 34;
    f.fill_round_rect(btn_x, tb_y + 5, 24, 18, 5, fb::RED);
    f.text(btn_x + 8, tb_y + 6, "x", fb::WHITE, None);
    let min_x = w.x + w.w - 62;
    f.fill_round_rect(min_x, tb_y + 5, 24, 18, 5, fb::ORANGE);
    f.text(min_x + 8, tb_y + 6, "_", fb::WHITE, None);

    // Content area.
    let cx = w.x + 6;
    let cy = w.y + TITLE_H + 6;
    let cw = w.w - 12;
    let ch = w.h - TITLE_H - 12;

    match w.state.kind {
        AppKind::Terminal => w.state.term.draw(f, cx, cy, cw, ch, fb::WHITE),
        _ => crate::gui::apps::draw_app(f, &w.state, cx, cy, cw, ch, hist, hidx),
    }
}

fn draw_cursor(f: &mut fb::Fb, (x, y, _, _, _): (usize, usize, bool, bool, bool)) {
    // Simple arrow: black outline + white fill.
    for (i, j) in [(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (1, 1), (2, 1), (0, 2), (1, 2), (0, 3)] {
        f.pixel(x + i, y + j, fb::BLACK);
        f.pixel(x + i + 1, y + j, fb::BLACK);
    }
    for (i, j) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2)] {
        f.pixel(x + i, y + j, fb::WHITE);
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Called every tick: read the mouse, apply drags/clicks, re-render.
pub fn tick() {
    let (sw, sh) = fb::dims();
    if sw == 0 {
        return;
    }
    crate::drivers::mouse::update_position(sw, sh);
    let (mx, my, left, right, _) = crate::drivers::mouse::state();

    let mut wm = WM.lock();
    let clicked = left && !wm.prev_left;
    let released = !left && wm.prev_left;
    wm.prev_left = left;

    // Lock screen: any left click unlocks.
    if wm.locked {
        if clicked {
            wm.locked = false;
            wm.notifications.push(Notification {
                text: String::from("Welcome to PurityOS v0.4.0"),
                born_tick: crate::drivers::timer::ticks(),
            });
        }
        drop(wm);
        draw_all();
        return;
    }

    // Sample heap usage for the monitor graph.
    let (used, _total) = crate::mem::heap_stats();
    let i = wm.heap_hist_idx;
    wm.heap_history[i] = used as u64;
    wm.heap_hist_idx = (i + 1) % 64;

    // Keyboard focus: grab keystrokes when the focused window accepts text/keys.
    let focus_kind = wm
        .focus
        .and_then(|fid| wm.windows.iter().find(|w| w.id == fid))
        .map(|w| w.state.kind);
    let grab = matches!(focus_kind, Some(AppKind::Editor) | Some(AppKind::Snake));
    set_kbd_grab(grab);
    if grab {
        // Drain kbd_buf directly from the already-held wm guard.
        let buf: alloc::vec::Vec<u8> = wm.kbd_buf.drain(..).collect();
        if let Some(fid) = wm.focus {
            if let Some(w) = wm.windows.iter_mut().find(|w| w.id == fid) {
                match w.state.kind {
                    AppKind::Editor => {
                        for b in buf {
                            crate::gui::apps::editor_key(&mut w.state, b);
                        }
                    }
                    AppKind::Snake => {
                        for b in buf {
                            snake_key(&mut w.state.snake, b);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Advance the Snake game on a fixed cadence for every Snake window.
    let now = crate::drivers::timer::ticks();
    for w in wm.windows.iter_mut() {
        if w.state.kind == AppKind::Snake {
            let g = &mut w.state.snake;
            if !g.dead && now.saturating_sub(g.last_tick) >= crate::gui::wm::SnakeGame::MOVE_TICKS {
                g.last_tick = now;
                g.step();
            }
        }
    }

    // Right-button down-edge opens a context menu.
    use core::sync::atomic::{AtomicBool, Ordering};
    static PREV_RIGHT: AtomicBool = AtomicBool::new(false);
    let prev = PREV_RIGHT.swap(right, Ordering::Relaxed);
    let right_edge = right && !prev;
    if right_edge {
        if wm.confirm.is_none() {
            wm.on_right_click(mx, my);
        }
    } else if clicked {
        if wm.confirm.is_some() {
            wm.on_click(mx, my); // modal handles it
        } else {
            wm.menu = None; // left click dismisses any open menu
            wm.on_click(mx, my);
        }
    }

    if released {
        wm.drag = None;
        wm.resize = None;
        // Release paint stroke.
        for w in wm.windows.iter_mut() {
            if w.state.kind == AppKind::Paint && w.state.paint_down {
                w.state.paint_down = false;
            }
        }
    }
    if left {
        wm.on_drag(mx, my);
    }
    drop(wm);

    draw_all();
}

impl Wm {
    fn on_click(&mut self, mx: usize, my: usize) {
        let (sw, sh) = fb::dims();

        // Modal confirmation dialog intercepts every other click.
        if let Some(kind) = self.confirm {
            let (_, _, yes, no) = confirm_layout(sw, sh);
            let inside = |x: usize, y: usize, r: (usize, usize, usize, usize)| {
                x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
            };
            if inside(mx, my, yes) {
                crate::fs::flush_to_disk();
                match kind {
                    ConfirmKind::Shutdown => unsafe {
                        x86_64::instructions::port::Port::<u16>::new(0x604).write(0x3400);
                    },
                    ConfirmKind::Restart => unsafe {
                        // Pulse the 8042 controller reset line.
                        let mut p: x86_64::instructions::port::Port<u8> =
                            x86_64::instructions::port::Port::new(0x64);
                        p.write(0xFE);
                    },
                }
            } else if inside(mx, my, no) {
                self.confirm = None;
            }
            return;
        }

        // Context menu item selection (takes priority).
        if let Some(menu) = &self.menu {
            let mh = menu.items.len() * 26 + 10;
            if mx >= menu.x && mx <= menu.x + 180 && my >= menu.y && my <= menu.y + mh {
                let idx = ((my - menu.y - 10) / 26) as usize;
                if idx < menu.items.len() {
                    let target = menu.target;
                    self.activate_menu(target, idx);
                }
            }
            self.menu = None;
            return;
        }

        // Start menu toggle.
        if my >= sh - TASKBAR_H && mx >= 10 && mx <= 106 {
            self.start_menu = !self.start_menu;
            return;
        }
        if self.start_menu {
            // Click on a start-menu entry opens the app (must match
            // apps::draw_start_menu's geometry).
            let entries = crate::gui::apps::start_menu_entries();
            let menu_h = entries.len() * 34 + 14;
            let ey = sh - TASKBAR_H - menu_h - 8 + 26;
            for (i, k) in entries.iter().enumerate() {
                if mx >= START_MENU_X && mx <= START_MENU_X + START_MENU_W
                    && my >= ey + i * 34 && my <= ey + i * 34 + 30 {
                    self.open(*k);
                    self.start_menu = false;
                    return;
                }
            }
            self.start_menu = false;
            return;
        }

        // Taskbar window buttons -> raise/minimize.
        if my >= sh - TASKBAR_H {
            let mut bx = 116;
            for w in self.windows.clone() {
                if mx >= bx && mx <= bx + 150 {
                    if w.minimized {
                        if let Some(ww) = self.windows.iter_mut().find(|x| x.id == w.id) {
                            ww.minimized = false;
                        }
                        self.raise(w.id);
                    } else if self.focus == Some(w.id) {
                        if let Some(ww) = self.windows.iter_mut().find(|x| x.id == w.id) {
                            ww.minimized = true;
                        }
                    } else {
                        self.raise(w.id);
                    }
                    return;
                }
                bx += 160;
            }
            return;
        }

        // Desktop icons -> open apps.
        let kinds = [
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
        ];
        let icon_start_y = MENUBAR_H + 24;
        for (i, k) in kinds.iter().enumerate() {
            let cx = 28 + (i % 4) * (ICON_W + 18);
            let cy = icon_start_y + (i / 4) * (ICON_H + 16);
            if mx >= cx && mx <= cx + ICON_W && my >= cy && my <= cy + ICON_H {
                self.open(*k);
                return;
            }
        }

        // Windows: topmost first (iterate reversed). Work on index copies so
        // the immutable borrow ends before any mutation.
        for i in (0..self.windows.len()).rev() {
            let (wmin, wx, wy, ww, wh, wid) = {
                let w = &self.windows[i];
                (w.minimized, w.x, w.y, w.w, w.h, w.id)
            };
            if wmin {
                continue;
            }
            if mx >= wx && mx < wx + ww && my >= wy && my < wy + wh {
                let tb_y = wy + 4;
                let btn_x = wx + ww - 34;
                let min_x = wx + ww - 62;
                if my >= tb_y && my <= tb_y + TITLE_H {
                    if mx >= btn_x && mx <= btn_x + 24 {
                        self.close(wid);
                        return;
                    }
                    if mx >= min_x && mx <= min_x + 24 {
                        if let Some(ww2) = self.windows.iter_mut().find(|x| x.id == wid) {
                            ww2.minimized = true;
                        }
                        self.focus = None;
                        return;
                    }
                    // Drag.
                    self.raise(wid);
                    self.drag = Some((wid, mx as isize - wx as isize, my as isize - wy as isize));
                    return;
                }
                // Content click.
                let cx = wx + 6;
                let cy = wy + TITLE_H + 6;
                let cw = ww - 12;
                let ch = wh - TITLE_H - 12;
                self.raise(wid);
                // Bottom-right resize handle (12 px).
                if mx >= wx + ww - 14 && my >= wy + wh - 14 {
                    self.resize = Some((wid, mx as isize, my as isize));
                    return;
                }
                if mx >= cx && my >= cy {
                    if let Some(ww2) = self.windows.iter_mut().find(|x| x.id == wid) {
                        // Double-click opens the file in the file manager.
                        let dbl = crate::drivers::mouse::consume_double_click();
                        let mut out = alloc::vec::Vec::new();
                        let action = crate::gui::apps::click_app_ex(&mut ww2.state, mx - cx, my - cy, cw, ch, dbl, &mut out);
                        match action {
                            crate::gui::apps::ClickAction::Theme(t) => self.theme = t,
                            crate::gui::apps::ClickAction::OpenEditor(path) => {
                                self.open_editor_with(&path);
                            }
                            crate::gui::apps::ClickAction::None => {}
                        }
                        // Write pending terminal output to the first Terminal window.
                        if !out.is_empty() {
                            if let Some(t) = self.windows.iter_mut().find(|w| w.state.kind == AppKind::Terminal) {
                                for b in out {
                                    t.state.term.put(b);
                                }
                            }
                        }
                    }
                }
                return;
            }
        }

        // Click on empty desktop clears focus.
        self.focus = None;
    }

    fn on_drag(&mut self, mx: usize, my: usize) {
        // Resize drag (bottom-right handle).
        if let Some((id, start_mx, start_my)) = self.resize {
            let (sw, sh) = fb::dims();
            let (ow, oh) = {
                match self.windows.iter().find(|w| w.id == id) {
                    Some(w) => (w.w, w.h),
                    None => return,
                }
            };
            let dw = mx as isize - start_mx;
            let dh = my as isize - start_my;
            if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                let max_w = (sw as isize - w.x as isize).max(240);
                let max_h = (sh as isize - w.y as isize - TASKBAR_H as isize).max(160);
                w.w = (ow as isize + dw).clamp(240, max_w) as usize;
                w.h = (oh as isize + dh).clamp(160, max_h) as usize;
            }
            return;
        }
        if let Some((id, ox, oy)) = self.drag {
            let (sw, sh) = fb::dims();
            if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                let nx = (mx as isize - ox).max(0) as usize;
                let ny = (my as isize - oy).max(MENUBAR_H as isize) as usize;
                w.x = nx.min(sw.saturating_sub(w.w));
                w.y = ny.min(sh.saturating_sub(w.h + TASKBAR_H));
            }
        }
        // Paint stroke.
        if let Some(fid) = self.focus {
            if let Some(w) = self.windows.iter_mut().find(|w| w.id == fid) {
                if w.state.kind == AppKind::Paint && w.state.paint_down {
                    // Must match draw_paint's dynamic canvas size: h there is
                    // the content-area height (w.h - TITLE_H - 12), and the
                    // canvas is clamped to (content_h - 90) leaving room for
                    // the header + palette strip.
                    let cw = (w.w - 20).min(480);
                    let content_h = w.h - TITLE_H - 12;
                    let ch = (cw * 180 / 240).min(content_h - 90);
                    let px = mx.saturating_sub(w.x + 6 + 8);
                    let py = my.saturating_sub(w.y + TITLE_H + 6 + 30);
                    crate::gui::apps::paint_stroke(&mut w.state, px, py, cw, ch);
                }
            }
        }
    }

    /// Right-button down: pop up a context menu at (mx, my).
    fn on_right_click(&mut self, mx: usize, my: usize) {
        // Is the click on a window title bar?
        for i in (0..self.windows.len()).rev() {
            let (min, wx, wy, ww, wid) = {
                let w = &self.windows[i];
                (w.minimized, w.x, w.y, w.w, w.id)
            };
            if min {
                continue;
            }
            if mx >= wx && mx <= wx + ww && my >= wy + 4 && my <= wy + 4 + TITLE_H {
                self.menu = Some(Menu {
                    x: mx,
                    y: my,
                    items: alloc::vec![
                        "Close",
                        "Minimize",
                        if self.windows[i].topmost { "Un-top" } else { "Topmost" },
                    ],
                    target: MenuTarget::TitleBar(wid),
                });
                return;
            }
        }
        // Otherwise: desktop context menu.
        self.menu = Some(Menu {
            x: mx,
            y: my,
            items: alloc::vec![
                "New file",
                "New folder",
                "Open terminal",
                "Open editor",
                "Next theme",
                "Next wallpaper",
                "Lock screen",
                "Shutdown",
                "Restart",
            ],
            target: MenuTarget::Desktop,
        });
    }

    /// Execute a context-menu entry.
    fn activate_menu(&mut self, target: MenuTarget, idx: usize) {
        match target {
            MenuTarget::Desktop => match idx {
                0 => {
                    crate::fs::with(|f| {
                        let path = alloc::format!("/home/newfile{}", f.entries.len());
                        f.create_file(&path);
                    });
                    notify(self, "New file created");
                }
                1 => {
                    crate::fs::with(|f| {
                        let path = alloc::format!("/home/newdir{}", f.entries.len());
                        f.create_dir(&path);
                    });
                    notify(self, "New folder created");
                }
                2 => self.open(AppKind::Terminal),
                3 => self.open(AppKind::Editor),
                4 => self.theme = (self.theme + 1) % 4,
                5 => self.wallpaper = (self.wallpaper + 1) % 5,
                6 => self.locked = true,
                7 => self.confirm = Some(ConfirmKind::Shutdown),
                8 => self.confirm = Some(ConfirmKind::Restart),
                _ => {}
            },
            MenuTarget::TitleBar(wid) => match idx {
                0 => self.close(wid),
                1 => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == wid) {
                        w.minimized = true;
                    }
                }
                2 => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == wid) {
                        w.topmost = !w.topmost;
                    }
                }
                _ => {}
            },
        }
    }
}

fn notify(wm: &mut Wm, text: &str) {
    wm.notifications.push(Notification {
        text: String::from(text),
        born_tick: crate::drivers::timer::ticks(),
    });
}
