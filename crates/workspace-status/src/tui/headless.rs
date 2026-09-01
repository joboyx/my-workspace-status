//! Headless ratatui session for cargo tests. Uses TestBackend. No TTY.
//!
//! Effects go through [`super::effect::Interpreter::interpret_sync`] (same
//! apply path as the live loop). Jobs run on this thread. TTY `$EDITOR`
//! (`Effect::EditFile`) is not run.

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::config::{load_workspace_status_config, WorkspaceStatusConfig};
use crate::snapshot::WorkspaceSnapshot;

use super::action::{Action, Effect};
use super::app::{collect_full_snapshot, TuiOpts};
use super::comments::CommentStore;
use super::effect::Interpreter;
use super::keys::event_to_action_with;
use super::render::draw;
use super::state::{AppState, FocusPane};
use super::tty::{
    decode_sgr_mouse, sgr_mouse_report, SGR_SHIFT_WHEEL_DOWN, SGR_WHEEL_RIGHT,
    SGR_WHEEL_RIGHT_MOTION,
};

/// Default TestBackend size. Wide enough for the tree + graph / diff split.
const WIDTH: u16 = 140;
const HEIGHT: u16 = 28;

/// Fixture-driven TUI session that paints on [`TestBackend`].
pub struct HeadlessTui {
    state: AppState,
    opts: TuiOpts,
    interp: Interpreter,
    width: u16,
    height: u16,
    quit: bool,
}

impl HeadlessTui {
    /// Open a session from a workspace directory. Hidden ignored stay out unless
    /// `show_ignored` is true (`-a`).
    ///
    /// Does not run the CLI GitHub Release check. That prompt exists only on a
    /// real `ws` / `workspace-status` TUI launch.
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
        let comment_path = viewed_path.with_file_name("comments.json");
        let opts = TuiOpts {
            cwd: cwd.clone(),
            snapshot: snapshot.clone(),
            config,
            start_fetch: false,
        };
        let mut state = AppState::with_viewed_path(cwd, snapshot, false, viewed_path);
        state.comment_path = comment_path;
        state.comment_store = CommentStore::new();
        let mut interp = Interpreter::new();
        interp.interpret_sync(&mut state, &opts, Effect::LoadRightPane, &Action::None);
        let mut session = Self {
            state,
            opts,
            interp,
            width: WIDTH,
            height: HEIGHT,
            quit: false,
        };
        let _ = session.frame();
        session
    }

    /// Point comment persist at `path`. Failed-write tests use an unwritable location.
    pub fn set_comment_store_path(&mut self, path: impl Into<PathBuf>) {
        self.state.comment_path = path.into();
    }

    /// Point viewed persist at `path`. Failed-write tests use an unwritable location.
    pub fn set_viewed_store_path(&mut self, path: impl Into<PathBuf>) {
        self.state.viewed_path = path.into();
    }

    /// Send one character through the real keymap.
    pub fn key(&mut self, c: char) {
        self.send(KeyCode::Char(c));
    }

    /// Send a terminal key-repeat of one character (hold).
    pub fn key_repeat(&mut self, c: char) {
        self.send_key_kind(KeyCode::Char(c), KeyModifiers::NONE, KeyEventKind::Repeat);
    }

    /// Send a keyboard-enhancement key-release of one character.
    ///
    /// The live loop requested `REPORT_EVENT_TYPES`. Release must not
    /// dispatch (it would clear the `gg` pending).
    pub fn key_release(&mut self, c: char) {
        self.send_key_kind(KeyCode::Char(c), KeyModifiers::NONE, KeyEventKind::Release);
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
        let action_for_load = action.clone();
        let effect = self.state.dispatch(action);
        self.interp
            .interpret_sync(&mut self.state, &self.opts, effect, &action_for_load);
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

    /// Send Backspace.
    pub fn backspace(&mut self) {
        self.send(KeyCode::Backspace);
    }

    /// Send Esc.
    pub fn esc(&mut self) {
        self.send(KeyCode::Esc);
    }

    /// Fire one live-watch poll (same as the TTY timer).
    pub fn watch_tick(&mut self) {
        if self.quit {
            return;
        }
        let effect = self.state.dispatch(Action::WatchTick);
        if matches!(effect, Effect::Quit) {
            self.quit = true;
            return;
        }
        self.interp
            .interpret_sync(&mut self.state, &self.opts, effect, &Action::WatchTick);
    }

    /// HEAD sha the graph was last loaded for, if any.
    pub fn graph_head(&self) -> Option<String> {
        self.state
            .graph_identity
            .as_ref()
            .map(|(_, head)| head.clone())
    }

    /// True when `id` is still inside the flash window.
    pub fn is_flashing(&self, id: &str) -> bool {
        self.state.flashes.contains_key(id)
    }

    /// Checkout `HEAD` from the current snapshot, if that repo is loaded.
    pub fn snapshot_head(&self, repo: &str) -> Option<String> {
        self.state
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .map(|row| row.head.clone())
    }

    /// Sync note from the current snapshot, if that repo is loaded.
    pub fn snapshot_sync_note(&self, repo: &str) -> Option<String> {
        self.state
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .map(|row| row.sync_note.clone())
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

    /// Left-button down through the real keymap (`Action::Click`).
    pub fn mouse_down(&mut self, col: u16, row: u16) {
        self.send_mouse(MouseEventKind::Down(MouseButton::Left), col, row);
    }

    /// Left-button drag through the real keymap (`Action::Drag`).
    pub fn mouse_drag(&mut self, col: u16, row: u16) {
        self.send_mouse(MouseEventKind::Drag(MouseButton::Left), col, row);
    }

    /// Left-button up through the real keymap (`Action::Release`).
    pub fn mouse_up(&mut self) {
        self.send_mouse(MouseEventKind::Up(MouseButton::Left), 0, 0);
    }

    /// Vertical wheel down through the real keymap.
    pub fn mouse_scroll_down(&mut self, col: u16, row: u16) {
        self.send_mouse(MouseEventKind::ScrollDown, col, row);
    }

    /// Horizontal wheel right through the real keymap.
    pub fn mouse_scroll_right(&mut self, col: u16, row: u16) {
        self.send_mouse(MouseEventKind::ScrollRight, col, row);
    }

    /// Shift+wheel down (common terminal encoding of trackpad hscroll).
    pub fn mouse_shift_scroll_down(&mut self, col: u16, row: u16) {
        self.send_mouse_mods(MouseEventKind::ScrollDown, col, row, KeyModifiers::SHIFT);
    }

    /// Trackpad hscroll as a TTY SGR wheel-right report (`CSI < 67 ; C ; R M`).
    ///
    /// `col` / `row` are 0-based cells. The bytes are 1-based, then decoded
    /// with [`super::tty::decode_sgr_mouse`] (same contract as `read_event`).
    pub fn mouse_sgr_scroll_right(&mut self, col: u16, row: u16) {
        self.send_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT, col, row));
    }

    /// Trackpad hscroll as a TTY SGR Shift+wheel-down report (`CSI < 69 ; C ; R M`).
    pub fn mouse_sgr_shift_wheel_down(&mut self, col: u16, row: u16) {
        self.send_sgr_mouse(&sgr_mouse_report(SGR_SHIFT_WHEEL_DOWN, col, row));
    }

    /// Dispatch one decoded SGR mouse report through the real keymap.
    ///
    /// Unknown reports are dropped, matching `read_event` / `event::read`
    /// skipping a crossterm parse error (the TTY never sees those bytes as
    /// an `Event`).
    pub fn send_sgr_mouse(&mut self, seq: &[u8]) {
        let Some(event) = decode_sgr_mouse(seq) else {
            return;
        };
        self.dispatch_event(event);
    }

    /// Trackpad hscroll as SGR wheel-right with the 1003 motion bit (`CSI < 99 ; C ; R M`).
    pub fn mouse_sgr_motion_scroll_right(&mut self, col: u16, row: u16) {
        self.send_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT_MOTION, col, row));
    }

    /// File-diff horizontal pan (`diff_col_offset`).
    pub fn diff_col_offset(&self) -> u16 {
        self.state.diff_col_offset
    }

    /// File-diff vertical skip (`diff_scroll`).
    pub fn diff_scroll(&self) -> u16 {
        self.state.diff_scroll
    }

    /// Focused file-diff row (section, hunk, or line).
    pub fn diff_cursor(&self) -> usize {
        self.state.diff_cursor
    }

    /// Diff horizontal scrollbar track from the last paint, if the bar is up.
    pub fn diff_hscrollbar_track(&self) -> Option<(u16, u16, u16)> {
        let y = self.state.layout.diff_hscrollbar_y?;
        let width = self.state.layout.diff_hscrollbar_width;
        if width == 0 {
            return None;
        }
        Some((self.state.layout.diff_hscrollbar_x, y, width))
    }

    /// 0-based diff vertical scrollbar column from the last paint.
    pub fn diff_scrollbar_col(&self) -> Option<u16> {
        self.state.layout.diff_scrollbar_x
    }

    /// Inner tree pane origin x from the last paint.
    pub fn tree_inner_x(&self) -> u16 {
        self.state.layout.tree_x
    }

    /// Inner tree pane origin y from the last paint.
    pub fn tree_inner_y(&self) -> u16 {
        self.state.layout.tree_y
    }

    /// Horizontal pan of the left list (`left_col_offset`).
    pub fn left_col_offset(&self) -> u16 {
        self.state.left_col_offset
    }

    /// 0-based first column of the right pane from the last paint.
    pub fn pane_right_x(&self) -> u16 {
        self.state.layout.right_x
    }

    /// Right-pane horizontal pan offset.
    pub fn right_col_offset(&self) -> u16 {
        self.state.right_col_offset
    }

    /// Graph horizontal scrollbar track (0-based x, y, width) from the last paint.
    pub fn graph_hscrollbar_track(&self) -> Option<(u16, u16, u16)> {
        let y = self.state.layout.graph_hscrollbar_y?;
        let width = self.state.layout.graph_hscrollbar_width;
        if width == 0 {
            return None;
        }
        Some((self.state.layout.graph_hscrollbar_x, y, width))
    }

    /// Graph list cursor (selectable `visible_rows` index).
    pub fn graph_cursor(&self) -> usize {
        self.state.graph_cursor
    }

    /// Commit-file list cursor.
    pub fn commit_files_cursor(&self) -> usize {
        self.state.commit_files_cursor()
    }

    /// Visible commit-file row count (tree or flat).
    pub fn commit_files_len(&self) -> usize {
        self.state.commit_file_rows().len()
    }

    /// Current graph list skip (`graph_scroll`).
    pub fn graph_scroll(&self) -> u16 {
        self.state.graph_scroll
    }

    /// 0-based graph scrollbar column from the last paint, if a graph list is up.
    pub fn graph_scrollbar_col(&self) -> Option<u16> {
        self.state.layout.graph_scrollbar_x
    }

    /// Graph scrollbar track (0-based first row, height) from the last paint.
    pub fn graph_scrollbar_track(&self) -> Option<(u16, u16)> {
        let height = self.state.layout.graph_scrollbar_height;
        if self.state.layout.graph_scrollbar_x.is_none() || height == 0 {
            return None;
        }
        Some((self.state.layout.graph_scrollbar_y, height))
    }

    fn send_mouse(&mut self, kind: MouseEventKind, col: u16, row: u16) {
        self.send_mouse_mods(kind, col, row, KeyModifiers::NONE);
    }

    fn send_mouse_mods(
        &mut self,
        kind: MouseEventKind,
        col: u16,
        row: u16,
        modifiers: KeyModifiers,
    ) {
        if self.quit {
            return;
        }
        let event = Event::Mouse(MouseEvent {
            kind,
            column: col,
            row,
            modifiers,
        });
        self.dispatch_event(event);
    }

    fn send(&mut self, code: KeyCode) {
        self.send_key(code, KeyModifiers::NONE);
    }

    fn send_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        self.send_key_kind(code, modifiers, KeyEventKind::Press);
    }

    fn send_key_kind(&mut self, code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) {
        if self.quit {
            return;
        }
        self.dispatch_event(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)));
    }

    fn dispatch_event(&mut self, event: Event) {
        if let Event::Key(key) = &event {
            if key.kind == KeyEventKind::Release {
                return;
            }
        }
        let action = event_to_action_with(
            &event,
            self.state.input_mode(),
            self.state.right_is_diff(),
            matches!(self.state.focus, FocusPane::Right),
            self.state.graph_stash_focused(),
            self.state.graph_commit_focused(),
            self.state.hl_folds(),
        );
        let action_for_load = action.clone();
        let effect = self.state.dispatch(action);
        if matches!(effect, Effect::Quit) {
            self.quit = true;
            return;
        }
        self.interp
            .interpret_sync(&mut self.state, &self.opts, effect, &action_for_load);
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
