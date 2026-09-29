use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{git, git_env, seed_repo, unique_root};
use crate::support::{
    crumb_row, panes_tree_unfocused_diff_focused, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS,
    WAIT,
};

const KEY_GAP_MS: u64 = 50;
const ALPHA: &str = "ALPHA-NEW";
const OMEGA: &str = "OMEGA-NEW";

const COMMITTED: &str = "\
keep-a
keep-b
keep-c
ALPHA-OLD
keep-d
keep-e
keep-f
pad-1
pad-2
pad-3
pad-4
pad-5
pad-6
pad-7
pad-8
OMEGA-OLD
keep-x
keep-y
keep-z
";

fn two_hunk_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty-visual-palette");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", false);
    let repo = workspace.join("app");
    fs::write(repo.join("regions.txt"), COMMITTED).unwrap();
    git(&repo, &["add", "regions.txt"]);
    git(&repo, &["commit", "-q", "-m", "regions"]);
    fs::write(
        repo.join("regions.txt"),
        COMMITTED
            .replace("ALPHA-OLD", ALPHA)
            .replace("OMEGA-OLD", OMEGA),
    )
    .unwrap();
    (root, workspace)
}

fn first_paint(screen: &str) -> bool {
    tree_cursor_on(screen, "regions.txt")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains(ALPHA)
        && screen.contains(OMEGA)
        && !screen.contains("VISUAL")
}

fn right_diff_focused(screen: &str) -> bool {
    !tree_cursor_on(screen, "regions.txt")
        && panes_tree_unfocused_diff_focused(screen)
        && screen.contains(ALPHA)
        && !screen.contains("VISUAL")
}

fn highlight_active(screen: &str) -> bool {
    screen.contains("VISUAL")
        && screen.contains("stage / unstage")
        && screen.contains("revert")
        && screen.contains("cancel highlight")
        && panes_tree_unfocused_diff_focused(screen)
}

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn palette_highlight_rows(screen: &str) -> bool {
    palette_open(screen)
        && screen.contains("HIGHLIGHT")
        && screen.contains("Stage highlighted lines")
        && screen.contains("Revert highlighted lines")
        && screen.contains("Exit highlight")
}

fn revert_range_confirm(screen: &str) -> bool {
    screen.contains("Discard highlighted lines in regions.txt?")
        && screen.contains("cancel")
        && !screen.contains("VISUAL")
        && !screen.contains("revert + delete untracked")
}

fn reverted_range_toast(screen: &str) -> bool {
    crumb_row(screen).contains("reverted range regions.txt")
        && !screen.contains("Discard highlighted lines")
        && !screen.contains("VISUAL")
}

fn git_diff(repo: &Path, cached: bool) -> String {
    let mut cmd = Command::new("git");
    cmd.arg("diff");
    if cached {
        cmd.arg("--cached");
    }
    cmd.args(["--", "regions.txt"]).current_dir(repo);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().expect("git diff");
    assert!(
        out.status.success(),
        "git diff failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Type a palette filter with a gap after nav letters (held-nav backlog).
fn type_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(KEY_GAP_MS);
        }
    }
}

/// `V` over the first hunk, `:` palette, then "Revert highlighted lines".
///
/// Highlight mode opens the palette. The HIGHLIGHT rows show first. A
/// whole-file row shows why it waits (`exit highlight first`). Esc returns
/// to highlight with the same range. Enter on "Revert highlighted lines"
/// opens the range confirm, and `y` drops ALPHA from the worktree, keeps
/// OMEGA, and leaves the index untouched. Git on disk is the oracle.
#[test]
fn pty_v_colon_palette_reverts_highlighted_range() {
    let (_root, workspace) = two_hunk_workspace();
    let repo = workspace.join("app");
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        first_paint,
        "launch is the two-hunk dirty regions.txt file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_diff_focused,
        "Tab focuses the two-hunk regions.txt diff",
        WAIT,
    );

    tui.shift_letter('V');
    tui.wait_pred(highlight_active, "Shift+V paints VISUAL", WAIT);
    tui.wait_ms(KEY_GAP_MS);
    for _ in 0..6 {
        tui.letter_press('j');
        tui.wait_ms(KEY_GAP_MS);
    }
    tui.wait_pred(
        |screen| highlight_active(screen) && screen.contains(ALPHA),
        "j extends VISUAL over the first hunk (ALPHA)",
        WAIT,
    );

    tui.key(':');
    tui.wait_pred(
        palette_highlight_rows,
        "`:` in highlight opens the palette with the HIGHLIGHT rows",
        WAIT,
    );
    type_filter(&mut tui, "fetch");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Fetch remotes")
                && screen.contains("exit highlight first")
        },
        "a whole-file row shows the exit-highlight reason in the footer",
        WAIT,
    );
    tui.enter();
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains("exit highlight first"),
        "Enter on a disabled row keeps the palette open",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| !palette_open(screen) && highlight_active(screen),
        "Esc closes the palette and keeps the highlight",
        WAIT,
    );

    tui.key(':');
    tui.wait_pred(palette_highlight_rows, "`:` reopens the palette", WAIT);
    type_filter(&mut tui, "revert highlighted");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("revert highlighted")
                && screen.contains("Revert highlighted lines")
                && !screen.contains("Stage highlighted lines")
        },
        "filter narrows to Revert highlighted lines",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        revert_range_confirm,
        "Enter opens the range revert confirm",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert!(
        git_diff(&repo, false).contains(ALPHA),
        "the confirm alone must not touch the worktree"
    );

    tui.letter_press('y');
    tui.wait_pred(
        reverted_range_toast,
        "y reverts the highlighted range (reverted range toast)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let unstaged = git_diff(&repo, false);
    let cached = git_diff(&repo, true);
    assert!(
        !unstaged.contains(ALPHA) && unstaged.contains(OMEGA),
        "worktree must drop ALPHA and keep OMEGA:\nunstaged={unstaged}"
    );
    assert!(
        cached.is_empty(),
        "range revert must not touch the index:\ncached={cached}"
    );
}
