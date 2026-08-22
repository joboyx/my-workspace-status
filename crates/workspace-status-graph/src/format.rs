//! Headless line format for graph rows. The widget paints these strings.

use crate::glyphs::GlyphSet;
use crate::model::{Commit, GraphRow, Stash, SyncState, SyncStatus, Worktree};

/// Format the sync header line (`branch` plus ahead/behind marks).
pub fn format_sync(sync: &SyncState, glyphs: &GlyphSet) -> String {
    let mut parts = vec![sync.branch.clone()];
    match sync.status {
        SyncStatus::NoUpstream => parts.push("no-upstream".to_string()),
        SyncStatus::UpToDate => {}
        SyncStatus::Ahead | SyncStatus::Behind | SyncStatus::Diverged => {
            if sync.ahead > 0 {
                parts.push(format!("{}{}", glyphs.ahead, sync.ahead));
            }
            if sync.behind > 0 {
                parts.push(format!("{}{}", glyphs.behind, sync.behind));
            }
        }
    }
    parts.join(" ")
}

/// Format one visible row. Headless callers share this function.
/// The widget paints a multi-lane gutter and uses [`format_label`].
pub fn format_row(row: &GraphRow, glyphs: &GlyphSet) -> String {
    match row {
        GraphRow::Uncommitted => format!("{} dirty", glyphs.uncommitted),
        GraphRow::Stash(stash) => format_stash(stash, glyphs),
        GraphRow::Commit {
            commit,
            is_head,
            worktrees,
        } => format_commit(commit, *is_head, worktrees, glyphs),
        GraphRow::Worktree(worktree) => format_worktree(worktree, glyphs),
    }
}

/// Label after the gutter. Commit and stash drop the node glyph because
/// the gutter already paints it.
pub fn format_label(row: &GraphRow, glyphs: &GlyphSet) -> String {
    match row {
        GraphRow::Uncommitted | GraphRow::Worktree(_) => format_row(row, glyphs),
        GraphRow::Stash(stash) => format!("{}  {}", stash.stash_ref, stash.subject),
        GraphRow::Commit {
            commit,
            is_head,
            worktrees,
        } => format_commit_text(commit, *is_head, worktrees, glyphs),
    }
}

fn format_stash(stash: &Stash, glyphs: &GlyphSet) -> String {
    format!("{} {}  {}", glyphs.stash, stash.stash_ref, stash.subject)
}

fn format_commit_text(
    commit: &Commit,
    is_head: bool,
    worktrees: &[Worktree],
    glyphs: &GlyphSet,
) -> String {
    let mut line = format!("{}  {}", short_id(&commit.id), commit.subject);
    if is_head {
        line.push_str("  [HEAD]");
    }
    for name in &commit.refs {
        line.push_str("  ");
        line.push_str(name);
    }
    for worktree in worktrees {
        line.push_str("  ");
        line.push_str(&worktree_mark(worktree, glyphs));
    }
    line
}

fn format_commit(
    commit: &Commit,
    is_head: bool,
    worktrees: &[Worktree],
    glyphs: &GlyphSet,
) -> String {
    let node = if is_head {
        glyphs.head_commit
    } else {
        glyphs.commit
    };
    format!("{} {}", node, format_commit_text(commit, is_head, worktrees, glyphs))
}

fn format_worktree(worktree: &Worktree, glyphs: &GlyphSet) -> String {
    let mut line = worktree_mark(worktree, glyphs);
    if worktree.is_current {
        line.push_str("  [HEAD]");
    }
    line
}

fn worktree_mark(worktree: &Worktree, glyphs: &GlyphSet) -> String {
    let mut mark = format!("{} {}", glyphs.worktree, worktree.path);
    if let Some(branch) = &worktree.branch {
        mark.push(' ');
        mark.push_str(branch);
    }
    if worktree.ignored {
        mark.push_str("  [ignored]");
    }
    mark
}

fn short_id(id: &str) -> &str {
    let end = id.char_indices().nth(7).map(|(i, _)| i).unwrap_or(id.len());
    &id[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{ASCII, UNICODE};
    use crate::model::{Commit, GraphRow, Worktree};

    #[test]
    fn short_id_keeps_ids_shorter_than_seven() {
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("abcdefg"), "abcdefg");
        assert_eq!(short_id("abcdefghij"), "abcdefg");
    }

    #[test]
    fn format_sync_ascii_uses_caret_and_v() {
        let sync = SyncState {
            branch: "main".into(),
            status: SyncStatus::Diverged,
            ahead: 2,
            behind: 3,
        };
        assert_eq!(format_sync(&sync, &ASCII), "main ^2 v3");
    }

    #[test]
    fn format_row_commit_includes_worktree_mark() {
        let row = GraphRow::Commit {
            commit: Commit {
                id: "aaa1111bbbb".into(),
                subject: "add graph crate".into(),
                parents: Vec::new(),
                refs: vec!["main".into()],
            },
            is_head: true,
            worktrees: vec![Worktree {
                path: ".worktrees/feature/graph".into(),
                head_id: Some("aaa1111bbbb".into()),
                branch: Some("feature/graph".into()),
                ignored: false,
                is_current: false,
            }],
        };
        assert_eq!(
            format_row(&row, &UNICODE),
            "⊙ aaa1111  add graph crate  [HEAD]  main  🔗 .worktrees/feature/graph feature/graph"
        );
    }

    #[test]
    fn format_row_ignored_worktree() {
        let row = GraphRow::Worktree(Worktree {
            path: "notes".into(),
            head_id: None,
            branch: None,
            ignored: true,
            is_current: false,
        });
        assert_eq!(format_row(&row, &UNICODE), "🔗 notes  [ignored]");
    }
}
