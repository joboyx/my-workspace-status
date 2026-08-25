//! Daily-path ratatui e2e. TestBackend only — no TTY.
//!
//! Covers the tree, file diff, multi-lane graph, search, theme
//! cycle, commit drill, and hidden ignored repos. Fixtures follow the same
//! git seed style as `snapshot_contract.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use workspace_status::tui::HeadlessTui;
use workspace_status_graph::UNICODE;

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

/// Long linear history so the graph list overflows and shows a scrollbar thumb.
fn seed_tall_graph(workspace: &Path, name: &str) {
    seed_repo(workspace, name, "main", false);
    let repo = workspace.join(name);
    for i in 0..30 {
        fs::write(repo.join("count.txt"), format!("{i}\n")).unwrap();
        git(&repo, &["add", "count.txt"]);
        git(&repo, &["commit", "-q", "-m", &format!("count {i}")]);
    }
    git(&repo, &["checkout", "-q", "-b", "feature/tall"]);
}

fn seed_demo_dest() -> PathBuf {
    std::env::temp_dir().join(format!(
        "ws-tui-demo-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn seed_demo_workspace(dest: &Path) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/seed-demo-workspace.sh");
    let status = Command::new("bash")
        .arg(&script)
        .arg(dest)
        .status()
        .expect("seed script runs");
    assert!(status.success(), "seed-demo-workspace.sh failed");
}

fn first_bracket_chip(line: &str) -> &str {
    let start = line.find('[').unwrap_or(0);
    let Some(rel_end) = line[start..].find(']') else {
        return "";
    };
    &line[start..=start + rel_end]
}

#[test]
fn demo_merged_head_chip_matches_footer_on_painted_row() {
    let dest = seed_demo_dest();
    seed_demo_workspace(&dest);
    assert!(
        dest.join("merger/.worktrees/recon").is_dir(),
        "seed must include a linked worktree on the current branch"
    );
    let mut tui = open(&dest);
    tui.search("merger");
    let frame = tui.frame();
    let chip = format!(
        "[{}{}feature/reconciliation]",
        UNICODE.checkout_mark, UNICODE.sync_mark
    );
    assert!(
        frame.contains(&chip),
        "demo HEAD chip must be one pair of brackets:\n{frame}"
    );
    let spacer = frame
        .lines()
        .find(|l| l.contains(&chip) && l.contains("Demo User"))
        .unwrap_or("");
    let footer = frame
        .lines()
        .find(|l| l.contains(&chip) && !l.contains("Demo User"))
        .unwrap_or("");
    assert!(
        spacer.contains(&chip),
        "commit spacer must match footer chip {chip}:\n{spacer}\n{frame}"
    );
    assert!(
        footer.contains(&chip),
        "selection footer must show {chip}:\n{footer}\n{frame}"
    );
    assert_eq!(
        first_bracket_chip(spacer),
        first_bracket_chip(footer),
        "painted spacer chip must equal footer chip\nspacer={spacer}\nfooter={footer}"
    );
    assert!(
        !spacer.contains(".worktrees"),
        "worktree path must not be a second chip on the spacer:\n{spacer}\n{frame}"
    );
    assert!(
        !spacer.contains(UNICODE.worktree),
        "worktree glyph must not prefix the footer chip:\n{spacer}"
    );
    let split_checkout = format!("[{}]", UNICODE.checkout_mark);
    let split_sync = format!("[{}]", UNICODE.sync_mark);
    assert!(
        !spacer.contains(&split_checkout) && !spacer.contains(&split_sync),
        "marks must not paint as separate chips:\n{spacer}"
    );
    let _ = fs::remove_dir_all(&dest);
}

#[test]
fn demo_narrow_graph_truncates_chip_name_footer_keeps_full_ref() {
    let dest = seed_demo_dest();
    seed_demo_workspace(&dest);
    let mut tui = open(&dest);
    tui.search("merger");
    tui.tab();
    tui.resize(64, 28);
    let frame = tui.frame();
    let full = "feature/reconciliation";
    let footer = frame
        .lines()
        .find(|l| l.contains(full) && !l.contains("Demo User"))
        .unwrap_or("");
    assert!(
        footer.contains(full),
        "footer still lists the full ref:\n{frame}"
    );
    assert!(
        !footer.contains("[+"),
        "footer must not collapse refs to [+N]:\n{footer}\n{frame}"
    );

    let spacer = frame
        .lines()
        .find(|l| l.contains("Demo User") && l.contains('['))
        .unwrap_or("");
    assert!(
        !spacer.is_empty(),
        "expected a commit spacer with chips:\n{frame}"
    );
    let has_full_name = spacer.contains(full);
    let truncated = spacer.contains('…') && spacer.contains("…]");
    assert!(
        has_full_name || truncated,
        "spacer should keep a full or truncated chip, not drop the name:\n{spacer}\n{frame}"
    );
    if truncated && !has_full_name {
        assert!(
            !spacer.contains("[+"),
            "truncated last chip must not count toward [+N]:\n{spacer}"
        );
    }

    for _ in 0..24 {
        tui.key('l');
    }
    let panned = tui.frame();
    assert!(
        panned.contains(full),
        "h/l pan still leaves the full ref in the footer:\n{panned}"
    );
    let _ = fs::remove_dir_all(&dest);
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

fn assert_help_version(frame: &str) {
    let version = workspace_status::APP_VERSION;
    assert_contains(frame, version);
    let line = frame
        .lines()
        .rev()
        .find(|line| line.contains(version))
        .unwrap_or_else(|| panic!("expected version {version} in:\n{frame}"));
    let idx = line.rfind(version).expect("version");
    let after = &line[idx + version.len()..];
    assert!(
        after
            .chars()
            .all(|c| c.is_whitespace() || matches!(c, '│' | '╯' | '╮' | '┘' | '┐' | '║' | '┤')),
        "package version should sit in the help overlay lower-right:\n{line}"
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
        tui.left_is_files() && (diff.contains("files") || diff.contains(" files")),
        "depth 2 puts the commit-file list on the left:\n{diff}"
    );
    assert!(
        tui.focus_is_right(),
        "Enter that drills keeps right focus:\n{diff}"
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
    if tui.focus_is_right() {
        tui.esc();
    }
    let unfocus = tui.frame();
    assert!(
        tui.right_is_diff() && !tui.focus_is_right(),
        "Esc on the right pane unfocuses without popping:\n{unfocus}"
    );
    tui.esc();
    let back_files = tui.frame();
    assert!(
        tui.right_is_files() && tui.left_is_graph(),
        "Esc on the left pane pops to commit files (graph left):\n{back_files}"
    );
    tui.esc();
    let back_graph = tui.frame();
    assert!(
        tui.right_is_graph(),
        "Esc on the left pane pops to the graph:\n{back_graph}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn left_pane_move_after_drill_updates_right_pane() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("merger");
    tui.enter();
    tui.key('j');
    tui.key('j');
    tui.enter();
    let files = tui.frame();
    assert!(
        tui.right_is_files(),
        "Enter on a graph commit should open the file list:\n{files}"
    );
    tui.esc();
    if tui.focus_is_right() {
        tui.esc();
    }
    assert!(
        tui.left_is_graph() && !tui.focus_is_right(),
        "Esc on the right pane should leave the graph on the left:\n{}",
        tui.frame()
    );
    let files_before = tui.frame();
    tui.key('j');
    let files_after = tui.frame();
    assert!(
        tui.right_is_files() && tui.left_is_graph() && !tui.focus_is_right(),
        "depth-1 j must stay on the left with files on the right:\n{files_after}"
    );
    assert_ne!(
        files_before, files_after,
        "depth-1 j must reload the right pane for the next graph row:\nBEFORE:\n{files_before}\nAFTER:\n{files_after}"
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
    assert_help_version(&help_wide);
    let help_wide_list = tui.pane_tree_height();
    tui.resize(80, 48);
    assert!(
        tui.pane_tree_height() < help_wide_list,
        "wrapped help should steal pane rows: {} vs {help_wide_list}",
        tui.pane_tree_height()
    );
    let help_narrow = tui.frame();
    assert_contains(&help_narrow, "MOVE");
    assert_help_version(&help_narrow);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn help_overlay_shows_app_version() {
    let dest = seed_demo_dest();
    seed_demo_workspace(&dest);
    let mut tui = open(&dest);
    tui.resize(160, 40);
    tui.key('?');
    let help = tui.frame();
    assert_contains(&help, "MOVE");
    assert_contains(&help, "GIT");
    assert_contains(&help, "VIEW");
    assert_contains(&help, "/ search help");
    assert_contains(&help, "down / up");
    assert_contains(&help, "search focused pane");
    assert_help_version(&help);

    tui.key('/');
    tui.key('q');
    tui.key('u');
    tui.key('i');
    tui.key('t');
    let searching = tui.frame();
    assert_contains(&searching, "Esc clears search");
    assert_contains(&searching, "stage scope");
    assert_help_version(&searching);
    let _ = fs::remove_dir_all(&dest);
}

#[test]
fn first_ctrl_c_prompts_second_quits() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.ctrl_c();
    let prompted = tui.frame();
    assert_contains(&prompted, "Press Ctrl+C again to exit");
    assert!(!tui.did_quit(), "a single Ctrl-C must not quit");
    tui.ctrl_c();
    assert!(
        tui.did_quit(),
        "second Ctrl-C within the window should quit"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn expired_ctrl_c_arm_does_not_quit() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.ctrl_c();
    assert_contains(&tui.frame(), "Press Ctrl+C again to exit");
    tui.expire_ctrl_c();
    assert_absent(&tui.frame(), "Press Ctrl+C again to exit");
    assert!(!tui.did_quit());
    tui.ctrl_c();
    assert!(!tui.did_quit(), "a late Ctrl-C re-arms instead of quitting");
    assert_contains(&tui.frame(), "Press Ctrl+C again to exit");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q_still_quits_immediately() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.key('q');
    assert!(tui.did_quit(), "q remains an immediate quit");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn tree_shift_arrows_pan_long_paths_and_h_still_folds() {
    let (root, workspace) = daily_workspace();
    let long_dir = workspace.join("app/deep/nested/unique-dir-name");
    fs::create_dir_all(&long_dir).unwrap();
    fs::write(long_dir.join("unique-pan-tail.rs"), "fn pan() {}\n").unwrap();

    let mut tui = open(&workspace);
    tui.key('t');
    tui.resize(64, 24);
    tui.search("unique-pan-tail");
    let clipped = tui.frame();
    assert_absent(&clipped, "unique-pan-tail.rs");

    for _ in 0..40 {
        tui.shift_right();
    }
    let panned = tui.frame();
    assert_contains(&panned, "unique-pan-tail");

    tui.key('j');
    tui.key('k');
    assert!(
        tui.cursor_label().contains("unique-pan-tail")
            || tui.cursor_label().contains("deep/nested"),
        "vertical j/k should still move after a pan, cursor={}",
        tui.cursor_label()
    );

    tui.key('G');
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
fn graph_h_l_pans_long_subject_and_j_still_moves() {
    let (root, workspace) = daily_workspace();
    seed_long_subject_repo(&workspace, "longsubj");
    let mut tui = open(&workspace);
    tui.resize(80, 28);
    tui.search("longsubj");
    tui.tab();
    assert!(tui.right_is_graph(), "right pane should be the graph");
    tui.search("UNIQUE_GRAP");
    tui.esc();
    tui.resize(80, 28);
    let clipped = tui.frame();
    // Footer / list clip to the pane, so the unique tail stays off-screen
    // until pan. Use a prefix of the marker: after max pan the remaining
    // label viewport is often shorter than UNIQUE_GRAPH_TAIL itself.
    assert_absent(&clipped, "UNIQUE_GRAP");

    for _ in 0..120 {
        tui.key('l');
    }
    let panned = tui.frame();
    assert_contains(&panned, "UNIQUE_GRAP");

    tui.key('j');
    tui.key('k');
    assert!(
        tui.right_is_graph(),
        "vertical j/k should keep the graph focused"
    );
    let _ = fs::remove_dir_all(root);
}

fn seed_long_subject_repo(workspace: &Path, name: &str) {
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
    fs::write(repo.join("README.md"), "# long\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "root"]);
    git(&repo, &["checkout", "-q", "-b", "feature/long-subject"]);
    fs::write(repo.join("wip.txt"), "x\n").unwrap();
    git(&repo, &["add", "wip.txt"]);
    let subject = format!("{}UNIQUE_GRAPH_TAIL", "n".repeat(80));
    git(&repo, &["commit", "-q", "-m", &subject]);
}

#[test]
fn diff_h_l_pans_long_lines() {
    let (root, workspace) = daily_workspace();
    fs::write(
        workspace.join("app/README.md"),
        format!("# {}\n", "d".repeat(80)),
    )
    .unwrap();
    let mut tui = open(&workspace);
    tui.resize(80, 24);
    tui.tab();
    assert!(tui.right_is_diff(), "right pane should be the file diff");
    let clipped = tui.frame();
    let tail = "d".repeat(20);
    let before_has_tail = clipped.contains(&tail);
    for _ in 0..60 {
        tui.key('l');
    }
    let panned = tui.frame();
    assert!(
        panned.contains(&tail) || panned.contains("pan"),
        "diff pan should reveal a long line or show a pan offset:\n{panned}"
    );
    if !before_has_tail {
        assert_contains(&panned, &tail);
    }
    tui.key('j');
    tui.key('k');
    assert!(tui.right_is_diff());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn confirm_overlay_keeps_y_n_after_resize() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("README");
    tui.key('x');
    assert_contains(&tui.frame(), "Revert");
    let before = tui.cursor_id();
    tui.resize(100, 24);
    assert_contains(&tui.frame(), "Revert");
    tui.key('j');
    assert_eq!(
        tui.cursor_id(),
        before,
        "confirm overlay should swallow movement keys"
    );
    tui.key('n');
    assert_absent(&tui.frame(), "Revert ");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graph_scrollbar_thumb_drag_and_track_jump() {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-sb-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_tall_graph(&workspace, "history");
    let mut tui = open(&workspace);
    tui.search("history");
    assert!(
        tui.right_is_graph(),
        "focusing the tall repo should paint the graph"
    );
    tui.tab();
    assert!(tui.focus_is_right());
    let frame = tui.frame();
    let col = tui
        .graph_scrollbar_col()
        .expect("graph list should reserve a scrollbar column");
    let (track_y, track_h) = tui
        .graph_scrollbar_track()
        .expect("graph list should expose a scrollbar track");
    assert!(track_h > 2, "track height {track_h} in:\n{frame}");
    let start = tui.graph_scroll();
    tui.mouse_down(col, track_y);
    assert_eq!(
        tui.graph_scroll(),
        start,
        "thumb grab at the top of the track must not jump"
    );
    tui.mouse_drag(col, track_y + track_h.saturating_sub(1));
    let dragged = tui.graph_scroll();
    assert!(
        dragged > start,
        "thumb drag should scroll the graph, start={start} dragged={dragged}"
    );
    tui.mouse_up();
    tui.key('j');
    tui.key('k');
    tui.key('G');
    let _ = tui.frame();
    tui.key('g');
    tui.key('g');
    tui.mouse_down(col, track_y + track_h.saturating_sub(1));
    assert!(
        tui.graph_scroll() > 0,
        "track click should jump toward the bottom"
    );
    tui.mouse_up();
    let _ = fs::remove_dir_all(root);
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn seed_watch_pair(workspace: &Path) {
    seed_repo(workspace, "alpha", "main", false);
    seed_repo(workspace, "beta", "main", false);
    git(
        &workspace.join("alpha"),
        &["checkout", "-q", "-b", "feature/watch"],
    );
    git(
        &workspace.join("beta"),
        &["checkout", "-q", "-b", "feature/other"],
    );
}

#[test]
fn watch_tick_reloads_silent_head_move_and_flashes_other_repo_dirty() {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-watch-live-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_watch_pair(&workspace);
    let mut tui = open(&workspace);
    tui.search("alpha");
    assert!(
        tui.right_is_graph(),
        "alpha row should load the graph:\n{}",
        tui.frame()
    );
    let before_head = tui.graph_head().expect("graph head");
    let before_frame = tui.frame();
    assert_absent(&before_frame, "watch-head-move");

    let alpha = workspace.join("alpha");
    fs::write(alpha.join("tick.txt"), "head-move\n").unwrap();
    git(&alpha, &["add", "tick.txt"]);
    git(&alpha, &["commit", "-q", "-m", "watch-head-move"]);
    let new_head = git_stdout(&alpha, &["rev-parse", "HEAD"]);
    assert_ne!(before_head, new_head);

    tui.watch_tick();
    let after_tree = tui.frame();
    assert_contains(&after_tree, "watch-head-move");
    assert_eq!(tui.graph_head().as_deref(), Some(new_head.as_str()));
    assert_eq!(
        tui.snapshot_head("alpha").as_deref(),
        Some(new_head.as_str())
    );

    tui.tab();
    assert!(
        tui.focus_is_right(),
        "graph focus must also pick up the next watch tick:\n{}",
        tui.frame()
    );
    fs::write(alpha.join("tick.txt"), "head-move-2\n").unwrap();
    git(&alpha, &["add", "tick.txt"]);
    git(&alpha, &["commit", "-q", "-m", "watch-head-move-2"]);
    tui.watch_tick();
    let after_graph = tui.frame();
    assert_contains(&after_graph, "watch-head-move-2");

    let beta = workspace.join("beta");
    fs::write(beta.join("dirty.txt"), "flash me\n").unwrap();
    tui.watch_tick();
    let dirty_frame = tui.frame();
    assert_contains(&dirty_frame, "dirty.txt");
    assert!(
        tui.is_flashing("file:beta:dirty.txt"),
        "dirty file on the other repo must flash without r:\n{dirty_frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn watch_tick_updates_ahead_count_without_reload_key() {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-watch-ahead-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    let remote = root.join("remote.git");
    fs::create_dir_all(&workspace).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "--bare", remote.to_str().unwrap()])
        .status();
    assert!(init.map(|s| s.success()).unwrap_or(false), "bare origin");
    seed_repo(&workspace, "tracker", "main", false);
    seed_repo(&workspace, "sidecar", "main", false);
    git(
        &workspace.join("sidecar"),
        &["checkout", "-q", "-b", "feature/sidecar"],
    );
    let tracker = workspace.join("tracker");
    git(
        &tracker,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&tracker, &["push", "-u", "origin", "main", "--quiet"]);
    for i in 1..=2 {
        fs::write(tracker.join("count.txt"), format!("{i}\n")).unwrap();
        git(&tracker, &["add", "count.txt"]);
        git(&tracker, &["commit", "-q", "-m", &format!("ahead {i}")]);
    }
    let mut tui = open(&workspace);
    tui.search("tracker");
    assert_eq!(
        tui.snapshot_sync_note("tracker").as_deref(),
        Some("ahead by 2 commits")
    );
    let before = tui.frame();
    assert!(
        before.contains("ahead 2") || before.contains(&format!("{}2", UNICODE.ahead)),
        "graph/tree should show ahead 2:\n{before}"
    );

    fs::write(tracker.join("count.txt"), "3\n").unwrap();
    git(&tracker, &["add", "count.txt"]);
    git(&tracker, &["commit", "-q", "-m", "ahead 3"]);
    tui.watch_tick();
    assert_eq!(
        tui.snapshot_sync_note("tracker").as_deref(),
        Some("ahead by 3 commits")
    );
    let after = tui.frame();
    assert_contains(&after, "ahead 3");
    assert!(
        after.contains("ahead 3") || after.contains(&format!("{}3", UNICODE.ahead)),
        "watch must paint ahead 3 without r:\n{after}"
    );
    let _ = fs::remove_dir_all(root);
}
