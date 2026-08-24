//! Headless ratatui session for cargo tests. Uses TestBackend. No TTY.

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::config::{load_workspace_status_config, WorkspaceStatusConfig};
use crate::snapshot::WorkspaceSnapshot;

use super::action::Effect;
use super::app::{apply_headless_effect, collect_full_snapshot, TuiOpts};
use super::keys::event_to_action_with;
use super::render::draw;
use super::state::{AppState, FocusPane};

/// Default TestBackend size. Wide enough for the tree + graph / diff split.
const WIDTH: u16 = 140;
const HEIGHT: u16 = 28;

/// Fixture-driven TUI session that paints on [`TestBackend`].
pub struct HeadlessTui {
    state: AppState,
    opts: TuiOpts,
    width: u16,
    height: u16,
    quit: bool,
}

impl HeadlessTui {
    /// Open a session from a workspace directory. Hidden ignored stay out unless
    /// `show_ignored` is true (`-a`).
    pub fn open(cwd: impl Into<PathBuf>, show_ignored: bool) -> Self {
        let cwd = cwd.into();
        let config = load_workspace_status_config(&cwd)
            .unwrap_or_else(|_| WorkspaceStatusConfig::with_defaults());
        let snapshot = collect_full_snapshot(&cwd, &config, &[], show_ignored, false);
        Self::from_snapshot(cwd, snapshot, config)
    }

    fn from_snapshot(
        cwd: PathBuf,
        snapshot: WorkspaceSnapshot,
        config: WorkspaceStatusConfig,
    ) -> Self {
        let viewed_path = unique_viewed_path();
        let opts = TuiOpts {
            cwd: cwd.clone(),
            snapshot: snapshot.clone(),
            config,
            start_fetch: false,
        };
        let mut state = AppState::with_viewed_path(cwd, snapshot, false, viewed_path);
        apply_headless_effect(&mut state, super::action::Effect::LoadRightPane, &opts);
        let mut session = Self {
            state,
            opts,
            width: WIDTH,
            height: HEIGHT,
            quit: false,
        };
        let _ = session.frame();
        session
    }

    /// Send one character through the real keymap.
    pub fn key(&mut self, c: char) {
        self.send(KeyCode::Char(c));
    }

    /// Send Tab.
    pub fn tab(&mut self) {
        self.send(KeyCode::Tab);
    }

    /// Apply a crossterm `Resize` and relayout panes to `cols` × `rows`.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.width = cols.max(1);
        self.height = rows.max(1);
        let event = Event::Resize(self.width, self.height);
        let action = event_to_action_with(
            &event,
            self.state.input_mode(),
            self.state.right_is_diff(),
            matches!(self.state.focus, FocusPane::Right),
            self.state.graph_stash_focused(),
            self.state.graph_commit_focused(),
            self.state.hl_folds(),
        );
        let effect = self.state.dispatch(action);
        apply_headless_effect(&mut self.state, effect, &self.opts);
        let _ = self.frame();
        self.state.sync_graph_scroll();
    }

    /// Outer tree pane width from the last paint.
    pub fn pane_tree_width(&self) -> u16 {
        self.state.layout.outer_tree_width
    }

    /// Right-pane inner width from the last paint.
    pub fn pane_diff_width(&self) -> u16 {
        self.state.layout.diff_pane_width
    }

    /// Left-pane inner list height from the last paint.
    pub fn pane_tree_height(&self) -> u16 {
        self.state.layout.tree_height
    }

    /// Send Enter.
    pub fn enter(&mut self) {
        self.send(KeyCode::Enter);
    }

    /// Send Esc.
    pub fn esc(&mut self) {
        self.send(KeyCode::Esc);
    }

    /// Send Ctrl-C through the real keymap (double-press quit chord).
    pub fn ctrl_c(&mut self) {
        self.send_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    }

    /// True after `q` or a completed double Ctrl-C.
    pub fn did_quit(&self) -> bool {
        self.quit
    }

    /// Expire an armed Ctrl-C window as if the confirm interval elapsed.
    pub fn expire_ctrl_c(&mut self) {
        let now = self.state.ctrl_c_armed_until.unwrap_or_else(Instant::now);
        self.state.expire_ctrl_c_prompt(now);
        let _ = self.frame();
    }

    /// Type `/query` and Enter (tree search).
    pub fn search(&mut self, query: &str) {
        self.key('/');
        for c in query.chars() {
            self.key(c);
        }
        self.enter();
    }

    /// Paint and return the full screen as lines (trailing spaces trimmed).
    pub fn frame(&mut self) -> String {
        self.paint().join("\n")
    }

    /// Paint and return a fingerprint of cell colours. Theme cycle must change it.
    pub fn style_fingerprint(&mut self) -> String {
        let backend = TestBackend::new(self.width, self.height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| draw(frame, &mut self.state))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = &buffer[(x, y)];
                out.push_str(&format!("{:?}/{:?};", cell.fg, cell.bg));
            }
        }
        out
    }

    /// Current tree cursor label (empty if the list is empty).
    pub fn cursor_label(&self) -> String {
        self.state
            .rows
            .get(self.state.cursor)
            .map(|r| r.label.clone())
            .unwrap_or_default()
    }

    /// Current tree cursor id.
    pub fn cursor_id(&self) -> String {
        self.state
            .rows
            .get(self.state.cursor)
            .map(|r| r.id.clone())
            .unwrap_or_default()
    }

    /// Whether the right pane is a file / commit diff.
    pub fn right_is_diff(&self) -> bool {
        self.state.right_is_diff() || self.state.drill.is_diff()
    }

    /// Whether the right pane is the commit file list.
    pub fn right_is_files(&self) -> bool {
        self.state.drill.is_files()
    }

    /// Whether the right pane is the graph.
    pub fn right_is_graph(&self) -> bool {
        self.state.drill.is_graph() && !self.state.right_is_diff()
    }

    /// Whether keyboard focus is on the right pane.
    pub fn focus_is_right(&self) -> bool {
        matches!(self.state.focus, FocusPane::Right)
    }

    /// Depth 2 left pane is the commit-file list.
    pub fn left_is_files(&self) -> bool {
        self.state.drill.is_diff()
    }

    /// Depth 1 left pane is the graph list.
    pub fn left_is_graph(&self) -> bool {
        self.state.drill.is_files()
    }

    /// Send Shift+Left (pan the focused pane, including the tree).
    pub fn shift_left(&mut self) {
        self.send_key(KeyCode::Left, KeyModifiers::SHIFT);
    }

    /// Send Shift+Right (pan the focused pane, including the tree).
    pub fn shift_right(&mut self) {
        self.send_key(KeyCode::Right, KeyModifiers::SHIFT);
    }

    fn send(&mut self, code: KeyCode) {
        self.send_key(code, KeyModifiers::NONE);
    }

    fn send_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.quit {
            return;
        }
        let event = Event::Key(KeyEvent::new(code, modifiers));
        let action = event_to_action_with(
            &event,
            self.state.input_mode(),
            self.state.right_is_diff(),
            matches!(self.state.focus, FocusPane::Right),
            self.state.graph_stash_focused(),
            self.state.graph_commit_focused(),
            self.state.hl_folds(),
        );
        let effect = self.state.dispatch(action);
        if matches!(effect, Effect::Quit) {
            self.quit = true;
            return;
        }
        apply_headless_effect(&mut self.state, effect, &self.opts);
    }

    fn paint(&mut self) -> Vec<String> {
        let backend = TestBackend::new(self.width, self.height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| draw(frame, &mut self.state))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..self.height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..self.width {
                    let symbol = buffer[(x, y)].symbol();
                    if !symbol.is_empty() {
                        line.push_str(symbol);
                    }
                }
                line.trim_end().to_string()
            })
            .collect()
    }
}

fn unique_viewed_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "ws-tui-e2e-viewed-{}-{nanos}.json",
        std::process::id()
    ))
}
