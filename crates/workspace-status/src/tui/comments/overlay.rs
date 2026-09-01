//! Comment overlay types shown while typing or after markdown copy.

use super::store::CommentKey;

/// Overlay while typing a comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentPrompt {
    /// Store key written on Enter.
    pub key: CommentKey,
    /// Body being edited.
    pub body: String,
    /// One-line target shown in the overlay.
    pub label: String,
    /// Caret as a Unicode scalar index into [`Self::body`].
    pub cursor: usize,
}

impl CommentPrompt {
    /// Open a prompt with the caret at the end of `body`.
    pub fn new(key: CommentKey, body: String, label: String) -> Self {
        let cursor = body.chars().count();
        Self {
            key,
            body,
            label,
            cursor,
        }
    }

    /// Insert `c` at the caret and advance.
    pub fn insert_char(&mut self, c: char) {
        self.clamp_cursor();
        let i = byte_index(&self.body, self.cursor);
        self.body.insert(i, c);
        self.cursor += 1;
    }

    /// Delete the scalar before the caret.
    pub fn backspace(&mut self) {
        self.clamp_cursor();
        if self.cursor == 0 {
            return;
        }
        let start = byte_index(&self.body, self.cursor - 1);
        let end = byte_index(&self.body, self.cursor);
        self.body.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// Delete the scalar after the caret.
    pub fn delete_forward(&mut self) {
        self.clamp_cursor();
        if self.cursor >= self.body.chars().count() {
            return;
        }
        let start = byte_index(&self.body, self.cursor);
        let end = byte_index(&self.body, self.cursor + 1);
        self.body.replace_range(start..end, "");
    }

    /// Move the caret one scalar left.
    pub fn move_left(&mut self) {
        self.clamp_cursor();
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the caret one scalar right.
    pub fn move_right(&mut self) {
        let len = self.body.chars().count();
        self.cursor = self.cursor.saturating_add(1).min(len);
    }

    /// Move the caret to the start of the body.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move the caret to the end of the body.
    pub fn move_end(&mut self) {
        self.cursor = self.body.chars().count();
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.body.chars().count());
    }
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

fn byte_index(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn caret_starts_at_end_and_home_inserts_prefix() {
        let mut prompt = prompt("hello");
        assert_eq!(prompt.cursor, 5);
        prompt.move_home();
        prompt.insert_char('X');
        assert_eq!(prompt.body, "Xhello");
        assert_eq!(prompt.cursor, 1);
    }

    #[test]
    fn left_then_insert_edits_mid_string() {
        let mut prompt = prompt("hello");
        prompt.move_left();
        prompt.move_left();
        prompt.insert_char('-');
        assert_eq!(prompt.body, "hel-lo");
        assert_eq!(prompt.cursor, 4);
    }

    #[test]
    fn delete_forward_is_not_backspace_at_end() {
        let mut prompt = prompt("hello");
        prompt.move_home();
        prompt.delete_forward();
        assert_eq!(prompt.body, "ello");
        assert_eq!(prompt.cursor, 0);
        prompt.backspace();
        assert_eq!(prompt.body, "ello");
        prompt.move_end();
        prompt.backspace();
        assert_eq!(prompt.body, "ell");
        assert_eq!(prompt.cursor, 3);
    }
}
