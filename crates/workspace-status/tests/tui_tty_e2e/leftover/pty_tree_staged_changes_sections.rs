use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, staged_and_changes_workspace};
use crate::support::{documented_launch_first_paint, tree_has, WAIT};

/// Right-pane `STAGED` / `UNSTAGED` headers are not tree section labels.
fn left_omits_section_labels(screen: &str) -> bool {
    let left = left_tree(screen);
    !left.contains("Staged") && !left.contains("Changes")
}

/// ASCII leftover PTY: `# Staged` then `~ Changes` on the left tree.
fn left_paints_both_section_headers(screen: &str) -> bool {
    let left = left_tree(screen);
    let Some(staged_at) = left.find("# Staged") else {
        return false;
    };
    let Some(changes_at) = left.find("~ Changes") else {
        return false;
    };
    staged_at < changes_at
        && tree_has(screen, "README.md")
        && tree_has(screen, "staged.txt")
        && tree_has(screen, "app")
}

/// Unstaged-only daily seed: no Staged / Changes chrome on the left tree.
fn documented_unstaged_only_omits_sections(screen: &str) -> bool {
    documented_launch_first_paint(screen) && left_omits_section_labels(screen)
}

/// Daily workspace (unstaged README only) must not paint tree section labels.
///
/// Docs: no staged paths → same file/dir trie, no `Section` nodes. Live
/// leftover PTY (`WS_STATUS_GLYPHS=ascii`): left tree after first paint.
/// Oracle is `left_tree` / `tree_has`, not the full screen — the right pane
/// already paints `UNSTAGED` / `STAGED` diff headers. A whole-screen
/// substring, a right-pane header treated as a tree section, or
/// Staged/Changes chrome on an unstaged-only checkout cannot pass.
#[test]
fn pty_tree_unstaged_only_omits_staged_changes_sections() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_unstaged_only_omits_sections,
        "first paint: leftover left tree has no Staged / Changes section labels",
        WAIT,
    );
}

/// Staged + unstaged/untracked under one checkout paints both section headers.
///
/// Docs: any staged path → `# Staged` then `~ Changes` (ASCII leftover PTY).
/// Seed: `app/staged.txt` is staged; `app/README.md` stays unstaged. Live PTY
/// after first paint. Oracle is `left_tree` / `tree_has` so right-pane
/// `STAGED` / `UNSTAGED` cannot satisfy the assert. Missing either header,
/// reversed order, or headers only on the diff pane cannot pass.
#[test]
fn pty_tree_staged_and_changes_paint_section_headers() {
    let (_root, workspace) = staged_and_changes_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        left_paints_both_section_headers,
        "first paint: leftover left tree shows ASCII # Staged then ~ Changes",
        WAIT,
    );
}
