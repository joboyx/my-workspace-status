//! Headless TestBackend e2e. No TTY.
//!
//! Covers the tree, file diff, multi-lane graph, search, theme
//! cycle, commit drill, and hidden ignored repos. Git seeds and the tree
//! hscroll oracle are shared with `tui_tty_e2e` via `tests/common/`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use common::hscroll::{
    assert_clipped, assert_panned_to_tail, DIFF_HSCROLL_TAIL, GRAPH_HSCROLL_VISIBLE,
    TREE_HSCROLL_PREFIX,
};
use common::seed::{
    daily_workspace, focus_workspace, git, git_env, seed_long_diff_file, seed_long_path_file,
    seed_long_subject_repo, seed_merge_mark_family, seed_primary_and_linked_family,
    seed_primary_merged_family, seed_repo, seed_tall_graph,
};
use workspace_status::tui::{HeadlessTui, InputMode};
use workspace_status_graph::UNICODE;

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

fn open(workspace: &Path) -> HeadlessTui {
    HeadlessTui::open(workspace, false)
}

fn gg(tui: &mut HeadlessTui) {
    tui.key('g');
    tui.key_release('g');
    tui.key('g');
    tui.key_release('g');
}

fn seed_tall_dirty_file(workspace: &Path, name: &str) {
    let mut body = String::new();
    for i in 0..50 {
        body.push_str(&format!("tall line {i} {name}\n"));
    }
    fs::write(workspace.join("app").join(name), body).unwrap();
}

/// Two committed files that can pan and scroll, so a depth-2 switch can
/// prove the new view starts at the origin.
fn seed_two_tall_commit_files(workspace: &Path) {
    seed_repo(workspace, "scrollbox", "main", false);
    let repo = workspace.join("scrollbox");
    let mut alpha = format!("{}ALPHA_PAN_TAIL\n", "n".repeat(80));
    let mut beta = format!("{}BETA_PAN_TAIL\n", "n".repeat(80));
    for i in 0..40 {
        alpha.push_str(&format!("alpha-line-{i}\n"));
        beta.push_str(&format!("beta-line-{i}\n"));
    }
    fs::write(repo.join("alpha.rs"), alpha).unwrap();
    fs::write(repo.join("beta.rs"), beta).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "tall-pair-scroll-reset"]);
    git(&repo, &["checkout", "-q", "-b", "feature/scroll-reset"]);
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

fn title_row_has_no_focus_glyph(top: &str) -> bool {
    !top.contains("● tree")
        && !top.contains("* tree")
        && !top.contains("● diff")
        && !top.contains("* diff")
}

#[test]
fn focused_pane_titles_are_plain_names() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let frame = tui.frame();
    let top = frame.lines().next().unwrap_or("");
    assert!(
        top.contains("tree"),
        "first paint left title is tree:\n{frame}"
    );
    assert!(
        top.contains("diff"),
        "first paint right title is diff:\n{frame}"
    );
    assert!(
        title_row_has_no_focus_glyph(top),
        "first paint titles must not mark focus with a glyph:\n{frame}"
    );

    tui.tab();
    let frame = tui.frame();
    let top = frame.lines().next().unwrap_or("");
    assert!(
        top.contains("tree"),
        "after Tab left title is tree:\n{frame}"
    );
    assert!(
        top.contains("diff"),
        "after Tab right title is diff:\n{frame}"
    );
    assert!(
        title_row_has_no_focus_glyph(top),
        "after Tab titles must not mark focus with a glyph:\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unfocused_pane_body_is_not_dimmed() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    assert!(
        !tui.unfocused_pane_body_has_dim(),
        "left-focused: unfocused right body must not DIM:\n{}",
        tui.frame()
    );
    tui.tab();
    assert!(tui.focus_is_right(), "Tab focuses the right pane");
    assert!(
        !tui.unfocused_pane_body_has_dim(),
        "right-focused: unfocused left body must not DIM:\n{}",
        tui.frame()
    );
    let _ = fs::remove_dir_all(root);
}

fn tree_line_has_cursor_bar(frame: &str, needle: &str) -> bool {
    frame.lines().any(|line| {
        let left = line.split("││").next().unwrap_or(line);
        left.contains(needle) && left.contains('▌')
    })
}

fn right_line_has_cursor_bar(frame: &str, needle: &str) -> bool {
    frame.lines().any(|line| {
        let right = line.split("││").nth(1).unwrap_or("");
        right.contains(needle) && right.contains('▌')
    })
}

#[test]
fn cursor_bar_follows_focused_list_after_tab() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let frame = tui.frame();
    assert!(
        tree_line_has_cursor_bar(&frame, "README.md"),
        "left-focused tree paints the cursor bar on README.md:\n{frame}"
    );
    assert!(
        !right_line_has_cursor_bar(&frame, "UNSTAGED")
            && !right_line_has_cursor_bar(&frame, "+dirty"),
        "unfocused diff must not paint the list cursor bar:\n{frame}"
    );

    tui.tab();
    assert!(tui.focus_is_right(), "Tab focuses the right pane");
    let frame = tui.frame();
    assert!(
        !tree_line_has_cursor_bar(&frame, "README.md"),
        "unfocused tree must not paint the list cursor bar:\n{frame}"
    );
    assert!(
        right_line_has_cursor_bar(&frame, "UNSTAGED")
            || right_line_has_cursor_bar(&frame, "+dirty")
            || right_line_has_cursor_bar(&frame, "@@"),
        "focused diff must paint the list cursor bar:\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn held_nav_repeat_moves_again_and_does_not_quit() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let start = tui.cursor_id();
    tui.key('j');
    let after_press = tui.cursor_id();
    assert_ne!(after_press, start, "press j should move");
    tui.key_repeat('j');
    let after_repeat = tui.cursor_id();
    assert_ne!(
        after_repeat, after_press,
        "repeat j should move again, start={start} press={after_press} repeat={after_repeat}"
    );
    tui.key_repeat('q');
    assert!(!tui.did_quit(), "repeat q must not quit");
    tui.key_repeat('z');
    tui.key_repeat('g');
    assert!(!tui.did_quit());
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
    assert_contains(&frame, "diff");
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

/// `gg` / `G` on the focused pane: left tree (including while a file diff is
/// shown), a focused file diff, the graph, and the commit-file list (including
/// while a commit diff is shown).
#[test]
fn gg_g_jump_focused_pane_including_file_diff() {
    let (root, workspace) = daily_workspace();
    seed_tall_dirty_file(&workspace, "unique-gg-file.rs");
    let merger = workspace.join("merger");
    fs::write(merger.join("one.txt"), "one\n").unwrap();
    fs::write(merger.join("two.txt"), "two\n").unwrap();
    git(&merger, &["add", "one.txt", "two.txt"]);
    git(&merger, &["commit", "-q", "-m", "two files"]);
    let mut tui = open(&workspace);

    tui.search("unique-gg-file");
    let _ = tui.frame();
    assert!(
        tui.right_is_diff() && !tui.focus_is_right(),
        "operator path: left tree focused with the file diff shown:\n{}",
        tui.frame()
    );
    let file_id = tui.cursor_id();
    assert!(
        file_id.contains("unique-gg-file"),
        "search should land on the tall file, got {file_id}"
    );
    tui.key('G');
    assert!(
        !tui.focus_is_right(),
        "G on the left pane must not steal focus to the diff"
    );
    assert!(
        tui.cursor_label().contains("No updates"),
        "G on the left tree (diff shown) should jump to the last row, got {}",
        tui.cursor_label()
    );
    gg(&mut tui);
    assert!(
        !tui.focus_is_right(),
        "gg on the left pane must not steal focus to the right"
    );
    assert_eq!(
        tui.cursor_id(),
        "workspace",
        "gg on the left tree should jump to the first row, got {}",
        tui.cursor_id()
    );

    tui.search("unique-gg-file");
    tui.tab();
    assert!(
        tui.right_is_diff() && tui.focus_is_right(),
        "Tab should focus the file diff:\n{}",
        tui.frame()
    );
    for _ in 0..20 {
        tui.key('j');
    }
    let mid = tui.diff_scroll();
    assert!(
        mid > 0,
        "j past the midpoint on a focused tall diff should leave the top, scroll={mid} cursor={}",
        tui.diff_cursor()
    );
    tui.key('G');
    assert!(
        tui.diff_scroll() > mid,
        "G on a focused diff should jump toward the end, mid={mid} after={}",
        tui.diff_scroll()
    );
    assert!(
        tui.focus_is_right() && tui.right_is_diff(),
        "G on a focused diff must not leave the diff"
    );
    gg(&mut tui);
    assert_eq!(
        tui.diff_scroll(),
        0,
        "gg on a focused diff should jump to the start"
    );
    assert!(
        tui.focus_is_right() && tui.right_is_diff(),
        "gg on a focused diff must not leave the diff"
    );

    tui.esc();
    if tui.focus_is_right() {
        tui.esc();
    }
    assert!(
        !tui.focus_is_right() && tui.cursor_id().contains("unique-gg-file"),
        "Esc should return to the left tree on the file row:\n{}",
        tui.frame()
    );
    tui.key('j');
    assert!(
        tui.cursor_id().contains("merger") && tui.right_is_graph(),
        "j from the tall file should land on merger with the graph:\n{}",
        tui.frame()
    );
    tui.tab();
    assert!(
        tui.right_is_graph() && tui.focus_is_right(),
        "Tab should focus the graph:\n{}",
        tui.frame()
    );
    let graph_start = tui.graph_cursor();
    tui.key('G');
    assert_ne!(
        tui.graph_cursor(),
        graph_start,
        "G on the graph should leave the first row"
    );
    gg(&mut tui);
    assert_eq!(
        tui.graph_cursor(),
        0,
        "gg on the graph should jump to the first row, got {}",
        tui.graph_cursor()
    );

    tui.key('j');
    tui.enter();
    let files = tui.frame();
    assert!(
        tui.right_is_files() && tui.focus_is_right(),
        "Enter on a graph commit should open the file list:\n{files}"
    );
    let file_count = tui.commit_files_len();
    assert!(
        file_count > 1,
        "need more than one commit-file row to jump, got {file_count}:\n{files}"
    );
    tui.key('G');
    assert_eq!(
        tui.commit_files_cursor(),
        file_count - 1,
        "G on the commit-file list should jump to the last row"
    );
    gg(&mut tui);
    assert_eq!(
        tui.commit_files_cursor(),
        0,
        "gg on the commit-file list should jump to the first row"
    );

    tui.esc();
    if tui.focus_is_right() {
        tui.esc();
    }
    assert!(
        tui.left_is_graph() && !tui.focus_is_right(),
        "Esc should leave the graph on the left:\n{}",
        tui.frame()
    );
    tui.key('G');
    assert_ne!(
        tui.graph_cursor(),
        0,
        "G on a left graph (files on the right) should leave the first row"
    );
    gg(&mut tui);
    assert_eq!(
        tui.graph_cursor(),
        0,
        "gg on a left graph should jump to the first row"
    );

    tui.search("two files");
    tui.enter();
    tui.enter();
    tui.esc();
    if tui.focus_is_right() {
        tui.esc();
    }
    assert!(
        tui.left_is_files() && tui.right_is_diff() && !tui.focus_is_right(),
        "depth-2 left is the commit-file list with the commit diff shown:\n{}",
        tui.frame()
    );
    let depth2_count = tui.commit_files_len();
    assert!(
        depth2_count > 1,
        "depth-2 commit-file list should have more than one row"
    );
    tui.key('G');
    assert_eq!(
        tui.commit_files_cursor(),
        depth2_count - 1,
        "G on the left commit-file list (diff shown) should jump to the last row"
    );
    assert!(
        tui.right_is_diff() && !tui.focus_is_right(),
        "left-pane G while a commit diff is shown must keep the diff and left focus"
    );
    gg(&mut tui);
    assert_eq!(
        tui.commit_files_cursor(),
        0,
        "gg on the left commit-file list (diff shown) should jump to the first row"
    );
    assert!(
        tui.right_is_diff() && !tui.focus_is_right(),
        "left-pane gg while a commit diff is shown must keep the diff and left focus"
    );

    tui.tab();
    assert!(
        tui.right_is_diff() && tui.focus_is_right(),
        "Tab should focus the commit diff:\n{}",
        tui.frame()
    );
    tui.key('G');
    gg(&mut tui);
    assert!(
        tui.right_is_diff() && tui.focus_is_right(),
        "gg/G on a focused commit diff must stay on that diff"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn multi_lane_graph_paints_merge_and_stash_spur() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("merger");
    let frame = tui.frame();
    assert_contains(&frame, "graph");
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
fn depth_2_new_commit_file_resets_diff_viewport() {
    let (root, workspace) = daily_workspace();
    seed_two_tall_commit_files(&workspace);
    let mut tui = open(&workspace);
    tui.resize(80, 24);
    tui.search("scrollbox");
    tui.enter();
    assert!(
        tui.right_is_graph() && tui.focus_is_right(),
        "Enter on scrollbox should focus its graph:\n{}",
        tui.frame()
    );
    tui.search("tall-pair-scroll-reset");
    tui.enter();
    let files = tui.frame();
    assert!(
        tui.right_is_files() && tui.focus_is_right(),
        "Enter on the tall-pair commit should open its files:\n{files}"
    );
    assert!(
        files.contains("alpha.rs") && files.contains("beta.rs"),
        "commit should list both tall files:\n{files}"
    );
    tui.enter();
    let first = tui.frame();
    assert!(
        tui.right_is_diff() && tui.focus_is_right(),
        "Enter on the first commit file should open its diff:\n{first}"
    );
    assert_contains(&first, "alpha.rs");
    tui.key('G');
    for _ in 0..40 {
        tui.key('l');
    }
    let _ = tui.frame();
    assert!(
        tui.diff_scroll() > 0,
        "G on the first commit diff must leave the top, scroll={}",
        tui.diff_scroll()
    );
    assert!(
        tui.diff_col_offset() > 0,
        "l on the first commit diff must leave the left edge, pan={}",
        tui.diff_col_offset()
    );
    tui.esc();
    if tui.focus_is_right() {
        tui.esc();
    }
    assert!(
        tui.left_is_files() && !tui.focus_is_right(),
        "Esc should leave the commit-file list focused on the left:\n{}",
        tui.frame()
    );
    tui.key('j');
    let second = tui.frame();
    assert!(
        tui.right_is_diff() && !tui.focus_is_right(),
        "j on the left list should load the next commit diff:\n{second}"
    );
    assert_contains(&second, "beta.rs");
    assert_eq!(
        tui.diff_cursor(),
        0,
        "new commit file must drop the previous row"
    );
    assert_eq!(
        tui.diff_scroll(),
        0,
        "new commit file must start at the top"
    );
    assert_eq!(
        tui.diff_col_offset(),
        0,
        "new commit file must start at the left"
    );
    assert!(
        tui.diff_scrollbar_col().is_none(),
        "vertical bar stays hidden at the origin:\n{second}"
    );
    assert!(
        !second.contains("pan "),
        "header must not keep the previous pan:\n{second}"
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
        frame.contains("Fetched"),
        "manual f must apply fetch through the interpreter (Fetched after git, not only Fetching from dispatch):\n{frame}"
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
fn tree_mouse_hscroll_does_not_move_focused_row() {
    let (root, workspace) = daily_workspace();
    let long_dir = workspace.join("app/deep/nested/unique-dir-name");
    fs::create_dir_all(&long_dir).unwrap();
    fs::write(long_dir.join("unique-hscroll-tail.rs"), "fn pan() {}\n").unwrap();

    let mut tui = open(&workspace);
    tui.key('t');
    tui.resize(64, 24);
    tui.search("unique-hscroll-tail");
    let clipped = tui.frame();
    assert_absent(&clipped, "unique-hscroll-tail.rs");

    let focused = tui.cursor_id();
    let col = tui.tree_inner_x().saturating_add(4);
    let row = tui.tree_inner_y().saturating_add(1);
    for _ in 0..40 {
        tui.mouse_scroll_right(col, row);
    }
    assert_eq!(
        tui.cursor_id(),
        focused,
        "wheel left/right over the tree must not move the focused row"
    );
    let panned = tui.frame();
    assert_contains(&panned, "unique-hscroll-tail");
    assert!(
        tui.left_col_offset() > 0,
        "tree mouse hscroll should pan a long path"
    );

    tui.mouse_shift_scroll_down(col, row);
    assert_eq!(
        tui.cursor_id(),
        focused,
        "Shift+wheel over the tree must not move the focused row"
    );

    let before_click = tui.cursor_id();
    tui.mouse_down(col, tui.tree_inner_y());
    assert_ne!(
        tui.cursor_id(),
        before_click,
        "click still selects the row under the pointer"
    );

    let after_click = tui.cursor_id();
    tui.mouse_scroll_down(col, row);
    assert_ne!(
        tui.cursor_id(),
        after_click,
        "vertical wheel over the tree still moves the cursor"
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

/// Trackpad hscroll as TTY SGR bytes through the same decoder the live loop
/// uses (`tty::decode_sgr_mouse`, matching crossterm 0.28 `event::read`).
/// Motion-bit wheel (`CSI < 99`) is dropped, so it must not pan. Wheel-right
/// (`CSI < 67`) pans the tree without changing the focused row. Click still
/// selects; `h` / `l` still fold.
#[test]
fn tree_trackpad_sgr_hscroll_pans_without_stealing_focus() {
    let (root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);

    let mut tui = open(&workspace);
    tui.resize(64, 24);
    let start = tui.frame();
    let (readme_col, readme_row) = left_pane_cell_on(&start, &tui, "README.md");
    tui.mouse_down(readme_col, readme_row);
    assert!(
        tui.cursor_label().contains("README.md"),
        "click a short row before hscroll, cursor={}",
        tui.cursor_label()
    );
    let focused = tui.cursor_id();
    let was_diff = tui.right_is_diff();

    let clipped = tui.frame();
    assert_clipped(&left_pane(&clipped, tui.pane_right_x()));
    let (col, row) = left_pane_cell_on(&clipped, &tui, TREE_HSCROLL_PREFIX);
    assert!(
        col < tui.pane_right_x(),
        "pointer must sit in the left pane, col={col} right_x={}",
        tui.pane_right_x()
    );

    for _ in 0..40 {
        tui.mouse_sgr_motion_scroll_right(col, row);
    }
    assert_eq!(
        tui.cursor_id(),
        focused,
        "dropped SGR 99 over a long tree path must not steal the focused row"
    );
    assert_eq!(
        tui.right_is_diff(),
        was_diff,
        "dropped SGR 99 must not load a different right pane"
    );
    assert_eq!(
        tui.left_col_offset(),
        0,
        "crossterm 0.28 event::read drops SGR 99; the tree must not pan"
    );
    let ignored = tui.frame();
    let ignored_left = left_pane(&ignored, tui.pane_right_x());
    assert_clipped(&ignored_left);

    for _ in 0..40 {
        tui.mouse_sgr_scroll_right(col, row);
    }
    assert_eq!(
        tui.cursor_id(),
        focused,
        "SGR wheel right over a long tree path must not steal the focused row"
    );
    assert_eq!(
        tui.right_is_diff(),
        was_diff,
        "hscroll must not load a different right pane"
    );
    let panned = tui.frame();
    let panned_left = left_pane(&panned, tui.pane_right_x());
    assert_panned_to_tail(&panned_left);
    assert!(
        tui.left_col_offset() > 0,
        "SGR wheel right should pan a long tree path"
    );

    tui.mouse_sgr_shift_wheel_down(col, row);
    assert_eq!(
        tui.cursor_id(),
        focused,
        "SGR Shift+wheel over the tree must not move the focused row"
    );

    let before_click = tui.cursor_id();
    tui.mouse_down(col, row);
    assert_ne!(
        tui.cursor_id(),
        before_click,
        "click still selects the row under the pointer"
    );

    for _ in 0..40 {
        tui.shift_left();
    }
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

fn left_pane(frame: &str, right_x: u16) -> String {
    let width = right_x as usize;
    frame
        .lines()
        .map(|line| line.chars().take(width).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn left_pane_cell_on(
    frame: &str,
    tui: &workspace_status::tui::HeadlessTui,
    needle: &str,
) -> (u16, u16) {
    let left = left_pane(frame, tui.pane_right_x());
    for (i, line) in left.lines().enumerate() {
        if line.contains(needle) {
            return (tui.tree_inner_x().saturating_add(2), i as u16);
        }
    }
    panic!("left pane should show clipped path `{needle}`:\n{frame}");
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
    tui.search(GRAPH_HSCROLL_VISIBLE);
    tui.esc();
    tui.resize(80, 28);
    let clipped = tui.frame();
    // Footer / list clip to the pane, so the unique tail stays off-screen
    // until pan. Use a prefix of the marker: after max pan the remaining
    // label viewport is often shorter than GRAPH_HSCROLL_TAIL itself.
    assert_absent(&clipped, GRAPH_HSCROLL_VISIBLE);

    for _ in 0..120 {
        tui.key('l');
    }
    let panned = tui.frame();
    assert_contains(&panned, GRAPH_HSCROLL_VISIBLE);

    tui.key('j');
    tui.key('k');
    assert!(
        tui.right_is_graph(),
        "vertical j/k should keep the graph focused"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mouse_hscroll_pans_graph_and_shows_horizontal_bar() {
    let (root, workspace) = daily_workspace();
    seed_long_subject_repo(&workspace, "longsubj");
    let mut tui = open(&workspace);
    tui.resize(80, 28);
    tui.search("longsubj");
    assert!(tui.right_is_graph(), "right pane should be the graph");
    assert!(
        !tui.focus_is_right(),
        "search leaves keyboard focus on the tree"
    );
    let _ = tui.frame();
    assert!(
        tui.graph_hscrollbar_track().is_none(),
        "horizontal bar stays hidden at the left edge"
    );
    let col = tui.pane_right_x().saturating_add(2);
    for _ in 0..80 {
        tui.mouse_scroll_right(col, 6);
    }
    let panned = tui.frame();
    assert_contains(&panned, GRAPH_HSCROLL_VISIBLE);
    assert!(
        tui.right_col_offset() > 0,
        "mouse hscroll should pan the graph under the cursor"
    );
    assert!(
        !tui.focus_is_right(),
        "mouse hscroll must not steal tree focus"
    );
    assert!(
        tui.graph_hscrollbar_track().is_some(),
        "horizontal bar is shown once the viewport leaves the left edge"
    );
    tui.tab();
    tui.key('h');
    tui.key('h');
    assert!(
        tui.right_is_graph(),
        "keyboard h/l still pan a focused graph"
    );
    let _ = fs::remove_dir_all(root);
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

/// Long file-diff hscroll: pointer over the left pane, TTY SGR through
/// `tty::decode_sgr_mouse` (same contract as live `event::read`). Motion-bit
/// wheel (`CSI < 99`) is dropped, so it must not pan. Wheel-right (`CSI < 67`)
/// pans the painted long diff. Origin-hidden bars: h-bar after leaving the
/// left edge, v-bar after leaving the top. Keyboard `h` / `j` / `k` still
/// pan and scroll. Short tree paths still pan the tree when the diff fits
/// (see `tree_trackpad_sgr_hscroll_pans_without_stealing_focus`).
#[test]
fn left_pane_trackpad_hscroll_pans_long_diff_and_shows_scrollbars() {
    let (root, workspace) = daily_workspace();
    let marker = DIFF_HSCROLL_TAIL;
    seed_long_diff_file(&workspace, "unique-diffline.rs", marker);

    let mut tui = open(&workspace);
    tui.resize(80, 24);
    tui.search("unique-diffline");
    assert!(
        tui.right_is_diff(),
        "file row should load the long-line diff"
    );
    assert!(
        !tui.focus_is_right(),
        "search leaves keyboard focus on the tree"
    );
    let clipped = tui.frame();
    assert_absent(&clipped, marker);
    assert_eq!(tui.diff_col_offset(), 0);
    assert_eq!(tui.left_col_offset(), 0);
    assert!(
        tui.diff_hscrollbar_track().is_none(),
        "horizontal bar stays hidden at the left edge"
    );
    assert!(
        tui.diff_scrollbar_col().is_none(),
        "vertical bar stays hidden at the top"
    );

    let col = tui.tree_inner_x().saturating_add(4);
    let row = tui.tree_inner_y().saturating_add(1);
    assert!(
        col < tui.pane_right_x(),
        "pointer must sit in the left pane, col={col} right_x={}",
        tui.pane_right_x()
    );

    for _ in 0..80 {
        tui.mouse_sgr_motion_scroll_right(col, row);
    }
    assert_eq!(
        tui.diff_col_offset(),
        0,
        "crossterm 0.28 event::read drops SGR 99; the long diff must not pan"
    );
    assert_eq!(
        tui.left_col_offset(),
        0,
        "dropped SGR 99 must not pan the tree"
    );
    let ignored = tui.frame();
    assert_absent(&ignored, marker);
    assert!(
        tui.diff_hscrollbar_track().is_none(),
        "dropped SGR 99 must not reveal the horizontal bar"
    );

    for _ in 0..80 {
        tui.mouse_sgr_scroll_right(col, row);
    }
    let panned = tui.frame();
    assert_contains(&panned, marker);
    assert!(
        tui.diff_col_offset() > 0,
        "SGR wheel right over the left pane must pan the long diff"
    );
    assert_eq!(
        tui.left_col_offset(),
        0,
        "short tree paths stay unpanned when the painted diff can pan"
    );
    assert!(
        !tui.focus_is_right(),
        "trackpad hscroll must not steal tree focus"
    );
    assert!(
        tui.diff_hscrollbar_track().is_some(),
        "horizontal bar is shown once the viewport leaves the left edge"
    );
    assert!(
        tui.diff_scrollbar_col().is_none(),
        "vertical bar stays hidden while still at the top"
    );

    tui.tab();
    tui.key('h');
    tui.key('h');
    assert!(
        tui.right_is_diff(),
        "keyboard h/l still pan a focused file diff"
    );
    for _ in 0..30 {
        tui.key('j');
    }
    let _ = tui.frame();
    assert!(
        tui.diff_scroll() > 0,
        "j on a focused diff should leave the top"
    );
    assert!(
        tui.diff_scrollbar_col().is_some(),
        "vertical bar is shown once the diff leaves the top"
    );
    for _ in 0..40 {
        tui.key('k');
    }
    let _ = tui.frame();
    assert_eq!(tui.diff_scroll(), 0, "k returns to the top of the diff");
    assert!(
        tui.diff_scrollbar_col().is_none(),
        "vertical bar hides again at the top"
    );
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
    let _ = tui.frame();
    assert!(
        tui.graph_scrollbar_col().is_none(),
        "vertical graph scrollbar stays hidden at the top"
    );
    tui.key('G');
    let frame = tui.frame();
    let col = tui
        .graph_scrollbar_col()
        .expect("graph list should paint a scrollbar after leaving the top");
    let (track_y, track_h) = tui
        .graph_scrollbar_track()
        .expect("graph list should expose a scrollbar track after leaving the top");
    assert!(track_h > 2, "track height {track_h} in:\n{frame}");
    let start = tui.graph_scroll();
    assert!(start > 0, "G should leave the top of the graph");
    let thumb_y = track_y + track_h.saturating_sub(1);
    tui.mouse_down(col, thumb_y);
    assert_eq!(
        tui.graph_scroll(),
        start,
        "thumb grab at the bottom of the track must not jump"
    );
    tui.mouse_drag(col, track_y);
    let dragged = tui.graph_scroll();
    assert!(
        dragged < start,
        "thumb drag toward the top should scroll up, start={start} dragged={dragged}"
    );
    tui.mouse_up();
    tui.key('j');
    tui.key('k');
    tui.key('G');
    let _ = tui.frame();
    tui.mouse_down(col, track_y);
    assert!(
        tui.graph_scroll() < start,
        "track click toward the top should jump toward that position, now={}",
        tui.graph_scroll()
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

/// Open-vs-default mark (`ICON_OPEN_VS_DEFAULT` / nf-fa-tree).
const OPEN_VS_DEFAULT: &str = "";
/// Merged-into-default mark (`ICON_MERGED_INTO_DEFAULT` / nf-fa-check-circle).
const MERGED_INTO_DEFAULT: &str = "";
/// Nested primary checkout glyph (`ICON_BRANCH`).
const PRIMARY_CHECKOUT_GLYPH: &str = "";
/// Linked extra glyph (`ICON_LINKED_WORKTREE`).
const LINKED_WORKTREE_GLYPH: &str = "";

fn tree_pane(line: &str) -> &str {
    let rest = line.strip_prefix('│').unwrap_or(line);
    rest.split('│').next().unwrap_or(rest)
}

#[test]
fn default_main_worktree_row_omits_pinetree_linked_keeps_mark() {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-primary-pinetree-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_primary_and_linked_family(&workspace);
    fs::write(workspace.join("app/README.md"), "# app\ndirty primary\n").unwrap();
    let mut tui = open(&workspace);
    let frame = tui.frame();
    let primary = frame
        .lines()
        .map(tree_pane)
        .find(|line| line.contains(PRIMARY_CHECKOUT_GLYPH) && line.contains("feature/primary-open"))
        .unwrap_or("");
    let linked = frame
        .lines()
        .map(tree_pane)
        .find(|line| line.contains(LINKED_WORKTREE_GLYPH) && line.contains("feature/linked-open"))
        .unwrap_or("");
    assert!(
        !primary.is_empty(),
        "expected a painted primary (main) worktree row:\n{frame}"
    );
    assert!(
        !linked.is_empty(),
        "expected a painted linked worktree row:\n{frame}"
    );
    assert!(
        !primary.contains(LINKED_WORKTREE_GLYPH),
        "primary checkout must use the git/branch glyph, not the linked mark:\n{primary}\n{frame}"
    );
    assert!(
        !primary.contains(OPEN_VS_DEFAULT),
        "primary (main) worktree must not paint the open-vs-default tree:\n{primary}\n{frame}"
    );
    assert!(
        linked.contains(OPEN_VS_DEFAULT),
        "linked worktree keeps the open-vs-default mark:\n{linked}\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn default_tip_linked_worktree_paints_open_not_merged() {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-default-tip-merge-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_merge_mark_family(&workspace);
    let mut tui = open(&workspace);
    let frame = tui.frame();
    let just_created = frame
        .lines()
        .map(tree_pane)
        .find(|line| line.contains(LINKED_WORKTREE_GLYPH) && line.contains("feature/just-created"))
        .unwrap_or("");
    let landed = frame
        .lines()
        .map(tree_pane)
        .find(|line| line.contains(LINKED_WORKTREE_GLYPH) && line.contains("feature/landed"))
        .unwrap_or("");
    assert!(
        !just_created.is_empty(),
        "expected a painted default-tip linked row:\n{frame}"
    );
    assert!(
        !landed.is_empty(),
        "expected a painted merged linked row:\n{frame}"
    );
    assert!(
        just_created.contains(OPEN_VS_DEFAULT) && !just_created.contains(MERGED_INTO_DEFAULT),
        "HEAD equal to the default tip must paint open, not merged:\n{just_created}\n{frame}"
    );
    assert!(
        landed.contains(MERGED_INTO_DEFAULT) && !landed.contains(OPEN_VS_DEFAULT),
        "strict-ancestor linked row must keep the merged check:\n{landed}\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn primary_merged_checkout_paints_check_not_open() {
    let root = std::env::temp_dir().join(format!(
        "ws-tui-primary-merged-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_primary_merged_family(&workspace);
    let mut tui = open(&workspace);
    let frame = tui.frame();
    let primary = frame
        .lines()
        .map(tree_pane)
        .find(|line| {
            line.contains(PRIMARY_CHECKOUT_GLYPH) && line.contains("feature/primary-merged")
        })
        .unwrap_or("");
    let linked_merged = frame
        .lines()
        .map(tree_pane)
        .find(|line| line.contains(LINKED_WORKTREE_GLYPH) && line.contains("feature/linked-merged"))
        .unwrap_or("");
    assert!(
        !primary.is_empty(),
        "expected a painted primary checkout row:\n{frame}"
    );
    assert!(
        !linked_merged.is_empty(),
        "expected a painted linked merged row:\n{frame}"
    );
    assert!(
        primary.contains(MERGED_INTO_DEFAULT),
        "primary merged into default must paint the check:\n{primary}\n{frame}"
    );
    assert!(
        !primary.contains(OPEN_VS_DEFAULT),
        "primary checkout must not paint open-vs-default:\n{primary}\n{frame}"
    );
    assert!(
        linked_merged.contains(MERGED_INTO_DEFAULT),
        "linked extra keeps the merged check:\n{linked_merged}\n{frame}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graph_branch_focus_hides_unrelated_history_and_clears() {
    let (root, workspace) = focus_workspace();

    let mut tui = open(&workspace);
    tui.search("focusbox");
    tui.tab();
    assert!(tui.right_is_graph(), "right pane should be the graph");
    let full = tui.frame();
    assert_contains(&full, "keep-leaf-commit");
    assert_contains(&full, "noise-leaf-commit");
    assert_contains(&full, "main-leaf-commit");
    assert_contains(&full, "focus-root-commit");

    tui.resize(160, 40);
    tui.key('?');
    let help = tui.frame();
    assert_contains(&help, "MOVE");
    assert_contains(&help, "graph focus branches");
    tui.esc();

    tui.key('o');
    let overlay = tui.frame();
    assert_contains(&overlay, "Focus branches");
    assert_contains(&overlay, "feature/keep");
    assert_contains(&overlay, "topic/noise");
    assert_contains(&overlay, "Enter apply");

    for c in "keep".chars() {
        tui.key(c);
    }
    tui.enter();
    let focused = tui.frame();
    assert_contains(&focused, "keep-leaf-commit");
    assert_contains(&focused, "focus-root-commit");
    assert_absent(&focused, "noise-leaf-commit");
    assert_absent(&focused, "main-leaf-commit");

    tui.key('o');
    for c in "noise".chars() {
        tui.key(c);
    }
    tui.enter();
    let switched = tui.frame();
    assert_contains(&switched, "noise-leaf-commit");
    assert_contains(&switched, "focus-root-commit");
    assert!(
        !switched.contains("keep-leaf-commit"),
        "filter-then-Enter after a focus is on must apply the visible hit, not hidden marks:\n{switched}"
    );
    assert_absent(&switched, "main-leaf-commit");

    tui.key('O');
    let restored = tui.frame();
    assert_contains(&restored, "keep-leaf-commit");
    assert_contains(&restored, "noise-leaf-commit");
    assert_contains(&restored, "main-leaf-commit");
    assert_contains(&restored, "focus-root-commit");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn semicolon_reopens_covering_range_on_marked_line() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.tab();
    assert!(
        tui.focus_is_right() && tui.right_is_diff(),
        "Tab focuses the dirty README diff:\n{}",
        tui.frame()
    );
    tui.key('V');
    tui.key('j');
    tui.key('j');
    tui.key('j');
    assert_contains(&tui.frame(), "VISUAL");
    tui.key(';');
    let overlay = tui.frame();
    assert_contains(&overlay, "README.md:1-2");
    for c in "range-note-e2e".chars() {
        tui.key(c);
    }
    tui.enter();
    let saved = tui.frame();
    assert_contains(&saved, "comment saved");
    assert_absent(&saved, "VISUAL");
    tui.key(';');
    let reopen = tui.frame();
    assert_contains(&reopen, "README.md:1-2");
    assert_contains(&reopen, "range-note-e2e");
    for _ in 0.."range-note-e2e".len() {
        tui.backspace();
    }
    tui.enter();
    let deleted = tui.frame();
    assert_contains(&deleted, "comment deleted");
    tui.key(';');
    let empty = tui.frame();
    assert_absent(&empty, "README.md:1-2");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn watch_tick_drops_visual_so_semicolon_is_one_line() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.tab();
    tui.key('V');
    tui.key('j');
    tui.key('j');
    tui.key('j');
    assert_contains(&tui.frame(), "VISUAL");
    fs::write(
        workspace.join("app").join("README.md"),
        "# app\ndirty\nwatch-extra-line\n",
    )
    .unwrap();
    tui.watch_tick();
    let after = tui.frame();
    assert_contains(&after, "watch-extra-line");
    assert_absent(&after, "VISUAL");
    tui.key(';');
    let overlay = tui.frame();
    assert_contains(&overlay, "Comment");
    assert_absent(&overlay, "README.md:1-2");
    assert_absent(&overlay, "README.md:1-3");
    for c in "watch-span-e2e".chars() {
        tui.key(c);
    }
    tui.enter();
    let saved = tui.frame();
    assert_contains(&saved, "comment saved");
    let _ = fs::remove_dir_all(root);
}

fn unwritable_store_path(prefix: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let blocker = root.join("not-a-dir");
    fs::write(&blocker, "x").unwrap();
    (root, blocker.join("store.json"))
}

#[test]
fn semicolon_save_names_failure_when_unwritable() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let (blocker_root, path) = unwritable_store_path("ws-headless-comment-fail");
    tui.set_comment_store_path(path);
    tui.tab();
    tui.key(';');
    for c in "fail-note-e2e".chars() {
        tui.key(c);
    }
    tui.enter();
    let frame = tui.frame();
    assert_absent(&frame, "comment saved");
    assert_contains(&frame, "comment save failed");
    let _ = fs::remove_dir_all(blocker_root);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn space_reviewed_names_save_failure_when_unwritable() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    let (blocker_root, path) = unwritable_store_path("ws-headless-viewed-fail");
    tui.set_viewed_store_path(path);
    tui.key(' ');
    let frame = tui.frame();
    assert_absent(&frame, "comment saved");
    assert_contains(&frame, "viewed save failed");
    let _ = fs::remove_dir_all(blocker_root);
    let _ = fs::remove_dir_all(root);
}

fn type_palette_query(tui: &mut HeadlessTui, query: &str) {
    for c in query.chars() {
        tui.key(c);
    }
}

#[test]
fn command_palette_filter_pull_enter_closes_palette() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    gg(&mut tui);
    tui.ctrl_k();
    assert_eq!(tui.input_mode(), InputMode::CommandPalette);
    type_palette_query(&mut tui, "pull");
    tui.enter();
    assert_ne!(tui.input_mode(), InputMode::CommandPalette);
    let frame = tui.frame();
    assert_absent(&frame, "Enter run");
    assert!(
        frame.contains("nothing behind to pull") || frame.contains("Pulling"),
        "palette Pull must dispatch; frame={frame:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_palette_filter_help_enter_opens_keymap_help() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.ctrl_k();
    type_palette_query(&mut tui, "help");
    tui.enter();
    assert_ne!(tui.input_mode(), InputMode::CommandPalette);
    assert_eq!(tui.input_mode(), InputMode::Help);
    let frame = tui.frame();
    assert_contains(&frame, "MOVE");
    assert_help_version(&frame);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_palette_esc_keeps_cursor() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("README");
    let id = tui.cursor_id();
    let label = tui.cursor_label();
    tui.key(':');
    assert_eq!(tui.input_mode(), InputMode::CommandPalette);
    tui.esc();
    assert_ne!(tui.input_mode(), InputMode::CommandPalette);
    assert_eq!(tui.cursor_id(), id);
    assert_eq!(tui.cursor_label(), label);
    assert_absent(&tui.frame(), "Enter run");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_palette_disabled_pull_on_file_keeps_palette_open() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("README");
    let id = tui.cursor_id();
    let head = tui.snapshot_head("app");
    let sync = tui.snapshot_sync_note("app");
    tui.ctrl_k();
    type_palette_query(&mut tui, "pull");
    tui.enter();
    assert_eq!(tui.input_mode(), InputMode::CommandPalette);
    assert_eq!(tui.cursor_id(), id);
    assert_eq!(tui.snapshot_head("app"), head);
    assert_eq!(tui.snapshot_sync_note("app"), sync);
    assert_contains(&tui.frame(), "Enter run");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_palette_disabled_view_gates_keep_palette_open() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("README");
    let id = tui.cursor_id();
    for query in ["focus branches", "highlight"] {
        tui.ctrl_k();
        type_palette_query(&mut tui, query);
        tui.enter();
        assert_eq!(
            tui.input_mode(),
            InputMode::CommandPalette,
            "{query} must stay dimmed on a file row"
        );
        assert_eq!(tui.cursor_id(), id, "{query}");
        assert_contains(&tui.frame(), "Enter run");
        tui.esc();
    }
    tui.esc();
    gg(&mut tui);
    tui.ctrl_k();
    type_palette_query(&mut tui, "full-file");
    tui.enter();
    assert_eq!(
        tui.input_mode(),
        InputMode::CommandPalette,
        "full-file must stay dimmed when the right pane is not a file diff"
    );
    assert_contains(&tui.frame(), "Enter run");
    tui.esc();
    tui.ctrl_k();
    type_palette_query(&mut tui, "reviewed");
    tui.enter();
    assert_eq!(
        tui.input_mode(),
        InputMode::CommandPalette,
        "reviewed must stay dimmed on a workspace or repo row"
    );
    assert_contains(&tui.frame(), "Enter run");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_palette_revert_opens_boxed_confirm() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.search("README");
    tui.ctrl_k();
    type_palette_query(&mut tui, "revert");
    tui.enter();
    assert_ne!(tui.input_mode(), InputMode::CommandPalette);
    assert_eq!(tui.input_mode(), InputMode::Confirm);
    let frame = tui.frame();
    assert_contains(&frame, "Revert");
    assert_contains(&frame, "y");
    assert_contains(&frame, "n");
    assert_absent(&frame, "Enter run");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_palette_does_not_steal_daily_keys() {
    let (root, workspace) = daily_workspace();
    let mut tui = open(&workspace);
    tui.key('?');
    assert_eq!(tui.input_mode(), InputMode::Help);
    tui.key('?');
    tui.key('/');
    assert_eq!(tui.input_mode(), InputMode::SearchPrompt);
    tui.esc();
    tui.key('p');
    assert_ne!(tui.input_mode(), InputMode::CommandPalette);
    tui.key('P');
    assert_ne!(tui.input_mode(), InputMode::CommandPalette);
    let _ = fs::remove_dir_all(root);
}
