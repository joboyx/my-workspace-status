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
    let root = unique_root("ws-tui-tty-visual-stage");
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

fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

fn highlight_active(screen: &str) -> bool {
    screen.contains("VISUAL")
        && screen.contains("cancel highlight")
        && panes_tree_unfocused_diff_focused(screen)
        && tree_has(screen, "regions.txt")
        && !tree_cursor_on(screen, "regions.txt")
        && !screen.contains("MOVE")
}

fn right_diff_focused(screen: &str) -> bool {
    tree_has(screen, "regions.txt")
        && !tree_cursor_on(screen, "regions.txt")
        && panes_tree_unfocused_diff_focused(screen)
        && screen.contains("UNSTAGED")
        && screen.contains(ALPHA)
        && !screen.contains("VISUAL")
        && !screen.contains("MOVE")
}

fn first_paint(screen: &str) -> bool {
    tree_cursor_on(screen, "regions.txt")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains(ALPHA)
        && screen.contains(OMEGA)
        && !screen.contains("VISUAL")
        && !screen.contains("MOVE")
}

fn staged_range_toast(screen: &str) -> bool {
    crumb_row(screen).contains("staged range")
        && !screen.contains("VISUAL")
        && screen.contains("STAGED")
}

fn unstaged_range_toast(screen: &str) -> bool {
    crumb_row(screen).contains("unstaged range")
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
    let code = out.status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "git diff failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo_cached(workspace: &Path) -> String {
    git_diff(&workspace.join("app"), true)
}

fn repo_unstaged(workspace: &Path) -> String {
    git_diff(&workspace.join("app"), false)
}

fn highlight_first_hunk(tui: &mut PtySession) {
    tui.shift_letter('V');
    tui.wait_pred(
        highlight_active,
        "CSI-u Shift+V paints VISUAL highlight",
        WAIT,
    );
    tui.wait_ms(KEY_GAP_MS);
    for _ in 0..6 {
        csi_u_letter(tui, 'j');
        tui.wait_ms(KEY_GAP_MS);
    }
    tui.wait_pred(
        |screen| highlight_active(screen) && screen.contains(ALPHA),
        "j extends VISUAL over the first hunk (ALPHA)",
        WAIT,
    );
}

/// `V` then `s` stages only the highlighted hunk. `u` unstages that range.
///
/// Two separable hunks in `regions.txt`. Whole-file `git add` would put
/// both ALPHA and OMEGA in the index. Highlight + `s` must stage ALPHA
/// only. `u` on the staged hunk must drop ALPHA from the index and leave
/// OMEGA unstaged. Git on disk is the oracle, not the toast.
#[test]
fn pty_v_s_stages_one_hunk_u_unstages_that_range() {
    let (_root, workspace) = two_hunk_workspace();
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

    highlight_first_hunk(&mut tui);
    tui.key('s');
    tui.wait_pred(
        staged_range_toast,
        "s stages the highlighted range (VISUAL off, staged range toast)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let cached = repo_cached(&workspace);
    let unstaged = repo_unstaged(&workspace);
    assert!(
        cached.contains(ALPHA) && !cached.contains(OMEGA),
        "index must contain ALPHA only, not a whole-file stage:\ncached={cached}\nunstaged={unstaged}"
    );
    assert!(
        unstaged.contains(OMEGA) && !unstaged.contains(ALPHA),
        "worktree diff must still show OMEGA unstaged:\ncached={cached}\nunstaged={unstaged}"
    );

    tui.key('g');
    tui.wait_ms(KEY_GAP_MS);
    tui.key('g');
    tui.wait_pred(
        |screen| {
            panes_tree_unfocused_diff_focused(screen)
                && screen.contains("STAGED")
                && screen.contains(ALPHA)
                && !screen.contains("VISUAL")
        },
        "gg on the focused diff keeps STAGED ALPHA in view",
        WAIT,
    );

    tui.shift_letter('V');
    tui.wait_pred(
        |screen| highlight_active(screen) && screen.contains("STAGED"),
        "V on the staged hunk paints VISUAL",
        WAIT,
    );
    tui.wait_ms(KEY_GAP_MS);
    for _ in 0..6 {
        csi_u_letter(&mut tui, 'j');
        tui.wait_ms(KEY_GAP_MS);
    }
    tui.key('u');
    tui.wait_pred(
        unstaged_range_toast,
        "u unstages the highlighted staged range",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let cached = repo_cached(&workspace);
    let unstaged = repo_unstaged(&workspace);
    assert!(
        !cached.contains(ALPHA) && !cached.contains(OMEGA),
        "index must drop ALPHA after range unstage:\ncached={cached}"
    );
    assert!(
        unstaged.contains(ALPHA) && unstaged.contains(OMEGA),
        "both hunks must be unstaged after u:\nunstaged={unstaged}"
    );
}
