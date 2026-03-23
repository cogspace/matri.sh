use std::collections::HashSet;
use std::io::Write;
use std::os::unix::io::{BorrowedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::libc;
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::select::{select, FdSet};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::termios::{self, SetArg};
use nix::sys::time::TimeVal;
use rand::seq::SliceRandom;
use rand::Rng;

// ─── SIGWINCH flag ────────────────────────────────────────────────────────────

static SIGWINCH_RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn sigwinch_handler(_: libc::c_int) {
    SIGWINCH_RECEIVED.store(true, Ordering::Relaxed);
}

// ─── BorrowedFd helper ────────────────────────────────────────────────────────

/// Wrap a raw fd for APIs that require AsFd.
/// SAFETY: caller must ensure fd is valid for the duration of the call.
unsafe fn bfd(fd: RawFd) -> BorrowedFd<'static> {
    BorrowedFd::borrow_raw(fd)
}

// ─── Environment helpers ──────────────────────────────────────────────────────

fn env_float(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_int(name: &str, default: i64) -> i64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_str(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

// ─── Config ───────────────────────────────────────────────────────────────────

struct Config {
    chars: Vec<char>,
    min_speed: f64,
    max_speed: f64,
    min_length: usize,
    speed_scale_rows: f64,
    min_glitch_interval: f64,
    max_glitch_interval: f64,
    scroll_lines: usize,
}

impl Config {
    fn load() -> Self {
        let chars_str = env_str(
            "MATRISH_CHARS",
            "ｦｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789Z|¦+=.*\"<>-:",
        );
        Config {
            chars: chars_str.chars().collect(),
            min_speed: env_float("MATRISH_MIN_SPEED", 1.6),
            max_speed: env_float("MATRISH_MAX_SPEED", 5.2),
            min_length: env_int("MATRISH_MIN_LENGTH", 1) as usize,
            speed_scale_rows: env_float("MATRISH_SPEED_SCALE_ROWS", 20.0),
            min_glitch_interval: env_float("MATRISH_MIN_GLITCH_INTERVAL", 0.004),
            max_glitch_interval: env_float("MATRISH_MAX_GLITCH_INTERVAL", 0.04),
            scroll_lines: env_int("MATRISH_SCROLL_LINES", 3) as usize,
        }
    }
}

// ─── Color / attribute escapes ────────────────────────────────────────────────

fn color_esc(color: vt100::Color, fg: bool) -> String {
    let reset = if fg { 39 } else { 49 };
    let base  = if fg { 38 } else { 48 };
    match color {
        vt100::Color::Default      => format!("\x1b[{reset}m"),
        vt100::Color::Idx(idx)     => format!("\x1b[{base};5;{idx}m"),
        vt100::Color::Rgb(r, g, b) => format!("\x1b[{base};2;{r};{g};{b}m"),
    }
}

fn attr_esc(cell: &vt100::Cell) -> String {
    let mut codes: Vec<&str> = Vec::new();
    if cell.bold()      { codes.push("1"); }
    if cell.italic()    { codes.push("3"); }
    if cell.underline() { codes.push("4"); }
    if cell.inverse()   { codes.push("7"); }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

// ─── Rain column ──────────────────────────────────────────────────────────────

const HEAD: &str = "\x1b[1;97m";    // bold bright white
const NEAR: &str = "\x1b[1;92m";    // bold bright green
const MID:  &str = "\x1b[32m";      // green
const FAR:  &str = "\x1b[2;32m";    // dim green
const TAIL: &str = "\x1b[38;5;22m"; // very dark green

struct Column {
    height: usize,
    head: f64,
    speed: f64,
    length: usize,
    chars: Vec<char>,
    ttg: f64,
}

impl Column {
    fn new(height: usize, stagger: bool, cfg: &Config) -> Self {
        let mut col = Column { height, head: 0.0, speed: 0.0, length: 0, chars: vec![], ttg: 0.0 };
        col.reinit(stagger, cfg);
        col
    }

    fn reinit(&mut self, stagger: bool, cfg: &Config) {
        let mut rng = rand::thread_rng();
        self.speed  = rng.gen_range(cfg.min_speed..cfg.max_speed);
        let max_len = std::cmp::max(cfg.min_length, self.height * 2);
        self.length = rng.gen_range(cfg.min_length..=max_len);
        let h       = self.height.max(1);
        self.chars  = (0..h).map(|_| *cfg.chars.choose(&mut rng).unwrap_or(&'*')).collect();
        self.ttg    = rng.gen_range(cfg.min_glitch_interval..cfg.max_glitch_interval);
        let lo = if stagger { -(self.height as f64 * 0.8) } else { -10.0 };
        let hi = if stagger {  self.height as f64 * 0.8  } else {   0.0 };
        self.head   = rng.gen_range(lo..hi);
    }

    fn tick(&mut self, dt: f64, cfg: &Config) {
        self.head += self.speed * dt * (self.height as f64 / cfg.speed_scale_rows);
        if self.head - self.length as f64 > self.height as f64 {
            self.reinit(false, cfg);
        }
        self.ttg -= dt;
        if self.ttg <= 0.0 && !self.chars.is_empty() {
            let mut rng = rand::thread_rng();
            let idx = rng.gen_range(0..self.chars.len());
            self.chars[idx] = *cfg.chars.choose(&mut rng).unwrap_or(&'*');
            self.ttg = rng.gen_range(cfg.min_glitch_interval..cfg.max_glitch_interval);
        }
    }

    fn cell_at(&self, row: usize) -> Option<(char, &'static str)> {
        let dist = self.head - row as f64;
        if dist < 0.0 || dist > self.length as f64 || self.chars.is_empty() {
            return None;
        }
        let ch   = self.chars[row % self.chars.len()];
        let fade = dist / self.length as f64;
        let color = if      fade < 0.07 { HEAD }
                    else if fade < 0.18 { NEAR }
                    else if fade < 0.45 { MID  }
                    else if fade < 0.70 { FAR  }
                    else                { TAIL };
        Some((ch, color))
    }
}

// ─── Terminal size / ioctl ────────────────────────────────────────────────────

fn get_terminal_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0 && ws.ws_row > 0
        {
            return (ws.ws_col, ws.ws_row);
        }
    }
    (80, 24)
}

fn set_winsize(fd: RawFd, rows: u16, cols: u16) {
    let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws); }
}

// ─── Mouse SGR parsing ────────────────────────────────────────────────────────

struct MouseEvent {
    btn:   u32,
    col:   usize,  // 0-based
    row:   usize,  // 0-based
    press: bool,
}

/// Consume SGR mouse escapes from `data`, push events, return stripped bytes.
fn parse_mouse_events(data: &[u8]) -> (Vec<MouseEvent>, Vec<u8>) {
    let mut events  = Vec::new();
    let mut output  = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if i + 3 <= data.len()
            && data[i]     == b'\x1b'
            && data[i + 1] == b'['
            && data[i + 2] == b'<'
        {
            if let Some((ev, consumed)) = parse_sgr_mouse(&data[i..]) {
                events.push(ev);
                i += consumed;
                continue;
            }
        }
        output.push(data[i]);
        i += 1;
    }
    (events, output)
}

fn parse_sgr_mouse(data: &[u8]) -> Option<(MouseEvent, usize)> {
    // data starts with ESC [ <
    let mut pos = 3usize;
    let mut nums: Vec<u32> = Vec::new();
    let mut cur  = 0u32;
    let mut seen = false;
    while pos < data.len() {
        match data[pos] {
            b'0'..=b'9' => { cur = cur * 10 + (data[pos] - b'0') as u32; seen = true; }
            b';' => { if !seen { return None; } nums.push(cur); cur = 0; seen = false; }
            b'M' | b'm' => {
                if !seen || nums.len() < 2 { return None; }
                nums.push(cur);
                return Some((MouseEvent {
                    btn:   nums[0],
                    col:   (nums[1] as usize).saturating_sub(1),
                    row:   (nums[2] as usize).saturating_sub(1),
                    press: data[pos] == b'M',
                }, pos + 1));
            }
            _ => return None,
        }
        pos += 1;
    }
    None
}

// ─── Cell helpers ─────────────────────────────────────────────────────────────

fn shell_owns_cell(cell: &vt100::Cell) -> bool {
    if cell.is_wide_continuation() { return false; }
    let s = cell.contents();
    let has_char = !s.is_empty() && s != " " && s != "\x00";
    if !has_char {
        return cell.fgcolor() != vt100::Color::Default || cell.inverse();
    }
    true
}

// ─── Clipboard ────────────────────────────────────────────────────────────────

fn clipboard_osc52(text: &str) {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let _ = std::io::stdout().write_all(format!("\x1b]52;c;{b64}\x07").as_bytes());
    let _ = std::io::stdout().flush();
}

fn clipboard_subprocess(text: &str) {
    let candidates: &[&[&str]] = &[
        &["wl-copy"],
        &["xclip", "-selection", "clipboard"],
        &["xsel", "--clipboard", "--input"],
    ];
    for &cmd in candidates {
        if cmd.first().map(|c| has_in_path(c)).unwrap_or(false) {
            if let Ok(mut child) = std::process::Command::new(cmd[0])
                .args(&cmd[1..])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                return;
            }
        }
    }
}

fn has_in_path(cmd: &str) -> bool {
    std::env::var_os("PATH").map(|p| {
        std::env::split_paths(&p).any(|d| d.join(cmd).is_file())
    }).unwrap_or(false)
}

// ─── Application ──────────────────────────────────────────────────────────────

struct MatrixShell {
    cols: u16,
    rows: u16,
    rain: Vec<Column>,
    parser: vt100::Parser,
    master_fd: RawFd,
    shell_pid: libc::pid_t,
    running: bool,
    altscreen: bool,
    scroll_offset: usize,
    sel_start: Option<(usize, usize)>,  // (row, col) 0-based
    sel_end:   Option<(usize, usize)>,
    selecting: bool,
    cfg: Config,
}

impl MatrixShell {
    fn new() -> Self {
        let (cols, rows) = get_terminal_size();
        let cfg = Config::load();
        let scrollback_len = env_int("MATRISH_SCROLLBACK", 10000) as usize;
        let rain = (0..cols as usize)
            .map(|_| Column::new(rows as usize, true, &cfg))
            .collect();
        MatrixShell {
            cols,
            rows,
            rain,
            parser: vt100::Parser::new(rows, cols, scrollback_len),
            master_fd: -1,
            shell_pid: -1,
            running: true,
            altscreen: false,
            scroll_offset: 0,
            sel_start: None,
            sel_end: None,
            selecting: false,
            cfg,
        }
    }

    fn spawn_shell(&mut self) -> nix::Result<()> {
        let ws = Winsize { ws_row: self.rows, ws_col: self.cols, ws_xpixel: 0, ws_ypixel: 0 };
        let result = unsafe { forkpty(Some(&ws), None)? };
        match result {
            ForkptyResult::Child => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let c = std::ffi::CString::new(shell).expect("CString");
                unsafe { libc::execvp(c.as_ptr(), [c.as_ptr(), std::ptr::null()].as_ptr()); }
                unsafe { libc::_exit(1); }
            }
            ForkptyResult::Parent { child, master } => {
                self.shell_pid = child.as_raw();
                self.master_fd = {
                    use std::os::unix::io::IntoRawFd;
                    master.into_raw_fd()
                };
                set_winsize(self.master_fd, self.rows, self.cols);
            }
        }
        Ok(())
    }

    fn on_sigwinch(&mut self) {
        let (cols, rows) = get_terminal_size();
        self.cols = cols;
        self.rows = rows;
        self.scroll_offset = 0;
        self.parser.set_size(rows, cols);
        // Sync rain columns
        while self.rain.len() < cols as usize {
            self.rain.push(Column::new(rows as usize, false, &self.cfg));
        }
        self.rain.truncate(cols as usize);
        for col in &mut self.rain {
            col.height = rows as usize;
        }
        if self.master_fd >= 0 {
            set_winsize(self.master_fd, rows, cols);
        }
        if self.shell_pid > 0 {
            unsafe { libc::kill(self.shell_pid, libc::SIGWINCH); }
        }
    }

    fn enable_mouse(&self) {
        let _ = std::io::stdout().write_all(b"\x1b[?1002h\x1b[?1006h");
        let _ = std::io::stdout().flush();
    }

    fn disable_mouse(&self) {
        let _ = std::io::stdout().write_all(b"\x1b[?1006l\x1b[?1002l");
        let _ = std::io::stdout().flush();
    }

    fn sel_ordered(&self) -> Option<((usize, usize), (usize, usize))> {
        let a = self.sel_start?;
        let b = self.sel_end?;
        if a <= b { Some((a, b)) } else { Some((b, a)) }
    }

    /// Max scrollback rows currently stored.
    fn scrollback_max(&mut self) -> usize {
        // Probe by setting offset to usize::MAX (clamped internally), then read back.
        self.parser.set_scrollback(usize::MAX);
        let max = self.parser.screen().scrollback();
        self.parser.set_scrollback(self.scroll_offset);
        max
    }

    fn copy_selection(&mut self) {
        let Some(((r0, c0), (r1, c1))) = self.sel_ordered() else { return };
        // Temporarily set scrollback offset so screen.cell() returns the right rows
        self.parser.set_scrollback(self.scroll_offset);

        let mut lines: Vec<String> = Vec::new();
        for r in r0..=r1 {
            let start_c = if r == r0 { c0 } else { 0 };
            let end_c   = if r == r1 { c1 } else { self.cols as usize - 1 };
            let mut row_str = String::new();
            for c in start_c..=end_c {
                let s = self.parser.screen()
                    .cell(r as u16, c as u16)
                    .map(|cell| cell.contents())
                    .filter(|s| !s.is_empty() && s != "\x00")
                    .unwrap_or_else(|| " ".to_string());
                row_str.push_str(&s);
            }
            lines.push(row_str.trim_end().to_string());
        }
        let text = lines.join("\n");
        if !text.trim().is_empty() {
            clipboard_osc52(&text);
            clipboard_subprocess(&text);
        }
    }

    fn filter_input(&mut self, data: &[u8]) -> Vec<u8> {
        let (events, remaining) = parse_mouse_events(data);
        for ev in events {
            match (ev.btn, ev.press) {
                (64, _) => {
                    let max = self.scrollback_max();
                    let new = (self.scroll_offset + self.cfg.scroll_lines).min(max);
                    self.scroll_offset = new;
                    self.parser.set_scrollback(new);
                }
                (65, _) => {
                    let new = self.scroll_offset.saturating_sub(self.cfg.scroll_lines);
                    self.scroll_offset = new;
                    self.parser.set_scrollback(new);
                }
                (0, true) => {
                    self.sel_start = Some((ev.row, ev.col));
                    self.sel_end   = Some((ev.row, ev.col));
                    self.selecting = true;
                }
                (32, true) if self.selecting => {
                    self.sel_end = Some((ev.row, ev.col));
                }
                (0, false) if self.selecting => {
                    self.sel_end   = Some((ev.row, ev.col));
                    self.selecting = false;
                    self.copy_selection();
                    self.sel_start = None;
                    self.sel_end   = None;
                }
                _ => {}
            }
        }
        remaining
    }

    fn render(&mut self, dt: f64, out: &mut Vec<u8>) {
        for col in &mut self.rain {
            col.tick(dt, &self.cfg);
        }

        let scrolled = self.scroll_offset > 0;

        out.extend_from_slice(b"\x1b[?2026h"); // begin sync update
        out.extend_from_slice(b"\x1b[H");

        let screen = self.parser.screen();

        // Pre-scan: find shell-owned cells and build 1-cell halo
        let mut owned: HashSet<(u16, u16)> = HashSet::new();
        for r in 0..self.rows {
            for c in 0..self.cols {
                if screen.cell(r, c).map(shell_owns_cell).unwrap_or(false) {
                    owned.insert((r, c));
                }
            }
        }
        if !scrolled {
            let (cy, cx) = screen.cursor_position();
            owned.insert((cy, cx));
        }

        let mut halo: HashSet<(u16, u16)> = HashSet::new();
        for &(r, c) in &owned {
            for dr in -1i32..=1 {
                for dc in -1i32..=1 {
                    if dr == 0 && dc == 0 { continue; }
                    let nr = r as i32 + dr;
                    let nc = c as i32 + dc;
                    if nr >= 0 && nr < self.rows as i32 && nc >= 0 && nc < self.cols as i32 {
                        let p = (nr as u16, nc as u16);
                        if !owned.contains(&p) { halo.insert(p); }
                    }
                }
            }
        }

        let sel_span = if self.selecting || self.sel_start.is_some() {
            self.sel_ordered()
        } else {
            None
        };
        let (sel_r0, sel_c0, sel_r1, sel_c1) = match sel_span {
            Some(((r0, c0), (r1, c1))) => (r0 as i32, c0 as i32, r1 as i32, c1 as i32),
            None => (-1, -1, -1, -1),
        };

        let mut prev_esc = String::new();

        for r in 0..self.rows {
            let mut skip_next = false;
            for c in 0..self.cols {
                if skip_next { skip_next = false; continue; }

                let cell_opt = screen.cell(r, c);

                // Skip right-half of wide chars (vt100 marks them as wide_continuation)
                if cell_opt.map(|cell| cell.is_wide_continuation()).unwrap_or(false) {
                    continue;
                }

                let in_sel = if sel_r0 >= 0 {
                    let row = r as i32;
                    let col = c as i32;
                    if sel_r0 == sel_r1 {
                        row == sel_r0 && col >= sel_c0 && col <= sel_c1
                    } else if row == sel_r0 {
                        col >= sel_c0
                    } else if row == sel_r1 {
                        col <= sel_c1
                    } else {
                        row > sel_r0 && row < sel_r1
                    }
                } else {
                    false
                };

                let (glyph, cell_esc): (String, String) =
                    if let Some(cell) = cell_opt.filter(|cell| shell_owns_cell(cell)) {
                        let esc = format!(
                            "\x1b[0m{}{}{}",
                            color_esc(cell.fgcolor(), true),
                            color_esc(cell.bgcolor(), false),
                            attr_esc(cell),
                        );
                        let contents = cell.contents();
                        let wide = cell.is_wide();
                        if wide {
                            if c + 1 >= self.cols {
                                (" ".to_string(), esc) // clip at right margin
                            } else {
                                skip_next = true;
                                (contents, esc)
                            }
                        } else {
                            (contents, esc)
                        }
                    } else if halo.contains(&(r, c)) {
                        (" ".to_string(), "\x1b[0m".to_string())
                    } else {
                        match self.rain.get(c as usize).and_then(|col| col.cell_at(r as usize)) {
                            Some((ch, color)) => (ch.to_string(), color.to_string()),
                            None              => (" ".to_string(), "\x1b[0m".to_string()),
                        }
                    };

                let cell_esc = if in_sel { format!("{cell_esc}\x1b[7m") } else { cell_esc };

                if cell_esc != prev_esc {
                    out.extend_from_slice(cell_esc.as_bytes());
                    prev_esc = cell_esc;
                }
                out.extend_from_slice(glyph.as_bytes());
            }
            if r < self.rows - 1 {
                out.extend_from_slice(b"\r\n");
            }
        }

        if scrolled {
            let indicator = format!(" \u{2191}{} ", self.scroll_offset);
            let ind_col   = (self.cols as usize).saturating_sub(indicator.len()).max(1);
            let s = format!("\x1b[{};{}H\x1b[0;7m{indicator}\x1b[0m", self.rows, ind_col);
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\x1b[?25l");
        } else {
            let (cy, cx) = screen.cursor_position();
            let s = format!("\x1b[0m\x1b[{};{}H\x1b[?25h", cy + 1, cx + 1);
            out.extend_from_slice(s.as_bytes());
        }

        out.extend_from_slice(b"\x1b[?2026l");
    }

    // ── Event loop ────────────────────────────────────────────────────────────

    fn run(&mut self) {
        let stdin_fd = libc::STDIN_FILENO;

        if unsafe { libc::isatty(stdin_fd) } == 0 {
            eprintln!("matri: must be run in an interactive terminal");
            return;
        }

        let saved_termios = termios::tcgetattr(unsafe { bfd(stdin_fd) })
            .expect("tcgetattr");
        let mut raw = saved_termios.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(unsafe { bfd(stdin_fd) }, SetArg::TCSANOW, &raw)
            .expect("tcsetattr raw");

        let sa = SigAction::new(
            SigHandler::Handler(sigwinch_handler),
            SaFlags::empty(),
            SigSet::empty(),
        );
        unsafe { sigaction(Signal::SIGWINCH, &sa).expect("sigaction"); }

        self.enable_mouse();
        self.spawn_shell().expect("spawn_shell");

        const FPS: u64 = 60;
        let frame_dur = Duration::from_nanos(1_000_000_000 / FPS);
        let mut last_render = Instant::now();
        let mut render_buf: Vec<u8> = Vec::with_capacity(64 * 1024);

        'main: loop {
            if SIGWINCH_RECEIVED.swap(false, Ordering::Relaxed) {
                self.on_sigwinch();
            }

            let elapsed = last_render.elapsed();
            let wait    = if elapsed >= frame_dur { Duration::ZERO } else { frame_dur - elapsed };
            let mut tv  = TimeVal::new(wait.as_secs() as i64, wait.subsec_micros() as i64);

            let mut rfds = FdSet::new();
            unsafe {
                rfds.insert(bfd(stdin_fd));
                rfds.insert(bfd(self.master_fd));
            }
            let nfds = self.master_fd.max(stdin_fd) + 1;

            let interrupted = match select(nfds, Some(&mut rfds), None, None, Some(&mut tv)) {
                Ok(_)                              => false,
                Err(nix::errno::Errno::EINTR)      => true,
                Err(_)                             => break,
            };

            // stdin → shell
            if !interrupted && unsafe { rfds.contains(bfd(stdin_fd)) } {
                let mut buf = [0u8; 256];
                match unsafe { libc::read(stdin_fd, buf.as_mut_ptr() as *mut _, buf.len()) } {
                    n if n > 0 => {
                        let data = self.filter_input(&buf[..n as usize]);
                        if !data.is_empty() {
                            self.scroll_offset = 0;
                            self.parser.set_scrollback(0);
                            self.sel_start = None;
                            self.sel_end   = None;
                            unsafe { libc::write(self.master_fd, data.as_ptr() as *const _, data.len()); }
                        }
                    }
                    _ => break 'main,
                }
            }

            // shell → parser
            if !interrupted && self.master_fd >= 0 && unsafe { rfds.contains(bfd(self.master_fd)) } {
                let mut buf = [0u8; 4096];
                match unsafe { libc::read(self.master_fd, buf.as_mut_ptr() as *mut _, buf.len()) } {
                    n if n > 0 => {
                        let chunk = &buf[..n as usize];
                        let entering = memmem(chunk, b"\x1b[?1049h") || memmem(chunk, b"\x1b[?47h");
                        let exiting  = memmem(chunk, b"\x1b[?1049l") || memmem(chunk, b"\x1b[?47l");
                        if entering { self.altscreen = true; }
                        if self.altscreen {
                            let _ = std::io::stdout().write_all(chunk);
                            let _ = std::io::stdout().flush();
                            if exiting {
                                self.altscreen    = false;
                                self.scroll_offset = 0;
                                self.parser.set_scrollback(0);
                                self.enable_mouse();
                            }
                        } else {
                            self.parser.process(chunk);
                        }
                    }
                    _ => { break 'main; }
                }
            }

            // Render frame
            if !self.altscreen {
                let now     = Instant::now();
                let elapsed = now.duration_since(last_render);
                if elapsed >= frame_dur {
                    render_buf.clear();
                    self.render(elapsed.as_secs_f64(), &mut render_buf);
                    let _ = std::io::stdout().write_all(&render_buf);
                    let _ = std::io::stdout().flush();
                    last_render = now;
                }
            }

            if !self.running { break; }
        }

        // Cleanup
        self.disable_mouse();
        let _ = termios::tcsetattr(
            unsafe { bfd(stdin_fd) }, SetArg::TCSADRAIN, &saved_termios,
        );
        let _ = std::io::stdout().write_all(b"\x1b[0m\x1b[?25h\r\n");
        let _ = std::io::stdout().flush();

        if self.shell_pid > 0 {
            unsafe { libc::waitpid(self.shell_pid, std::ptr::null_mut(), 0); }
        }
        if self.master_fd >= 0 {
            unsafe { libc::close(self.master_fd); }
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn memmem(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    if std::env::var("MATRI_SH").as_deref() == Ok("true") {
        std::process::exit(1);
    }
    // SAFETY: single-threaded at this point
    unsafe { std::env::set_var("MATRI_SH", "true"); }

    MatrixShell::new().run();
}
