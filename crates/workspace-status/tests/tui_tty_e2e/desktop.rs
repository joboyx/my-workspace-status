//! xfce4-terminal (keys) and xterm (XTEST wheel) desktop path.
//!
//! GitHub-hosted runners have no physical trackpad. Keys go through
//! xfce4-terminal (VTE). Wheel right is an XTEST pointer event (`click 7`,
//! no `--window`) in **xterm**: VTE 0.76 does not report X11 buttons 6/7, so
//! xfce never encodes SGR 67. xterm does. Screen text is recovered from a
//! `script(1)` typescript on that TTY.
//!
//! Ignored in `cargo test --workspace`. The `tui-tty-desktop` GitHub Actions
//! job runs these tests under `scripts/with-desktop-session.sh`. Local Linux:
//! see docs/tui-tty-e2e.md.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::common::hscroll::TREE_HSCROLL_TAIL;
use super::harness::{clipped_long_path_row, write_fresh_update_check, COLS, ROWS};
use super::seed::git_env;

const OPENBOX_RC: &str = include_str!("../../../../scripts/openbox.xml");

#[derive(Clone, Copy, Debug)]
enum Emulator {
    Xfce,
    Xterm,
}

pub struct DesktopSession {
    term: Option<Child>,
    ws_pid: Option<u32>,
    typescript: PathBuf,
    wid: String,
    root: PathBuf,
    cols: u16,
    rows: u16,
}

impl DesktopSession {
    pub fn open(workspace: &Path) -> Self {
        Self::open_with(workspace, COLS, ROWS, Emulator::Xfce)
    }

    /// xterm encodes XTEST button 7 as SGR 67. VTE does not.
    pub fn open_xterm_size(workspace: &Path, cols: u16, rows: u16) -> Self {
        Self::open_with(workspace, cols, rows, Emulator::Xterm)
    }

    fn open_with(workspace: &Path, cols: u16, rows: u16, emulator: Emulator) -> Self {
        require_desktop_tools(emulator);
        let stage = workspace.join(".e2e-desktop");
        fs::create_dir_all(&stage).unwrap();
        ensure_openbox(&stage);
        let typescript = stage.join("typescript");
        let _ = fs::remove_file(&typescript);
        fs::write(&typescript, []).unwrap();

        let state_home = workspace.join(".e2e-state");
        fs::create_dir_all(&state_home).unwrap();
        let update_store = state_home.join("update-check.json");
        write_fresh_update_check(&update_store);

        let bin = env!("CARGO_BIN_EXE_workspace-status");
        let launcher = stage.join("run-tui.sh");
        let mut script = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
        script.push_str("unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE\n");
        script.push_str("export TERM=xterm-256color COLORTERM=truecolor\n");
        script.push_str("export LANG=C.UTF-8 LC_ALL=C.UTF-8\n");
        script.push_str("export WS_STATUS_GLYPHS=ascii\n");
        script.push_str("export WS_STATUS_WATCH_MS=0 WS_STATUS_FETCH_MS=0\n");
        script.push_str(&format!(
            "export XDG_STATE_HOME={}\n",
            sh_quote(&state_home)
        ));
        script.push_str(&format!(
            "export WS_STATUS_UPDATE_CHECK_STORE={}\n",
            sh_quote(&update_store)
        ));
        script.push_str(&format!(
            "export WS_STATUS_VIEWED_STORE={}\n",
            sh_quote(&state_home.join("viewed-files.json"))
        ));
        script.push_str(&format!(
            "export WS_STATUS_COMMENT_STORE={}\n",
            sh_quote(&state_home.join("comments.json"))
        ));
        for (k, v) in git_env() {
            script.push_str(&format!("export {k}={}\n", sh_quote_str(v)));
        }
        script.push_str(&format!("cd {}\n", sh_quote(workspace)));
        script.push_str(&format!(
            "exec script -q -f -c {} {}\n",
            sh_quote_str(&format!("exec {bin}")),
            sh_quote(&typescript)
        ));
        fs::write(&launcher, script).unwrap();
        let mut perms = fs::metadata(&launcher).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&launcher, perms).unwrap();

        let mut term = spawn_emulator(emulator, cols, rows, &launcher);
        let min_area = (u64::from(cols) * u64::from(rows) * 30).max(20_000);
        let wid = wait_window(&mut term, min_area);
        let ws_pid = wait_tui_pid(bin);
        let session = Self {
            term: Some(term),
            ws_pid,
            typescript,
            wid,
            root: workspace
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| workspace.to_path_buf()),
            cols,
            rows,
        };
        session.wait_contains_any(
            &[" tree", "Flat paths", "app", "README"],
            Duration::from_secs(20),
        );
        session.grab_input();
        session
    }

    fn grab_input(&self) {
        let _ = Command::new("xdotool")
            .args(["windowraise", &self.wid])
            .status();
        let _ = Command::new("xdotool")
            .args(["windowfocus", "--sync", &self.wid])
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("xdotool")
            .args(["windowactivate", "--sync", &self.wid])
            .stderr(Stdio::null())
            .status();
        // XTEST warp + click on the status row (not a fold chevron). A click
        // on the workspace header folds the tree and hides the long path.
        let (x, y) = self.cell_root_pixels(4, self.rows.saturating_sub(1));
        let _ = Command::new("xdotool")
            .args(["mousemove", "--sync", &x.to_string(), &y.to_string()])
            .status();
        let _ = Command::new("xdotool")
            .args(["click", "1"])
            .stderr(Stdio::null())
            .status();
        thread::sleep(Duration::from_millis(200));
    }

    fn focus(&self) {
        let _ = Command::new("xdotool")
            .args(["windowraise", &self.wid])
            .status();
        let _ = Command::new("xdotool")
            .args(["windowfocus", "--sync", &self.wid])
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("xdotool")
            .args(["windowactivate", "--sync", &self.wid])
            .stderr(Stdio::null())
            .status();
        thread::sleep(Duration::from_millis(40));
    }

    pub fn xdotool(&self, args: &[&str]) {
        self.focus();
        // No --window: VTE ignores synthetic XSendEvent keys. XTEST needs focus.
        let status = Command::new("xdotool")
            .args(["key", "--delay", "40"])
            .args(args)
            .status()
            .expect("xdotool key");
        assert!(status.success(), "xdotool key {args:?} failed");
        thread::sleep(Duration::from_millis(120));
    }

    pub fn key(&self, name: &str) {
        self.xdotool(&[name]);
    }

    pub fn type_text(&self, text: &str) {
        self.focus();
        let status = Command::new("xdotool")
            .args(["type", "--delay", "40", text])
            .status()
            .expect("xdotool type");
        assert!(status.success(), "xdotool type {text:?} failed");
        thread::sleep(Duration::from_millis(120));
    }

    /// Left button via XTEST (no `--window`). Setup only: focus a short
    /// tree row so hscroll pans the tree, not a long file-diff.
    pub fn click_cell(&self, col: u16, row: u16) {
        self.pointer_to_cell(col, row);
        let status = Command::new("xdotool")
            .args(["click", "1"])
            .status()
            .expect("xdotool click 1");
        assert!(status.success(), "xdotool click 1 (XTEST) failed");
        thread::sleep(Duration::from_millis(200));
    }

    /// XTEST wheel right. No `--window` on warp or click.
    ///
    /// `--window` uses XSendEvent. Real terminals ignore that, so a later
    /// `click 7` would fire at the real pointer instead of the tree cell.
    pub fn wheel_right_at_cell(&self, col: u16, row: u16, times: u32) {
        self.pointer_to_cell(col, row);
        for _ in 0..times {
            let status = Command::new("xdotool")
                .args(["click", "7"])
                .status()
                .expect("xdotool click 7");
            assert!(
                status.success(),
                "xdotool click 7 (XTEST wheel right) failed"
            );
            thread::sleep(Duration::from_millis(30));
        }
        thread::sleep(Duration::from_millis(200));
    }

    /// Warp the real X pointer (XTEST). No `--window`.
    fn pointer_to_cell(&self, col: u16, row: u16) {
        self.focus();
        let (x, y) = self.cell_root_pixels(col, row);
        let status = Command::new("xdotool")
            .args(["mousemove", "--sync", &x.to_string(), &y.to_string()])
            .status()
            .expect("xdotool mousemove");
        assert!(
            status.success(),
            "xdotool mousemove (XTEST) to {x},{y} failed"
        );
    }

    fn cell_root_pixels(&self, col: u16, row: u16) -> (i32, i32) {
        let (origin_x, origin_y, width, height) = self.window_geometry();
        if width == 0 || height == 0 {
            panic!("window size is zero");
        }
        let cols = f64::from(self.cols.max(1));
        let rows = f64::from(self.rows.max(1));
        let x = origin_x + ((f64::from(col) + 0.5) * (f64::from(width) / cols)).round() as i32;
        let y = origin_y + ((f64::from(row) + 0.5) * (f64::from(height) / rows)).round() as i32;
        (x.max(1), y.max(1))
    }

    fn window_geometry(&self) -> (i32, i32, i32, i32) {
        let out = Command::new("xdotool")
            .args(["getwindowgeometry", "--shell", &self.wid])
            .output()
            .expect("window geometry");
        let text = String::from_utf8_lossy(&out.stdout);
        let mut x = 0i32;
        let mut y = 0i32;
        let mut width = 0i32;
        let mut height = 0i32;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("X=") {
                x = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("Y=") {
                y = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("WIDTH=") {
                width = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("HEIGHT=") {
                height = v.parse().unwrap_or(0);
            }
        }
        (x, y, width, height)
    }

    pub fn screen(&self) -> String {
        let bytes = fs::read(&self.typescript).unwrap_or_default();
        let mut parser = vt100::Parser::new(self.rows, self.cols, 0);
        parser.process(&bytes);
        parser.screen().contents()
    }

    pub fn wait_contains(&self, needle: &str, timeout: Duration) {
        self.wait_pred(
            |screen| screen.contains(needle),
            &format!("screen contains `{needle}`"),
            timeout,
        );
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

    pub fn wait_ms(&self, ms: u64) {
        thread::sleep(Duration::from_millis(ms));
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
            thread::sleep(Duration::from_millis(40));
        }
    }
}

impl Drop for DesktopSession {
    fn drop(&mut self) {
        if let Some(pid) = self.ws_pid {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
        if let Some(mut term) = self.term.take() {
            let _ = term.kill();
            let _ = term.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn require_desktop_tools(emulator: Emulator) {
    assert!(
        std::env::var_os("DISPLAY").is_some(),
        "desktop TTY e2e needs DISPLAY (GitHub Actions tui-tty-desktop uses scripts/with-desktop-session.sh). See docs/tui-tty-e2e.md"
    );
    let term = match emulator {
        Emulator::Xfce => "xfce4-terminal",
        Emulator::Xterm => "xterm",
    };
    for bin in [term, "xdotool", "script"] {
        assert!(
            have(bin),
            "desktop TTY e2e needs `{bin}` on PATH. See docs/tui-tty-e2e.md"
        );
    }
}

fn spawn_emulator(emulator: Emulator, cols: u16, rows: u16, launcher: &Path) -> Child {
    let mut cmd = match emulator {
        Emulator::Xfce => {
            let mut cmd = Command::new("xfce4-terminal");
            cmd.args([
                "--disable-server",
                &format!("--geometry={cols}x{rows}+24+24"),
                "--hide-menubar",
                "--hide-toolbar",
                "--hide-scrollbar",
                "--hide-borders",
                "--dynamic-title-mode=none",
                "-T",
                "WSTTY",
                "-e",
            ]);
            cmd
        }
        Emulator::Xterm => {
            let mut cmd = Command::new("xterm");
            cmd.args([
                "-b",
                "0",
                "+sb",
                "-tn",
                "xterm-256color",
                "-geometry",
                &format!("{cols}x{rows}+24+24"),
                "-T",
                "WSTTY",
                "-e",
            ]);
            cmd
        }
    };
    cmd.arg(launcher)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{emulator:?} starts: {e}"))
}

fn have(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ensure_openbox(stage: &Path) {
    let rc = stage.join("openbox.xml");
    fs::write(&rc, OPENBOX_RC.as_bytes()).unwrap();
    if !have("openbox") {
        return;
    }
    let running = Command::new("pgrep")
        .args(["-x", "openbox"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if running {
        // Session Openbox (`scripts/with-desktop-session.sh`) should already
        // have been started with scripts/openbox.xml. Do not `--replace`:
        // that would steal a shared DISPLAY.
        return;
    }
    let _ = Command::new("openbox")
        .args(["--config-file"])
        .arg(&rc)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    thread::sleep(Duration::from_millis(400));
}

fn wait_window(term: &mut Child, min_area: u64) -> String {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if let Ok(Some(status)) = term.try_wait() {
            panic!("desktop terminal exited early: {status}");
        }
        if let Some(wid) = largest_terminal_window(min_area) {
            return wid;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("desktop terminal window never appeared");
}

fn largest_terminal_window(min_area: u64) -> Option<String> {
    let mut wids = Vec::new();
    for args in [
        ["search", "--name", "WSTTY"].as_slice(),
        ["search", "--class", "xfce4-terminal"].as_slice(),
        ["search", "--class", "XTerm"].as_slice(),
        ["search", "--class", "xterm"].as_slice(),
    ] {
        if let Ok(out) = Command::new("xdotool").args(args).output() {
            if out.status.success() {
                wids.extend(
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                );
            }
        }
    }
    let mut best: Option<(String, u64)> = None;
    for wid in wids {
        let out = match Command::new("xdotool")
            .args(["getwindowgeometry", "--shell", &wid])
            .output()
        {
            Ok(out) if out.status.success() => out,
            _ => continue,
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut width = 0u64;
        let mut height = 0u64;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("WIDTH=") {
                width = v.parse().unwrap_or(0);
            }
            if let Some(v) = line.strip_prefix("HEIGHT=") {
                height = v.parse().unwrap_or(0);
            }
        }
        let area = width.saturating_mul(height);
        if area >= min_area && best.as_ref().map_or(true, |(_, a)| area > *a) {
            best = Some((wid, area));
        }
    }
    best.map(|(wid, _)| wid)
}

fn wait_tui_pid(bin: &str) -> Option<u32> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(out) = Command::new("pgrep").args(["-f", bin]).output() {
            if out.status.success() {
                if let Some(pid) = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .rev()
                    .find_map(|l| l.trim().parse().ok())
                {
                    return Some(pid);
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    None
}

fn sh_quote(path: &Path) -> String {
    sh_quote_str(&path.display().to_string())
}

fn sh_quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
