//! File diff text for the right pane.

use std::path::Path;

use crate::git::exec_git;
use crate::snapshot::FileChange;

/// Load a unified diff for one dirty file. Untracked files show as added text.
pub fn load_file_diff(cwd: &Path, repo: &str, change: &FileChange) -> Vec<String> {
    let repo_dir = cwd.join(repo);
    if change.untracked {
        return untracked_lines(&repo_dir, &change.path);
    }
    let mut lines = Vec::new();
    if change.staged_status.is_some() {
        let staged = exec_git(
            &["diff", "--cached", "--", &change.path],
            &repo_dir,
        );
        if !staged.is_empty() {
            lines.push("staged".into());
            lines.extend(staged.lines().map(str::to_string));
        }
    }
    if change.unstaged_status.is_some() {
        let unstaged = exec_git(&["diff", "--", &change.path], &repo_dir);
        if !unstaged.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("unstaged".into());
            lines.extend(unstaged.lines().map(str::to_string));
        }
    }
    if lines.is_empty() {
        lines.push("(no diff)".into());
    }
    lines
}

fn untracked_lines(repo_dir: &Path, path: &str) -> Vec<String> {
    let abs = repo_dir.join(path);
    match std::fs::read_to_string(&abs) {
        Ok(body) => {
            let mut lines = vec![format!("untracked  {path}")];
            for line in body.lines() {
                lines.push(format!("+{line}"));
            }
            if lines.len() == 1 {
                lines.push("+".into());
            }
            lines
        }
        Err(_) => vec![format!("untracked  {path}"), "(unreadable)".into()],
    }
}
