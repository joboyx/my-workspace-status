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
