//! Daily-path ratatui e2e. TestBackend only — no TTY, no Ink harness.
//!
//! Covers the tree, file diff, multi-lane graph, search, EasyMotion, theme
//! cycle, commit drill, and hidden ignored repos. Fixtures follow the same
//! git seed style as `snapshot_contract.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use workspace_status::tui::HeadlessTui;

fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_AUTHOR_NAME", "workspace-status e2e"),
        ("GIT_AUTHOR_EMAIL", "workspace-status-e2e@example.invalid"),
        ("GIT_COMMITTER_NAME", "workspace-status e2e"),
        ("GIT_COMMITTER_EMAIL", "workspace-status-e2e@example.invalid"),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ]
}

fn git(cwd: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let out = cmd.output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn seed_repo(workspace: &Path, name: &str, branch: &str, dirty: bool) {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", branch])
        .current_dir(&repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(&repo, &["init", "-q"]);
        git(&repo, &["checkout", "-q", "-b", branch]);
    }
    git(&repo, &["config", "user.name", "workspace-status e2e"]);
    git(&repo, &["config", "user.email", "workspace-status-e2e@example.invalid"]);
    fs::write(repo.join("README.md"), format!("# {name}\n")).unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", &format!("seed {name}")]);
    if dirty {
        fs::write(repo.join("README.md"), format!("# {name}\ndirty\n")).unwrap();
    }
}

/// Merge diamond + stash spur on a non-default branch so the repo stays visible.
fn seed_merge_graph(workspace: &Path, name: &str) {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(&repo, &["init", "-q"]);
        git(&repo, &["checkout", "-q", "-b", "main"]);
    }
    git(&repo, &["config", "user.name", "workspace-status e2e"]);
    git(&repo, &["config", "user.email", "workspace-status-e2e@example.invalid"]);
    fs::write(repo.join("README.md"), "# root\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "root"]);
    git(&repo, &["checkout", "-q", "-b", "left"]);
    fs::write(repo.join("left.txt"), "left\n").unwrap();
    git(&repo, &["add", "left.txt"]);
    git(&repo, &["commit", "-q", "-m", "left"]);
    git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("right.txt"), "right\n").unwrap();
    git(&repo, &["add", "right.txt"]);
    git(&repo, &["commit", "-q", "-m", "right"]);
    git(
        &repo,
        &["merge", "--no-ff", "-m", "merge", "left"],
    );
    git(&repo, &["checkout", "-q", "-b", "feature/graph"]);
    fs::write(repo.join("wip.txt"), "stash me\n").unwrap();
    git(&repo, &["add", "wip.txt"]);
    git(&repo, &["stash", "push", "-q", "-m", "WIP on graph"]);
}

fn daily_workspace() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-daily-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    seed_repo(&workspace, "lib", "main", false);
    seed_repo(&workspace, "notes", "main", true);
    seed_merge_graph(&workspace, "merger");
    fs::write(
        workspace.join(".workspace-status-config.json"),
        "{\n  \"ignoredRepos\": [\"notes\"]\n}\n",
    )
    .unwrap();
    (root, workspace)
}

fn open(workspace: &Path) -> HeadlessTui {
    HeadlessTui::open(workspace, false)
}

fn assert_contains(frame: &str, needle: &str) {
    assert!(
        frame.contains(needle),
        "expected `{needle}` in frame:\n{frame}"
    );
}

fn assert_absent(frame: &str, needle: &str) {
    assert!(
        !frame.contains(needle),
        "did not expect `{needle}` in frame:\n{frame}"
    );
}

#[test]
fn tree_shows_dirty_and_folded_no_updates() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let frame = tui.frame();
    assert_contains(&frame, " tree");
    assert_contains(&frame, "app");
    assert_contains(&frame, "README.md");
    assert_contains(&frame, "No updates");
    assert_absent(&frame, "lib  main");
    assert_absent(&frame, "notes");

    tui.key('G');
    assert!(
        tui.cursor_label().contains("No updates"),
        "last row should be the folded group, got {}",
        tui.cursor_label()
    );
    tui.key('l');
    let opened = tui.frame();
    assert_contains(&opened, "lib");
    tui.key('h');
    let closed = tui.frame();
    assert_contains(&closed, "No updates");
    assert_absent(&closed, "lib  main");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn dirty_file_paints_diff_pane() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    assert!(
        tui.cursor_label().contains("README.md"),
        "initial cursor should be the dirty file, got {}",
        tui.cursor_label()
    );
    let frame = tui.frame();
    assert_contains(&frame, " diff");
    assert!(
        frame.contains("dirty") || frame.contains("README.md") || frame.contains("unstaged"),
        "diff pane should show the dirty file:\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn multi_lane_graph_paints_merge_and_stash_spur() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("merger");
    let frame = tui.frame();
    assert_contains(&frame, " graph");
    assert_contains(&frame, "merge");
    assert_contains(&frame, "stash@{0}");
    assert!(
        frame.contains('╮')
            || frame.contains('╭')
            || frame.contains('╯')
            || frame.contains('╰'),
        "merge / stash join elbows:\n{frame}"
    );
    assert!(
        frame.contains('◇') || frame.contains("stash"),
        "stash spur:\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn search_n_and_n_unfolds_parents() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let start = tui.frame();
    assert_absent(&start, "lib  main");

    tui.search("main");
    let first = tui.cursor_id();
    assert!(
        tui.cursor_label().contains("main"),
        "first match should mention main, got {}",
        tui.cursor_label()
    );

    tui.key('n');
    let second = tui.cursor_id();
    assert_ne!(first, second, "n should move to the next main match");
    let after_n = tui.frame();
    assert_contains(&after_n, "lib");
    assert!(
        tui.cursor_label().contains("lib"),
        "n should land on the folded lib row, got {}",
        tui.cursor_label()
    );

    tui.key('N');
    assert_eq!(
        tui.cursor_id(),
        first,
        "N should return to the previous match"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn easy_motion_hit_jumps_visible_row() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let before = tui.cursor_id();
    tui.key(';');
    let armed = tui.frame();
    assert_contains(&armed, "EasyMotion");
    assert!(
        armed.contains("a ") || armed.contains("a"),
        "viewport labels should paint:\n{armed}"
    );
    tui.key('a');
    let after = tui.frame();
    assert_absent(&after, "EasyMotion");
    assert_ne!(
        tui.cursor_id(),
        before,
        "hit on `a` should leave the dirty-file cursor"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn theme_cycle_changes_paint() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let before = tui.style_fingerprint();
    let before_frame = tui.frame();
    tui.key('T');
    let after = tui.style_fingerprint();
    let after_frame = tui.frame();
    assert_ne!(before, after, "T should change cell colours");
    assert!(
        after_frame.contains("Monokai") || after_frame != before_frame,
        "theme cycle should paint a change:\n{after_frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn drill_enter_and_esc_walk_commit_files_diff() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("merger");
    assert!(tui.right_is_graph(), "merger row should load the graph");
    tui.enter();
    tui.enter();
    let files = tui.frame();
    assert!(
        tui.right_is_files(),
        "Enter on a graph commit should open the file list:\n{files}"
    );
    assert!(
        files.contains("left.txt")
            || files.contains("right.txt")
            || files.contains("wip.txt")
            || files.contains("README.md"),
        "file list should name a commit path:\n{files}"
    );
    tui.enter();
    let diff = tui.frame();
    assert!(
        tui.right_is_diff(),
        "Enter on a commit file should open the diff:\n{diff}"
    );
    tui.esc();
    let back_files = tui.frame();
    assert!(
        tui.right_is_files(),
        "Esc should pop to the file list:\n{back_files}"
    );
    tui.esc();
    let back_graph = tui.frame();
    assert!(
        tui.right_is_graph(),
        "Esc should pop to the graph:\n{back_graph}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hidden_ignored_stay_out_until_shown() {
    let (root, workspace) = daily_workspace();
    let mut hidden = open(&workspace);
    let frame = hidden.frame();
    assert_absent(&frame, "notes  main");
    hidden.search("notes");
    assert_absent(&hidden.frame(), "notes  main");
    assert!(
        !hidden.cursor_label().contains("notes"),
        "hidden ignored must not become the cursor"
    );

    hidden.key('.');
    let shown = hidden.frame();
    assert_contains(&shown, "notes  main");

    hidden.key('.');
    assert_absent(&hidden.frame(), "notes  main");

    let mut all = HeadlessTui::open(&workspace, true);
    assert_contains(&all.frame(), "notes  main");
    let _ = fs::remove_dir_all(root);
}
