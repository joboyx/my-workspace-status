//! Daily-path ratatui e2e. TestBackend only — no TTY.
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
        (
            "GIT_COMMITTER_EMAIL",
            "workspace-status-e2e@example.invalid",
        ),
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
    git(
        &repo,
        &[
            "config",
            "user.email",
            "workspace-status-e2e@example.invalid",
        ],
    );
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
    git(
        &repo,
        &[
            "config",
            "user.email",
            "workspace-status-e2e@example.invalid",
        ],
    );
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
    git(&repo, &["merge", "--no-ff", "-m", "merge", "left"]);
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

/// True when a tree row names this repo (branch glyph present; not the `/` query).
fn frame_has_repo_row(frame: &str, name: &str) -> bool {
    frame
        .lines()
        .any(|line| line.contains(name) && (line.contains('') || line.contains(" & ")))
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
    assert_absent(&frame, "lib");
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
    assert_absent(&closed, "lib");
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
    assert_contains(&frame, "README.md");
    assert!(
        frame.contains("inline") || frame.contains("split"),
        "diff header should name the layout:\n{frame}"
    );
    assert!(
        frame.contains("UNSTAGED") || frame.contains("STAGED") || frame.contains("NEW"),
        "diff pane should label staged/unstaged/new:\n{frame}"
    );
    assert!(
        frame.contains("dirty") || frame.contains("│"),
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
        frame.contains("feature/graph") || frame.contains("[feature/graph]"),
        "graph should show the checked-out branch:\n{frame}"
    );
    assert!(
        frame.contains("just now")
            || frame.contains("1m")
            || frame.contains("1h")
            || frame.contains("2m"),
        "graph should show a relative date on the commit spacer:\n{frame}"
    );
    assert!(
        frame.contains("workspace-stat"),
        "graph should show the commit author:\n{frame}"
    );
    assert!(
        frame.contains('╮') || frame.contains('╭') || frame.contains('╯') || frame.contains('╰'),
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
    assert_absent(&start, "lib");

    tui.search("main");
    let first = tui.cursor_id();
    assert!(
        tui.cursor_label().contains("main"),
        "first match should mention main, got {}",
        tui.cursor_label()
    );
    let armed = tui.frame();
    assert_contains(&armed, "/main");
    assert_absent(&armed, "n next");

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
    assert!(
        armed.contains("EASY") || armed.contains("EasyMotion"),
        "EasyMotion should arm a status chip:\n{armed}"
    );
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
    tui.esc(); // clear armed search so later Esc is the ViewStack ladder
    tui.enter();
    tui.key('j');
    tui.key('j');
    tui.enter();
    let files = tui.frame();
    assert!(
        tui.right_is_files(),
        "Enter on a graph commit should open the file list:\n{files}"
    );
    assert!(
        files.contains("graph")
            && (files.contains("merge") || files.contains("left") || files.contains("right")),
        "depth-1 keeps the graph on the left:\n{files}"
    );
    assert!(
        files.contains("left.txt")
            || files.contains("right.txt")
            || files.contains("wip.txt")
            || files.contains("README.md"),
        "file list should name a commit path:\n{files}"
    );
    assert!(
        files.contains("workspace-stat") || files.contains("Ada") || files.contains('·'),
        "commit-detail subtitle should include author/date meta:\n{files}"
    );
    assert!(
        files.contains('›') || files.contains("›"),
        "breadcrumb should join with › :\n{files}"
    );
    tui.enter();
    let diff = tui.frame();
    assert!(
        tui.right_is_diff(),
        "Enter on a commit file should open the diff:\n{diff}"
    );
    assert!(
        diff.contains("files"),
        "depth-2 left pane is the commit file list:\n{diff}"
    );
    assert!(
        diff.contains("left.txt")
            || diff.contains("right.txt")
            || diff.contains("wip.txt")
            || diff.contains("README.md"),
        "commit diff header should keep the file path:\n{diff}"
    );
    assert!(
        diff.contains("inline") || diff.contains("split"),
        "commit diff header should name the layout:\n{diff}"
    );
    tui.esc();
    assert!(
        tui.right_is_diff(),
        "first Esc unfocuses and stays on the commit diff:\n{}",
        tui.frame()
    );
    tui.esc();
    let back_files = tui.frame();
    assert!(
        tui.right_is_files(),
        "second Esc pops to the file list:\n{back_files}"
    );
    tui.esc();
    let back_graph = tui.frame();
    assert!(
        tui.right_is_graph(),
        "third Esc pops to the graph:\n{back_graph}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn chrome_pills_breadcrumb_and_armed_search_chip() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let idle = tui.frame();
    assert_contains(&idle, "tree");
    assert!(
        idle.contains("split") || idle.contains("inline"),
        "mode pills should name the diff layout:\n{idle}"
    );
    assert_contains(&idle, "? help");
    assert!(
        idle.contains("q") || idle.contains("Tab") || idle.contains("…"),
        "extras q/Tab should appear or truncate with …:\n{idle}"
    );

    tui.search("merger");
    tui.tab();
    let graph = tui.frame();
    assert_contains(&graph, "/merger");
    assert_absent(&graph, "n next");
    assert!(
        graph.contains('›'),
        "breadcrumb should join workspace › repo:\n{graph}"
    );
    assert!(
        graph.contains("[merger]") || graph.contains("merger"),
        "breadcrumb should name the focused repo:\n{graph}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fetch_paints_running_op_progress_on_breadcrumb() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.key('f');
    let frame = tui.frame();
    assert!(
        frame.contains("Fetching 0/1…") || frame.contains("Fetching 0/"),
        "manual f should paint repo progress on the breadcrumb:\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hidden_ignored_stay_out_until_shown() {
    let (root, workspace) = daily_workspace();
    let mut hidden = open(&workspace);
    let frame = hidden.frame();
    assert!(
        !frame_has_repo_row(&frame, "notes"),
        "notes must stay out until shown:\n{frame}"
    );
    hidden.search("notes");
    assert!(
        !frame_has_repo_row(&hidden.frame(), "notes"),
        "search must not reveal hidden ignored:\n{}",
        hidden.frame()
    );
    assert!(
        !hidden.cursor_label().contains("notes"),
        "hidden ignored must not become the cursor"
    );

    hidden.key('.');
    let shown = hidden.frame();
    assert_contains(&shown, "notes");
    assert!(
        frame_has_repo_row(&shown, "notes"),
        "ignored notes should enter the tree:\n{shown}"
    );

    hidden.key('.');
    assert!(
        !frame_has_repo_row(&hidden.frame(), "notes"),
        "notes should hide again:\n{}",
        hidden.frame()
    );

    let mut all = HeadlessTui::open(&workspace, true);
    assert!(
        frame_has_repo_row(&all.frame(), "notes"),
        "-a should show notes:\n{}",
        all.frame()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn terminal_resize_relayouts_panes_gutter_help_and_lists() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.resize(200, 40);
    tui.search("merger");
    let _ = tui.frame();
    let wide_tree = tui.pane_tree_width();
    let wide_diff = tui.pane_diff_width();
    let wide_list = tui.pane_tree_height();

    tui.resize(80, 40);
    assert!(
        tui.pane_tree_width() < wide_tree,
        "tree pane should shrink: {} vs {wide_tree}",
        tui.pane_tree_width()
    );
    assert!(
        tui.pane_diff_width() < wide_diff,
        "right pane (graph gutter budget) should shrink: {} vs {wide_diff}",
        tui.pane_diff_width()
    );

    tui.resize(200, 16);
    assert!(
        tui.pane_tree_height() < wide_list,
        "list viewport should shrink: {} vs {wide_list}",
        tui.pane_tree_height()
    );

    tui.resize(200, 48);
    tui.key('?');
    let help_wide = tui.frame();
    assert_contains(&help_wide, "MOVE");
    assert_contains(&help_wide, "q");
    assert_contains(&help_wide, "Tab");
    let help_wide_list = tui.pane_tree_height();
    tui.resize(80, 48);
    assert!(
        tui.pane_tree_height() < help_wide_list,
        "wrapped help should steal pane rows: {} vs {help_wide_list}",
        tui.pane_tree_height()
    );
    assert_contains(&tui.frame(), "MOVE");
    let _ = fs::remove_dir_all(root);
}
