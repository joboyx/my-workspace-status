use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{compare_tip_shape_workspace, git_stdout};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Same gap as `pty_workspace_command_palette`. A same-letter `h`/`j`/`k`/`l`
/// burst is dropped by `discard_held_nav_backlog` after the first press.
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

const AHEAD: &str = "ahead";
const BEHIND: &str = "behind";
const DIVERGED: &str = "diverged";
const ORPHAN: &str = "orphan";

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

/// Type a palette filter at human key gaps for nav letters.
///
/// `keys("vs default")` can drop the `l` in `default`.
fn type_palette_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
        }
    }
}

fn open_vs_default(tui: &mut PtySession) {
    tui.ctrl_letter('k');
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains("Ctrl-k"),
        "Ctrl-k opens the command palette (a no-op leaves idle chrome without Enter run)",
        WAIT,
    );
    type_palette_filter(tui, "vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("vs default")
        },
        "palette filter `vs default` shows `Diff vs default` (Enter before the filter lands would run the first catalog row; a dropped nav letter would keep a truncated query)",
        WAIT,
    );
    tui.enter();
}

fn on_workspace(screen: &str) -> bool {
    screen.contains("# workspace")
        && !screen.contains("COMMITTED")
        && !screen.contains("...HEAD")
        && !screen.contains("No committed changes")
        && !screen.contains("No merge base between")
}

fn jump_workspace(tui: &mut PtySession) {
    tui.key('g');
    tui.key('1');
    tui.wait_pred(
        on_workspace,
        "g1 activates Workspace (compare chrome is not the focused surface)",
        WAIT,
    );
}

fn search_checkout(tui: &mut PtySession, name: &str) {
    tui.search(name);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, name) && !tree_cursor_on(screen, "workspace"),
        &format!("/{name} keeps the tree cursor on {name} (a miss jumps to workspace)"),
        WAIT,
    );
}

fn vs_count(screen: &str) -> usize {
    screen.matches("· vs").count()
}

fn head_sha(workspace: &Path, name: &str) -> String {
    git_stdout(&workspace.join(name), &["rev-parse", "HEAD"])
}

fn head_branch(workspace: &Path, name: &str) -> String {
    git_stdout(
        &workspace.join(name),
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )
}

/// Diff vs default paints ahead files, empty behind, diverged feature-only,
/// and unrelated merge-base chrome. Compare does not check out a tip.
#[test]
fn pty_compare_ahead_behind_diverged_and_unrelated() {
    let (_root, workspace) = compare_tip_shape_workspace();
    let ahead_sha = head_sha(&workspace, AHEAD);
    let ahead_branch = head_branch(&workspace, AHEAD);
    let behind_sha = head_sha(&workspace, BEHIND);
    let behind_branch = head_branch(&workspace, BEHIND);
    let diverged_sha = head_sha(&workspace, DIVERGED);
    let diverged_branch = head_branch(&workspace, DIVERGED);
    let orphan_sha = head_sha(&workspace, ORPHAN);
    let orphan_branch = head_branch(&workspace, ORPHAN);

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_has(screen, AHEAD)
                && tree_has(screen, BEHIND)
                && tree_has(screen, DIVERGED)
                && tree_has(screen, ORPHAN)
        },
        "first paint: ahead, behind, diverged, and orphan are on the tree",
        GIT_WAIT,
    );

    search_checkout(&mut tui, AHEAD);
    open_vs_default(&mut tui);
    tui.wait_pred(
        |screen| {
            screen.contains("ahead · vs origin/main")
                && screen.contains("alpha.txt")
                && !screen.contains("No merge base between")
                && !screen.contains("Base ref not found")
        },
        "ahead vs origin/main lists alpha.txt without merge-base or base-ref error chrome",
        GIT_WAIT,
    );

    jump_workspace(&mut tui);
    search_checkout(&mut tui, BEHIND);
    open_vs_default(&mut tui);
    tui.wait_pred(
        |screen| {
            screen.contains("behind · vs main")
                && screen.contains("No committed changes")
                && !screen.contains("alpha.txt")
                && !screen.contains("beta.txt")
        },
        "behind vs main paints No committed changes (no ahead-only alpha.txt / beta.txt)",
        GIT_WAIT,
    );

    jump_workspace(&mut tui);
    search_checkout(&mut tui, DIVERGED);
    open_vs_default(&mut tui);
    tui.wait_pred(
        |screen| screen.contains("diverged · vs main") && screen.contains("feature.txt"),
        "diverged vs main lists feature.txt",
        GIT_WAIT,
    );
    let diverged_screen = tui.screen();
    assert!(
        !diverged_screen.contains("main-only.txt"),
        "diverged must not list main-only.txt:\n{diverged_screen}"
    );

    jump_workspace(&mut tui);
    search_checkout(&mut tui, ORPHAN);
    open_vs_default(&mut tui);
    tui.wait_pred(
        |screen| {
            screen.contains("orphan · vs main")
                && screen.contains("No merge base between main and HEAD")
        },
        "orphan vs main keeps the compare tab with No merge base between main and HEAD",
        GIT_WAIT,
    );

    let strip = tui.screen();
    assert!(
        strip.contains("Workspace"),
        "tab strip must keep Workspace:\n{strip}"
    );
    assert!(
        strip.contains("ahead · vs origin/main")
            && strip.contains("behind · vs main")
            && strip.contains("diverged · vs main")
            && strip.contains("orphan · vs main"),
        "tab strip must show all four compare labels:\n{strip}"
    );
    assert_eq!(
        vs_count(&strip),
        4,
        "tab strip must have exactly four · vs labels:\n{strip}"
    );

    assert_eq!(
        head_sha(&workspace, AHEAD),
        ahead_sha,
        "ahead HEAD SHA changed; compare must not check out a tip"
    );
    assert_eq!(
        head_branch(&workspace, AHEAD),
        ahead_branch,
        "ahead HEAD branch changed; compare must not check out a tip"
    );
    assert_eq!(
        head_sha(&workspace, BEHIND),
        behind_sha,
        "behind HEAD SHA changed; compare must not check out a tip"
    );
    assert_eq!(
        head_branch(&workspace, BEHIND),
        behind_branch,
        "behind HEAD branch changed; compare must not check out a tip"
    );
    assert_eq!(
        head_sha(&workspace, DIVERGED),
        diverged_sha,
        "diverged HEAD SHA changed; compare must not check out a tip"
    );
    assert_eq!(
        head_branch(&workspace, DIVERGED),
        diverged_branch,
        "diverged HEAD branch changed; compare must not check out a tip"
    );
    assert_eq!(
        head_sha(&workspace, ORPHAN),
        orphan_sha,
        "orphan HEAD SHA changed; compare must not check out a tip"
    );
    assert_eq!(
        head_branch(&workspace, ORPHAN),
        orphan_branch,
        "orphan HEAD branch changed; compare must not check out a tip"
    );
}
