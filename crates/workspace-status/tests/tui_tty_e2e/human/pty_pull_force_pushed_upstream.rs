use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{force_pushed_workspace, git_stdout};
use crate::support::{
    ack_rows, after_repo_name, crumb_row, repo_row_in_sync, status_row, tree_cursor_on, tree_has,
    tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

fn rev(repo: &Path, name: &str) -> String {
    git_stdout(repo, &["rev-parse", name])
}

fn row_force_pushed(screen: &str) -> bool {
    after_repo_name(screen, "rewrite").is_some_and(|after| after.contains("force-pushed"))
}

fn no_wrong_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("pull it from a terminal")
        && !screen.contains("Stash ")
}

/// First paint: cursor on the `rewrite` row, which shows the force-push text
/// with two dropped commits and none of yours.
fn idle_r0_row(screen: &str) -> bool {
    tree_cursor_on(screen, "rewrite")
        && row_force_pushed(screen)
        && tree_line_containing(screen, "rewrite")
            .is_some_and(|line| line.contains("2 dropped, 0 yours"))
        && !screen.contains("was rewritten")
        && !screen.contains("Moved ")
        && no_wrong_overlays(screen)
}

/// `p` opened the reset confirm. Nothing moved yet.
fn reset_confirm_open(screen: &str) -> bool {
    screen.contains("rewrite: origin/main was rewritten. Move main to origin/main?")
        && screen.contains("Drops 2 commits that origin/main dropped; none are yours.")
        && screen.contains("reset    n")
        && !screen.contains("Moved ")
        && no_wrong_overlays(screen)
}

/// The row is back in sync: no force-push text, no diverged mark.
fn row_in_sync_after_reset(screen: &str) -> bool {
    repo_row_in_sync(screen, "rewrite")
        && !screen.contains("force-pushed")
        && !screen.contains("was rewritten")
        && !tree_has(screen, "diverged")
        && tree_line_containing(screen, "rewrite")
            .is_some_and(|line| !line.contains('Y') && !line.contains("dropped"))
        && no_wrong_overlays(screen)
}

/// `p` on a branch whose upstream was force-pushed (nothing of yours on top)
/// opens a y/n box. `y` moves the branch to the upstream with `git reset
/// --keep` and names the undo command.
///
/// Docs: `docs/git-operations.md`, `p` on a diverged row whose upstream was
/// rewritten. Live PTY after first paint (cursor already on `rewrite`):
/// the tree row shows `force-pushed · 2 dropped, 0 yours`. `p` opens the box
/// `rewrite: origin/main was rewritten. Move main to origin/main?` and writes nothing
/// (tip unchanged on disk). `y` moves `main` to `origin/main` on disk, prints
/// `Moved main to origin/main · undo: git reset --keep <old tip, 12 hex>`, and
/// the row returns to in sync with a clean working tree. The undo text is an
/// info note on the status line (not a toast), so the full 12-hex old tip shows.
#[test]
fn pty_p_resets_branch_onto_force_pushed_upstream() {
    let (_root, workspace) = force_pushed_workspace(false);
    let repo = workspace.join("rewrite");
    let old_tip = rev(&repo, "main");
    let upstream_tip = rev(&repo, "origin/main");
    assert_ne!(old_tip, upstream_tip, "seed must be diverged");
    // The full undo text with the 12-hex old tip, in the breadcrumb status slot.
    let undo = format!(
        "Moved main to origin/main · undo: git reset --keep {}",
        &old_tip[..12]
    );

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("rewrite", WAIT);
    tui.wait_pred(
        idle_r0_row,
        "first paint: cursor on rewrite, row shows force-pushed · 2 dropped, 0 yours",
        GIT_WAIT,
    );

    tui.key('p');
    tui.wait_pred(
        reset_confirm_open,
        "p opens the reset confirm (rewrite: origin/main was rewritten. Move main to origin/main?)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        reset_confirm_open,
        "the confirm holds: nothing moves until y",
        WAIT,
    );
    assert_eq!(
        rev(&repo, "main"),
        old_tip,
        "p alone must not move main:\n{}",
        tui.screen()
    );

    tui.key('y');
    // The info note expires after a few seconds and can be gone before the
    // reload lands under load, so remember it once seen on any polled screen.
    let seen = std::cell::Cell::new(false);
    tui.wait_pred(
        |screen| {
            if crumb_row(screen).contains(&undo) {
                seen.set(true);
            }
            seen.get() && row_in_sync_after_reset(screen)
        },
        "y resets: full undo text on the status line, row back in sync",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        row_in_sync_after_reset,
        "in-sync paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    assert_eq!(
        rev(&repo, "main"),
        rev(&repo, "origin/main"),
        "main must equal origin/main on disk:\n{}",
        tui.screen()
    );
    assert_eq!(rev(&repo, "main"), upstream_tip, "{}", tui.screen());
    assert_eq!(
        rev(&repo, "main@{1}"),
        old_tip,
        "the undo target (previous tip) must be the old main:\n{}",
        tui.screen()
    );
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain"]),
        "",
        "working tree must be clean:\n{}",
        tui.screen()
    );
    assert!(
        !status_row(&tui.screen()).contains("force-pushed"),
        "{}",
        tui.screen()
    );
}

/// R1 row: the upstream was force-pushed under one commit of yours.
fn idle_r1_row(screen: &str) -> bool {
    tree_cursor_on(screen, "rewrite")
        && tree_line_containing(screen, "rewrite")
            .is_some_and(|line| line.contains("force-pushed") && line.contains("1 yours"))
        && !screen.contains("was rewritten")
        && no_wrong_overlays(screen)
}

/// `p` named the rebase command and opened no box.
fn rebase_command_shown(screen: &str, fork_point: &str) -> bool {
    let acks = ack_rows(screen);
    let rebase = format!("rebase --onto origin/main {fork_point} main");
    (acks.contains("under your 1 commit") || screen.contains("under your 1 commit"))
        && (acks.contains(&rebase) || screen.contains(&rebase))
        && !screen.contains("was rewritten")
        && !screen.contains("Moved ")
}

/// `p` on a branch with a commit of yours under a force-pushed upstream
/// writes nothing and names the `git rebase --onto` that keeps the commit.
///
/// Docs: `docs/git-operations.md`, `p` on a rewritten upstream with commits
/// of yours. Live PTY after first paint (cursor already on `rewrite`): the row
/// shows `force-pushed · 1 dropped, 1 yours`. `p` opens no box and the status
/// reads `rewrite: origin/main was force-pushed under your 1 commit — run git
/// rebase --onto origin/main <fork point, 12 hex> main`. Branch tip, working
/// tree, and `origin/main` are unchanged on disk.
#[test]
fn pty_p_on_force_pushed_with_own_commit_shows_rebase_command() {
    let (_root, workspace) = force_pushed_workspace(true);
    let repo = workspace.join("rewrite");
    let old_tip = rev(&repo, "main");
    let upstream_tip = rev(&repo, "origin/main");
    let fork_point = rev(&repo, "HEAD~1");
    assert_ne!(old_tip, upstream_tip, "seed must be diverged");

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("rewrite", WAIT);
    tui.wait_pred(
        idle_r1_row,
        "first paint: cursor on rewrite, row shows force-pushed with 1 yours",
        GIT_WAIT,
    );

    tui.key('p');
    // The warn can sit on the status line or in a toast; remember it once seen.
    let seen = std::cell::Cell::new(false);
    let short = fork_point[..12].to_string();
    tui.wait_pred(
        |screen| {
            if rebase_command_shown(screen, &short) {
                seen.set(true);
            }
            seen.get()
        },
        "p names the rebase command (under your 1 commit, rebase --onto origin/main <fork> main)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    let screen = tui.screen();
    assert!(!screen.contains("was rewritten"), "no box:\n{screen}");
    assert!(!screen.contains("Moved "), "no reset:\n{screen}");
    tui.wait_pred(idle_r1_row, "row still shows the force-push after p", WAIT);

    assert_eq!(rev(&repo, "main"), old_tip, "tip must not move:\n{screen}");
    assert_eq!(rev(&repo, "origin/main"), upstream_tip, "{screen}");
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain"]),
        "",
        "{screen}"
    );
}
