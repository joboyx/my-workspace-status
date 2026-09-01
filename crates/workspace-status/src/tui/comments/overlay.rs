//! Comment overlay types shown while typing or after markdown copy.
//!
//! Edit state is a [`tui_textarea::TextArea`]. Paint stays in `render.rs`
//! (caret `▏`, boxed overlay). The widget itself is not drawn.

use crossterm::event::KeyEvent;
use tui_textarea::{CursorMove, Input, Key, TextArea};

use super::store::CommentKey;

/// First footer row: save / delete / Ctrl-R resolve or unresolve / cancel.
pub fn comment_overlay_footer_save(resolved: bool) -> String {
    let hint = if resolved {
        "Ctrl-R unresolve"
    } else {
        "Ctrl-R resolve"
    };
    format!("Enter save · empty deletes · {hint} · Esc cancel")
}

/// Second footer row: advertised textarea keys. Leftover PTY asserts these.
pub const COMMENT_OVERLAY_FOOTER_EDIT: &str =
    "Shift+Enter newline · Ctrl-A/E line · Ctrl-Left/Right word";

/// Border (2) + title + target + two footer rows. Body lines add to this.
pub const COMMENT_OVERLAY_CHROME_ROWS: u16 = 6;

/// Visible body lines inside the box. Extra lines scroll around the caret.
pub const COMMENT_OVERLAY_MAX_BODY_LINES: usize = 8;

/// Overlay while typing a comment.
#[derive(Clone, Debug)]
pub struct CommentPrompt {
    /// Store key written on Enter.
    pub key: CommentKey,
    textarea: TextArea<'static>,
    /// One-line target shown in the overlay.
    pub label: String,
    /// Draft resolve flag. Enter persists it with the body.
    pub resolved: bool,
}

impl PartialEq for CommentPrompt {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.label == other.label
            && self.body() == other.body()
            && self.cursor() == other.cursor()
            && self.resolved == other.resolved
    }
}

impl Eq for CommentPrompt {}

impl CommentPrompt {
    /// Open a prompt with the caret at the end of `body`.
    pub fn new(key: CommentKey, body: String, label: String) -> Self {
        let mut textarea = textarea_from_body(&body);
        textarea.move_cursor(CursorMove::Bottom);
        textarea.move_cursor(CursorMove::End);
        Self {
            key,
            textarea,
            label,
            resolved: false,
        }
    }

    /// Set the draft resolve flag.
    pub fn with_resolved(mut self, resolved: bool) -> Self {
        self.resolved = resolved;
        self
    }

    /// Body as stored on save (`\n` between lines).
    pub fn body(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Caret as `(line, column)` Unicode scalars.
    pub fn cursor(&self) -> (usize, usize) {
        self.textarea.cursor()
    }

    /// Line count (always ≥ 1).
    pub fn line_count(&self) -> usize {
        self.textarea.lines().len().max(1)
    }

    /// Overlay height including borders. Idle status does not add a row.
    pub fn overlay_rows(&self) -> u16 {
        COMMENT_OVERLAY_CHROME_ROWS + self.visible_line_count() as u16
    }

    /// Body lines painted in the box, clamped to [`COMMENT_OVERLAY_MAX_BODY_LINES`].
    pub fn visible_line_count(&self) -> usize {
        self.line_count().clamp(1, COMMENT_OVERLAY_MAX_BODY_LINES)
    }

    /// Inclusive `start..end` of painted body lines (caret stays in view).
    pub fn visible_line_range(&self) -> (usize, usize) {
        visible_window(
            self.cursor().0,
            self.line_count(),
            self.visible_line_count(),
        )
    }

    /// Body lines plus the caret column on the focused line.
    pub fn painted_lines(&self) -> Vec<CommentBodyLine> {
        let (row, col) = self.cursor();
        let (start, end) = self.visible_line_range();
        self.textarea.lines()[start..end]
            .iter()
            .enumerate()
            .map(|(offset, text)| {
                let line_idx = start + offset;
                CommentBodyLine {
                    text: text.clone(),
                    caret: (line_idx == row).then_some(col.min(text.chars().count())),
                }
            })
            .collect()
    }

    /// Apply one overlay key. Shift+Enter inserts a newline. Other keys use
    /// the textarea map (Ctrl-A/E line, Ctrl-Left/Right word, arrows).
    pub fn input(&mut self, key: KeyEvent) {
        self.input_mapped(Input::from(key));
    }

    /// Apply a backend-agnostic textarea [`Input`].
    pub fn input_mapped(&mut self, input: Input) {
        if input.key == Key::Enter {
            self.textarea.insert_newline();
            return;
        }
        self.textarea.input(input);
    }

    /// Insert `c` at the caret and advance.
    pub fn insert_char(&mut self, c: char) {
        self.textarea.insert_str(c.to_string());
    }

    /// Insert a newline at the caret.
    pub fn insert_newline(&mut self) {
        self.textarea.insert_newline();
    }

    /// Delete the scalar before the caret.
    pub fn backspace(&mut self) {
        self.textarea.delete_char();
    }

    /// Delete the scalar after the caret.
    pub fn delete_forward(&mut self) {
        self.textarea.delete_next_char();
    }

    /// Move the caret one scalar left.
    pub fn move_left(&mut self) {
        self.textarea.move_cursor(CursorMove::Back);
    }

    /// Move the caret one scalar right.
    pub fn move_right(&mut self) {
        self.textarea.move_cursor(CursorMove::Forward);
    }

    /// Move the caret to the start of the current line.
    pub fn move_home(&mut self) {
        self.textarea.move_cursor(CursorMove::Head);
    }

    /// Move the caret to the end of the current line.
    pub fn move_end(&mut self) {
        self.textarea.move_cursor(CursorMove::End);
    }

    /// Move the caret to the start of the previous word.
    pub fn move_word_back(&mut self) {
        self.textarea.move_cursor(CursorMove::WordBack);
    }

    /// Move the caret to the start of the next word.
    pub fn move_word_forward(&mut self) {
        self.textarea.move_cursor(CursorMove::WordForward);
    }
}

/// One painted body row. `caret` is a scalar index when this row holds `▏`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentBodyLine {
    /// Line text without the caret glyph.
    pub text: String,
    /// Caret column, when this is the focused line.
    pub caret: Option<usize>,
}

/// Overlay that shows exported markdown after copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentExport {
    /// Markdown copied to the clipboard.
    pub markdown: String,
}

/// Overlay / export label for a key.
pub fn comment_key_label(key: &CommentKey) -> String {
    match key {
        CommentKey::Branch { repo, branch } => format!("{repo} · branch {branch}"),
        CommentKey::Commit { repo, sha } => {
            format!("{repo} · commit {}", short_sha(sha))
        }
        CommentKey::Worktree { path } => format!("{path} · worktree"),
        CommentKey::WorktreeLine {
            repo,
            branch,
            path,
            line,
            end_line,
        } => format!(
            "{repo} · branch {branch} · {path}:{}",
            line_span_label(*line, *end_line)
        ),
        CommentKey::CommitLine {
            repo,
            sha,
            path,
            line,
            end_line,
        } => format!(
            "{repo} · commit {} · {path}:{}",
            short_sha(sha),
            line_span_label(*line, *end_line)
        ),
    }
}

fn textarea_from_body(body: &str) -> TextArea<'static> {
    let lines: Vec<String> = if body.is_empty() {
        vec![String::new()]
    } else {
        body.split('\n').map(str::to_string).collect()
    };
    TextArea::from(lines)
}

fn visible_window(cursor_row: usize, line_count: usize, window: usize) -> (usize, usize) {
    let n = line_count.max(1);
    let w = window.clamp(1, n);
    if n <= w {
        return (0, n);
    }
    let mut start = cursor_row.saturating_sub(w / 2);
    if start + w > n {
        start = n - w;
    }
    (start, start + w)
}

fn short_sha(sha: &str) -> &str {
    if sha.len() >= 7 {
        &sha[..7]
    } else {
        sha
    }
}

fn line_span_label(line: u32, end_line: u32) -> String {
    if end_line == line {
        line.to_string()
    } else {
        format!("{line}-{end_line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn prompt(body: &str) -> CommentPrompt {
        CommentPrompt::new(
            CommentKey::Branch {
                repo: "app".into(),
                branch: "main".into(),
            },
            body.into(),
            "app · branch main".into(),
        )
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn caret_starts_at_end_and_home_inserts_prefix() {
        let mut prompt = prompt("hello");
        assert_eq!(prompt.cursor(), (0, 5));
        prompt.move_home();
        prompt.insert_char('X');
        assert_eq!(prompt.body(), "Xhello");
        assert_eq!(prompt.cursor(), (0, 1));
    }

    #[test]
    fn left_then_insert_edits_mid_string() {
        let mut prompt = prompt("hello");
        prompt.move_left();
        prompt.move_right();
        prompt.move_left();
        prompt.move_left();
        prompt.insert_char('-');
        assert_eq!(prompt.body(), "hel-lo");
        assert_eq!(prompt.cursor(), (0, 4));
    }

    #[test]
    fn delete_forward_is_not_backspace_at_end() {
        let mut prompt = prompt("hello");
        prompt.move_home();
        prompt.delete_forward();
        assert_eq!(prompt.body(), "ello");
        assert_eq!(prompt.cursor(), (0, 0));
        prompt.backspace();
        assert_eq!(prompt.body(), "ello");
        prompt.move_end();
        prompt.backspace();
        assert_eq!(prompt.body(), "ell");
        assert_eq!(prompt.cursor(), (0, 3));
    }

    #[test]
    fn shift_enter_inserts_newline_not_append() {
        let mut prompt = prompt("one");
        prompt.input(key(KeyCode::Enter, KeyModifiers::SHIFT));
        prompt.insert_char('t');
        prompt.insert_char('w');
        prompt.insert_char('o');
        assert_eq!(prompt.body(), "one\ntwo");
        assert_eq!(prompt.cursor(), (1, 3));
        assert_eq!(prompt.line_count(), 2);
        assert_eq!(prompt.overlay_rows(), 8);
    }

    #[test]
    fn ctrl_left_moves_to_previous_word() {
        let mut prompt = prompt("two three");
        prompt.move_word_back();
        assert_eq!(prompt.body(), "two three");
        assert_eq!(prompt.cursor(), (0, 4));
        let painted = prompt.painted_lines();
        assert_eq!(painted.len(), 1);
        assert_eq!(painted[0].caret, Some(4));
    }

    #[test]
    fn ctrl_right_moves_to_next_word() {
        let mut prompt = prompt("two three");
        prompt.move_home();
        prompt.input(key(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(prompt.body(), "two three");
        assert_eq!(prompt.cursor(), (0, 4));
        prompt.move_word_forward();
        assert_eq!(prompt.cursor(), (0, 9));
    }

    #[test]
    fn ctrl_e_is_line_end_on_second_line() {
        let mut prompt = prompt("one");
        prompt.insert_newline();
        for c in "two three".chars() {
            prompt.insert_char(c);
        }
        prompt.move_home();
        prompt.input(key(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(prompt.body(), "one\ntwo three");
        assert_eq!(prompt.cursor(), (1, 9));
    }

    #[test]
    fn ctrl_a_is_line_start_on_second_line() {
        let mut prompt = prompt("one");
        prompt.insert_newline();
        for c in "two three".chars() {
            prompt.insert_char(c);
        }
        prompt.input(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(prompt.body(), "one\ntwo three");
        assert_eq!(prompt.cursor(), (1, 0));
    }

    #[test]
    fn home_is_current_line_not_buffer_start() {
        let mut prompt = prompt("one\ntwo");
        prompt.move_home();
        assert_eq!(prompt.cursor(), (1, 0));
        prompt.insert_char('X');
        assert_eq!(prompt.body(), "one\nXtwo");
    }

    #[test]
    fn with_resolved_keeps_body_and_caret() {
        let prompt = prompt("hello").with_resolved(true);
        assert!(prompt.resolved);
        assert_eq!(prompt.body(), "hello");
        assert_eq!(prompt.cursor(), (0, 5));
        assert!(comment_overlay_footer_save(true).contains("Ctrl-R unresolve"));
        assert!(comment_overlay_footer_save(false).contains("Ctrl-R resolve"));
    }
}
