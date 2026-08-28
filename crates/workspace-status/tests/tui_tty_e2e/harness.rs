//! Spawn the real `workspace-status` binary on a PTY.
//!
//! The child runs the live event loop (`tty::poll_event` / `tty::read_event`
//! → crossterm `event::read`). Keys and xterm SGR mouse reports are written
//! as bytes on the PTY master — the same path a terminal uses. This is not
//! `HeadlessTui` and does not construct crossterm `Event` values in memory.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use super::common::hscroll::{
    is_clipped, is_panned_to_tail, TREE_HSCROLL_PREFIX, TREE_HSCROLL_TAIL,
};
use super::seed::git_env;

pub const COLS: u16 = 140;
pub const ROWS: u16 = 32;

/// xterm SGR button for wheel right (trackpad hscroll).
pub const SGR_WHEEL_RIGHT: u8 = 67;
/// Wheel right with the 1003 motion bit (`67 | 32`). crossterm 0.28 drops this.
pub const SGR_WHEEL_RIGHT_MOTION: u8 = 67 | 32;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

pub struct PtySession {
    child: Box<dyn portable_pty::Child + Send>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    root: PathBuf,
    cols: u16,
    rows: u16,
}

impl PtySession {
    pub fn open(workspace: &Path) -> Self {
        Self::open_with_env(workspace, &[])
    }

    pub fn open_size(workspace: &Path, cols: u16, rows: u16) -> Self {
        Self::open_size_with_env(workspace, cols, rows, &[])
    }

    /// Spawn the binary with extra child env after the watch-off defaults.
    ///
    /// Existing tests keep `WS_STATUS_WATCH_MS=0` / `WS_STATUS_FETCH_MS=0`
    /// unless an override is passed here.
    pub fn open_with_env(workspace: &Path, extra_env: &[(&str, &str)]) -> Self {
        Self::open_size_with_env(workspace, COLS, ROWS, extra_env)
    }

    pub fn open_size_with_env(
        workspace: &Path,
        cols: u16,
        rows: u16,
        extra_env: &[(&str, &str)],
    ) -> Self {
        Self::spawn(workspace, cols, rows, extra_env, None, true)
    }

    /// Spawn without waiting for the TUI. Used when the GitHub Release
    /// prompt must paint on the primary screen first.
    pub fn open_pending(
        workspace: &Path,
        extra_env: &[(&str, &str)],
        last_check_unix: u64,
    ) -> Self {
        Self::spawn(
            workspace,
            COLS,
            ROWS,
            extra_env,
            Some(last_check_unix),
            false,
        )
    }

    fn spawn(
        workspace: &Path,
        cols: u16,
        rows: u16,
        extra_env: &[(&str, &str)],
        last_check_unix: Option<u64>,
        wait_ready: bool,
    ) -> Self {
        assert!(
            workspace.is_dir(),
            "workspace must exist: {}",
            workspace.display()
        );
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let bin = env!("CARGO_BIN_EXE_workspace-status");
        let mut cmd = CommandBuilder::new(bin);
        cmd.cwd(workspace);
        for (k, v) in std::env::vars() {
            if matches!(
                k.as_str(),
                "NO_COLOR" | "FORCE_COLOR" | "WS_STATUS_GLYPHS" | "CLICOLOR_FORCE"
            ) {
                continue;
            }
            cmd.env(k, v);
        }
        // CommandBuilder starts with the parent env. Skipping the copy
        // above does not drop these; remove them so the TTY paints colour.
        cmd.env_remove("NO_COLOR");
        cmd.env_remove("FORCE_COLOR");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("WS_STATUS_GLYPHS", "ascii");
        cmd.env("WS_STATUS_WATCH_MS", "0");
        cmd.env("WS_STATUS_FETCH_MS", "0");
        cmd.env("LANG", "C.UTF-8");
        cmd.env("LC_ALL", "C.UTF-8");
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        for (k, v) in extra_env {
            cmd.env(*k, *v);
        }

        let state_home = workspace.join(".e2e-state");
        fs::create_dir_all(&state_home).unwrap();
        cmd.env("XDG_STATE_HOME", &state_home);
        cmd.env(
            "WS_STATUS_VIEWED_STORE",
            state_home.join("viewed-files.json"),
        );
        let update_store = state_home.join("update-check.json");
        write_update_check(&update_store, last_check_unix);
        cmd.env("WS_STATUS_UPDATE_CHECK_STORE", &update_store);

        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("spawn workspace-status");
        let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
        let writer = pair.master.take_writer().expect("pty writer");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let parser_thread = Arc::clone(&parser);
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        parser_thread.lock().unwrap().process(&buf[..n]);
                    }
                    Err(_) => break,
                }
            }
        });

        let session = Self {
            child,
            writer,
            master: pair.master,
            parser,
            root: workspace
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| workspace.to_path_buf()),
            cols,
            rows,
        };
        if wait_ready {
            session.wait_ready();
        }
        session
    }

    /// Wait until the TUI has painted the tree (after the update prompt).
    pub fn wait_ready(&self) {
        self.wait_contains_any(&[" tree", "Flat paths", "app", "README"], DEFAULT_TIMEOUT);
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write pty");
        self.writer.flush().expect("flush pty");
    }

    pub fn key(&mut self, c: char) {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        self.send_bytes(s.as_bytes());
    }

    pub fn keys(&mut self, s: &str) {
        for c in s.chars() {
            self.key(c);
        }
    }

    /// Kitty CSI-u one event (`CSI code ; modifier : kind u`).
    ///
    /// The live loop pushes `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus
    /// `REPORT_EVENT_TYPES`, so Shift+letter is this encoding — not a raw
    /// UTF-8 byte. Kind 1 is press, 3 is release.
    pub fn csi_u(&mut self, codepoint: u32, modifier: u8, kind: u8) {
        let seq = format!("\x1b[{codepoint};{modifier}:{kind}u");
        self.send_bytes(seq.as_bytes());
    }

    /// Shift+letter the way `event::read` sees it with keyboard enhancement.
    ///
    /// Terminals report the unshifted codepoint with modifier 2 (Shift) and
    /// a press then a release. A raw `'O'` byte is a different path and does
    /// not catch that remap.
    pub fn shift_letter(&mut self, letter: char) {
        let lower = letter.to_ascii_lowercase();
        let codepoint = u32::from(lower);
        self.csi_u(codepoint, 2, 1);
        self.csi_u(codepoint, 2, 3);
    }

    /// Type `text` as Shift+letter CSI-u for A–Z, raw bytes otherwise.
    pub fn shift_keys(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_ascii_alphabetic() {
                self.shift_letter(c);
            } else {
                self.key(c);
            }
        }
    }

    /// Unshifted letter press via CSI-u (kind 1). Traditional bytes stay
    /// [`Self::key`] for printable keys that do not need Repeat.
    pub fn letter_press(&mut self, letter: char) {
        let codepoint = u32::from(letter.to_ascii_lowercase());
        self.csi_u(codepoint, 1, 1);
    }

    /// Held-key Repeat (`CSI code ; 1 : 2 u`). Must fail if Repeat is ignored.
    pub fn letter_repeat(&mut self, letter: char) {
        let codepoint = u32::from(letter.to_ascii_lowercase());
        self.csi_u(codepoint, 1, 2);
    }

    /// `gg` chord: two `g` bytes inside the 400ms window.
    pub fn gg(&mut self) {
        self.key('g');
        self.key('g');
    }

    /// `zz` chord: two `z` bytes inside the 400ms window.
    ///
    /// First `z` toggles the focused row. The second is `toggleSubtree`.
    pub fn zz(&mut self) {
        self.key('z');
        self.key('z');
    }

    /// Home as CSI `1 ; 1 : 1 ~` (press) then `: 3` (release).
    ///
    /// The live loop requested event types. `event::read` maps CSI `1~` to
    /// `KeyCode::Home`. `CSI 1 u` is not Home.
    pub fn home(&mut self) {
        self.send_bytes(b"\x1b[1;1:1~");
        self.send_bytes(b"\x1b[1;1:3~");
    }

    /// End as CSI `4 ; 1 : 1 ~` (press) then `: 3` (release).
    ///
    /// Same event-type encoding as [`Self::home`]. `CSI 4 u` is not End.
    pub fn end(&mut self) {
        self.send_bytes(b"\x1b[4;1:1~");
        self.send_bytes(b"\x1b[4;1:3~");
    }

    pub fn enter(&mut self) {
        self.send_bytes(b"\r");
    }

    pub fn esc(&mut self) {
        // CSI-u Escape. A lone `\x1b` is the CSI prefix and is easy to lose
        // against the TUI's keyboard-enhancement flags.
        self.send_bytes(b"\x1b[27u");
    }

    /// xterm CSI PageUp (`ESC [5~`). `event::read` maps this to `KeyCode::PageUp`.
    pub fn page_up(&mut self) {
        self.send_bytes(b"\x1b[5~");
    }

    /// xterm CSI PageDown (`ESC [6~`). `event::read` maps this to `KeyCode::PageDown`.
    pub fn page_down(&mut self) {
        self.send_bytes(b"\x1b[6~");
    }

    pub fn tab(&mut self) {
        self.send_bytes(b"\t");
    }

    pub fn ctrl(&mut self, c: char) {
        let b = (c.to_ascii_lowercase() as u8) & 0x1f;
        self.send_bytes(&[b]);
    }

    /// Ctrl+letter via CSI-u (`CSI code ; 5 : 1 u` press, `: 3` release).
    ///
    /// Modifier 5 is Control. The live loop requested
    /// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event types. A C0 byte
    /// (`\x15` / `\x04`) is a different path. Ctrl-d as `\x04` is also EOT.
    pub fn ctrl_letter(&mut self, letter: char) {
        let codepoint = u32::from(letter.to_ascii_lowercase());
        self.csi_u(codepoint, 5, 1);
        self.csi_u(codepoint, 5, 3);
    }

    pub fn search(&mut self, query: &str) {
        self.key('/');
        self.keys(query);
        self.enter();
    }

    /// Encode one xterm SGR mouse report (`CSI < Cb ; Cx ; Cy M`).
    ///
    /// `col` / `row` are 0-based cells, matching crossterm. The bytes are
    /// 1-based, which is what a TTY sends for trackpad hscroll.
    pub fn sgr_mouse(&mut self, button: u8, col: u16, row: u16) {
        let seq = format!(
            "\x1b[<{button};{};{}M",
            col.saturating_add(1),
            row.saturating_add(1)
        );
        self.send_bytes(seq.as_bytes());
    }

    /// Left press + release. Setup only (focus a short tree row before
    /// hscroll). Not a click-coverage claim: a no-op click still continues.
    pub fn sgr_click(&mut self, col: u16, row: u16) {
        self.sgr_mouse(0, col, row);
        let seq = format!(
            "\x1b[<0;{};{}m",
            col.saturating_add(1),
            row.saturating_add(1)
        );
        self.send_bytes(seq.as_bytes());
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize pty");
        self.parser.lock().unwrap().set_size(rows, cols);
        self.cols = cols;
        self.rows = rows;
    }

    pub fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    /// Fingerprint of every cell fg/bg. Theme cycle must change it.
    ///
    /// `screen()` is glyphs only. A colour-only `T` paint can keep the same
    /// text and still fail this.
    pub fn color_fingerprint(&self) -> String {
        let parser = self.parser.lock().unwrap();
        let screen = parser.screen();
        let mut out = String::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                out.push_str(&format!("{:?}/{:?};", cell.fgcolor(), cell.bgcolor()));
            }
        }
        out
    }

    /// True when any cell uses this 24-bit colour as fg or bg.
    pub fn has_rgb(&self, r: u8, g: u8, b: u8) -> bool {
        let parser = self.parser.lock().unwrap();
        let screen = parser.screen();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if color_is_rgb(cell.fgcolor(), r, g, b) || color_is_rgb(cell.bgcolor(), r, g, b) {
                    return true;
                }
            }
        }
        false
    }

    /// Wait until [`Self::has_rgb`] is true.
    pub fn wait_has_rgb(&self, r: u8, g: u8, b: u8, timeout: Duration) {
        self.wait_pred(
            |_| self.has_rgb(r, g, b),
            &format!("cell rgb({r},{g},{b})"),
            timeout,
        );
    }

    pub fn wait_contains(&self, needle: &str, timeout: Duration) {
        self.wait_pred(
            |screen| screen.contains(needle),
            &format!("screen contains `{needle}`"),
            timeout,
        );
    }

    /// Wait for `needle` while `tick` runs each poll (live input, not idle).
    pub fn wait_contains_while(
        &mut self,
        needle: &str,
        timeout: Duration,
        mut tick: impl FnMut(&mut Self),
    ) {
        let start = Instant::now();
        loop {
            let screen = self.screen();
            if screen.contains(needle) {
                return;
            }
            if start.elapsed() >= timeout {
                panic!("timeout waiting for screen contains `{needle}`:\n{screen}");
            }
            tick(self);
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_absent(&self, needle: &str, timeout: Duration) {
        self.wait_pred(
            |screen| !screen.contains(needle),
            &format!("screen does not contain `{needle}`"),
            timeout,
        );
    }

    pub fn wait_contains_any(&self, needles: &[&str], timeout: Duration) {
        self.wait_pred(
            |screen| needles.iter().any(|n| screen.contains(n)),
            &format!("screen contains one of {needles:?}"),
            timeout,
        );
    }

    pub fn wait_pred(&self, pred: impl Fn(&str) -> bool, what: &str, timeout: Duration) {
        let start = Instant::now();
        loop {
            let screen = self.screen();
            if pred(&screen) {
                return;
            }
            if start.elapsed() >= timeout {
                panic!("timeout waiting for {what}:\n{screen}");
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Wait until the left tree is clipped and a wheel target row exists.
    ///
    /// Returns the row from that same frame. A later paint must not pass a
    /// prefix check then lose the row on a bare `expect`.
    pub fn wait_clipped_long_path_row(&self, timeout: Duration) -> u16 {
        let start = Instant::now();
        loop {
            let screen = self.screen();
            if let Some(row) = clipped_long_path_row(&screen) {
                return row;
            }
            if start.elapsed() >= timeout {
                panic!(
                    "timeout waiting for clipped long path on a tree row (no {}):\n{screen}",
                    TREE_HSCROLL_TAIL
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn wait_ms(&self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }

    /// Wait until the child process exits (`q` / second Ctrl+C).
    pub fn wait_exit(&mut self, timeout: Duration) {
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {}
                Err(err) => panic!("wait child: {err}"),
            }
            if start.elapsed() >= timeout {
                panic!(
                    "timeout waiting for TUI process to exit; screen:\n{}",
                    self.screen()
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn write_fresh_update_check(path: &Path) {
    write_update_check(path, None);
}

/// `last_check_unix = None` writes "now" so the startup prompt is skipped.
pub(crate) fn write_update_check(path: &Path, last_check_unix: Option<u64>) {
    let unix = last_check_unix.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });
    let body = format!("{{\n  \"version\": 1,\n  \"lastCheckUnix\": {unix}\n}}\n");
    fs::write(path, body).unwrap();
}

fn color_is_rgb(color: vt100::Color, r: u8, g: u8, b: u8) -> bool {
    matches!(color, vt100::Color::Rgb(cr, cg, cb) if cr == r && cg == g && cb == b)
}

pub fn assert_contains(screen: &str, needle: &str) {
    assert!(
        screen.contains(needle),
        "expected `{needle}` in screen:\n{screen}"
    );
}

pub fn assert_absent(screen: &str, needle: &str) {
    assert!(
        !screen.contains(needle),
        "did not expect `{needle}` in screen:\n{screen}"
    );
}

/// Left list cells, excluding top/bottom chrome.
///
/// A search chip on the status row can contain [`TREE_HSCROLL_TAIL`] without
/// the tree having panned. Split on the pane join so the right pane is out.
pub fn left_tree(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    let start = usize::from(end > 1);
    lines[start..end]
        .iter()
        .map(|line| left_of_split(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn left_of_split(line: &str) -> String {
    for sep in ["││", "┐┌", "┘└"] {
        if let Some(idx) = line.find(sep) {
            return line[..idx].to_string();
        }
    }
    let n = (line.chars().count() * 2 / 5).max(12);
    line.chars().take(n).collect()
}

/// 0-based screen row of a left-tree cell that contains `needle`.
///
/// Uses the same rows as [`left_tree`]: skip the top chrome line and the
/// last two status rows so a search / hint chip cannot match.
pub fn tree_row_containing(screen: &str, needle: &str) -> Option<u16> {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    let start = usize::from(end > 1);
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        if left_of_split(line).contains(needle) {
            return Some(i as u16);
        }
    }
    None
}

/// Clipped long-path row on the same frame: prefix visible, tail not.
///
/// `None` if the prefix is missing, already panned, or there is no tree
/// row to aim the wheel at. Callers wait on this instead of a bare expect
/// after resize or click.
pub fn clipped_long_path_row(screen: &str) -> Option<u16> {
    let left = left_tree(screen);
    if is_clipped(&left) {
        tree_row_containing(screen, TREE_HSCROLL_PREFIX)
    } else {
        None
    }
}

pub fn assert_tree_clipped_long_path(screen: &str) {
    crate::common::hscroll::assert_clipped(&left_tree(screen));
}

pub fn tree_is_panned_to_tail(screen: &str) -> bool {
    is_panned_to_tail(&left_tree(screen))
}
