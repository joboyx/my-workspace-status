use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git, git_stdout};
use crate::support::{
    right_pane, status_line, tree_cursor_on, tree_has, tree_inactive_selection_on,
    tree_line_containing, GIT_WAIT, WAIT,
};

/// Strip label of the `app` checkout's Explorer tab.
const EXPLORER_APP: &str = "Explorer · app";
/// Strip label prefix shared by every Explorer tab.
const EXPLORER_LABEL: &str = "Explorer ·";
/// Status copy when a git write is pressed on an Explorer tab.
const READ_ONLY: &str = "Explorer tab is read-only · switch to Workspace tab";
/// Status copy for `Ctrl-w` on the Workspace tab.
const WORKSPACE_CANNOT_CLOSE: &str = "Workspace tab cannot be closed";
/// Preview copy on a folder row.
const FOLDER_HINT: &str = "folder · l / Enter opens or closes it";
/// Body of the committed, clean `src/kept.rs`.
const KEPT_BODY: &str = "pub fn kept_body() {}";
/// Committed line of `src/edit.rs` (the diff's removed side).
const EDIT_BEFORE: &str = "-pub fn edit_before() {}";
/// Worktree line of `src/edit.rs` (the diff's added side).
const EDIT_AFTER: &str = "+pub fn edit_after() {}";
/// Body of the untracked `src/fresh.rs`.
const FRESH_BODY: &str = "pub fn fresh_body() {}";
/// Ctrl-w as a C0 byte (what most terminals send without CSI-u).
const CTRL_W: u8 = 0x17;

/// Add to `app` of the daily seed: a committed `.gitignore` for
/// `node_modules/`, a clean `src/kept.rs`, a modified `src/edit.rs`, an
/// untracked `src/fresh.rs`, and an ignored `node_modules/dep/index.js`.
/// `app/README.md` stays modified from the seed.
fn seed_explorer_files(workspace: &Path) {
    let app = workspace.join("app");
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(app.join(".gitignore"), "node_modules/\n").unwrap();
    fs::write(app.join("src/kept.rs"), format!("{KEPT_BODY}\n")).unwrap();
    fs::write(app.join("src/edit.rs"), "pub fn edit_before() {}\n").unwrap();
    git(&app, &["add", ".gitignore", "src"]);
    git(&app, &["commit", "-q", "-m", "add src"]);
    fs::write(app.join("src/edit.rs"), "pub fn edit_after() {}\n").unwrap();
    fs::write(app.join("src/fresh.rs"), format!("{FRESH_BODY}\n")).unwrap();
    fs::create_dir_all(app.join("node_modules/dep")).unwrap();
    fs::write(app.join("node_modules/dep/index.js"), "ignored\n").unwrap();
}

fn tab_strip(screen: &str) -> &str {
    screen.lines().next().unwrap_or_default()
}

/// Pane title row: `┌<left title>──┐┌<right title>──┐`.
fn title_row(screen: &str) -> &str {
    screen.lines().nth(1).unwrap_or_default()
}

fn explorer_tab_count(screen: &str) -> usize {
    tab_strip(screen).matches(EXPLORER_LABEL).count()
}

/// The Workspace tab is active: its tree paints, one Explorer tab may sit
/// in the strip.
fn on_workspace(screen: &str) -> bool {
    tree_has(screen, "# workspace") && title_row(screen).starts_with("┌tree")
}

/// The single `app` Explorer tab is active: the strip names it once, the
/// tree pane is titled with the checkout, and the Workspace tree is gone.
fn on_explorer(screen: &str) -> bool {
    tab_strip(screen).contains(EXPLORER_APP)
        && explorer_tab_count(screen) == 1
        && title_row(screen).starts_with("┌app─")
        && !tree_has(screen, "# workspace")
}

/// Explorer row for `name` carries `mark` on its right.
fn row_marked(screen: &str, name: &str, mark: &str) -> bool {
    tree_line_containing(screen, name).is_some_and(|line| line.trim_end().ends_with(mark))
}

/// Explorer tree focused on `name` (bold `▌` bar, not the `▏` of an
/// unfocused pane).
fn explorer_cursor_on(screen: &str, name: &str) -> bool {
    on_explorer(screen) && tree_cursor_on(screen, name) && !tree_inactive_selection_on(screen, name)
}

/// Right pane is the worktree diff of `src/edit.rs`.
fn edit_diff_preview(screen: &str) -> bool {
    let right = right_pane(screen);
    title_row(screen).contains("┐┌diff")
        && right.contains("app/src/edit.rs")
        && right.contains("@@")
        && right.contains(EDIT_BEFORE)
        && right.contains(EDIT_AFTER)
}

/// Right pane is the read-only body of the clean `src/kept.rs`: titled by
/// the file, its text, no diff hunk.
fn kept_file_preview(screen: &str) -> bool {
    let right = right_pane(screen);
    title_row(screen).contains("kept.rs")
        && !title_row(screen).contains("┐┌diff")
        && right.contains(KEPT_BODY)
        && !right.contains("@@")
        && !right.contains("UNSTAGED")
}

fn folder_preview(screen: &str) -> bool {
    title_row(screen).contains("┐┌preview") && right_pane(screen).contains(FOLDER_HINT)
}

/// Launch on the seeded workspace with the Workspace tree cursor on the
/// first changed file, `src/edit.rs`.
fn launch(workspace: &Path) -> PtySession {
    let tui = PtySession::open(workspace);
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && explorer_tab_count(screen) == 0
                && tree_has(screen, "fresh.rs")
                && tree_has(screen, "README.md")
                && tree_cursor_on(screen, "edit.rs")
                && edit_diff_preview(screen)
        },
        "first paint: Workspace tree cursor on src/edit.rs with its diff",
        GIT_WAIT,
    );
    tui
}

/// `-` on the Workspace file row `src/edit.rs` opens `Explorer · app`
/// with that file selected, `src/` open, and its diff on the right.
fn open_explorer_on_edit(tui: &mut PtySession) {
    tui.key('-');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "edit.rs")
                && tree_line_containing(screen, "src").is_some_and(|line| line.contains("v src"))
                && edit_diff_preview(screen)
        },
        "`-` opens Explorer · app on src/edit.rs (src open, diff preview)",
        GIT_WAIT,
    );
}

/// `-` opens the Explorer on a changed file; `j` / `k`, `h` / `l`, Enter,
/// Esc, and `-` walk the tree and switch the preview between the diff of a
/// changed file, the body of a clean file, and the folder hint.
///
/// Also checks the status marks (`M`, `??`, the ASCII folder dot `*`) and
/// that the ignored `node_modules/` lists. Fail if a key moves the wrong
/// row, opens a second Explorer tab, or paints the wrong preview kind.
#[test]
fn pty_explorer_minus_opens_and_walks_tree() {
    let (_root, workspace) = daily_workspace();
    seed_explorer_files(&workspace);
    let mut tui = launch(&workspace);
    open_explorer_on_edit(&mut tui);

    let screen = tui.screen();
    assert!(
        row_marked(&screen, "edit.rs", "M")
            && row_marked(&screen, "fresh.rs", "??")
            && row_marked(&screen, "README.md", "M")
            && row_marked(&screen, "src", "*")
            && !row_marked(&screen, "kept.rs", "M")
            && !row_marked(&screen, ".gitignore", "M")
            && tree_has(&screen, "node_modules"),
        "status marks: edit.rs M, fresh.rs ??, README.md M, src *, ignored node_modules listed:\n{screen}"
    );
    assert!(
        status_line(&screen).contains("parent"),
        "Explorer hints on the status row:\n{screen}"
    );

    tui.key('j');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "fresh.rs")
                && title_row(screen).contains("┐┌diff")
                && right_pane(screen).contains(FRESH_BODY)
        },
        "`j` moves to the untracked src/fresh.rs and previews it as a diff",
        GIT_WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| explorer_cursor_on(screen, "kept.rs") && kept_file_preview(screen),
        "`j` moves to the clean src/kept.rs and previews its body",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            on_explorer(screen)
                && tree_inactive_selection_on(screen, "kept.rs")
                && !tree_cursor_on(screen, "kept.rs")
                && kept_file_preview(screen)
        },
        "Enter on a file focuses the preview (tree shows the unfocused bar)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| explorer_cursor_on(screen, "kept.rs") && kept_file_preview(screen),
        "Esc in the preview returns focus to the tree on kept.rs",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| tree_inactive_selection_on(screen, "kept.rs") && kept_file_preview(screen),
        "Enter focuses the preview again",
        WAIT,
    );
    tui.key('h');
    tui.wait_pred(
        |screen| explorer_cursor_on(screen, "kept.rs") && kept_file_preview(screen),
        "`h` in the preview returns focus to the tree, cursor still on kept.rs",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        |screen| explorer_cursor_on(screen, "fresh.rs"),
        "`k` moves back up to src/fresh.rs",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "src")
                && tree_has(screen, "edit.rs")
                && folder_preview(screen)
        },
        "`h` on a file jumps to its folder src (still open, folder hint)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "src")
                && tree_line_containing(screen, "src").is_some_and(|line| line.contains("> src"))
                && !tree_has(screen, "edit.rs")
                && !tree_has(screen, "kept.rs")
        },
        "`h` on the open folder src closes it",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "src")
                && tree_line_containing(screen, "src").is_some_and(|line| line.contains("v src"))
                && tree_has(screen, "kept.rs")
        },
        "`l` on the closed folder src opens it",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "src")
                && tree_line_containing(screen, "src").is_some_and(|line| line.contains("> src"))
                && !tree_has(screen, "kept.rs")
        },
        "Enter on the open folder src closes it",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "src")
                && tree_line_containing(screen, "src").is_some_and(|line| line.contains("v src"))
                && tree_has(screen, "kept.rs")
        },
        "Enter on the closed folder src opens it again",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "node_modules")
                && tree_line_containing(screen, "node_modules")
                    .is_some_and(|line| line.contains("> node_modules"))
        },
        "`k` moves to the ignored, closed node_modules folder",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "node_modules")
                && tree_line_containing(screen, "node_modules")
                    .is_some_and(|line| line.contains("v node_modules"))
                && tree_has(screen, "dep")
        },
        "`l` opens the ignored node_modules folder and lists dep",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| explorer_cursor_on(screen, "dep") && folder_preview(screen),
        "`j` moves to node_modules/dep",
        WAIT,
    );

    tui.key('-');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "node_modules")
                && tree_has(screen, "dep")
                && folder_preview(screen)
        },
        "`-` on the Explorer tab jumps to the parent folder node_modules",
        WAIT,
    );
    let screen = tui.screen();
    assert_eq!(
        explorer_tab_count(&screen),
        1,
        "`-` on the Explorer tab must not open a second tab:\n{screen}"
    );
}

/// `gT` / `gt` switch between Workspace and Explorer; `-` on another file of
/// the same checkout reuses the one Explorer tab and reveals that file; `s`
/// on the Explorer tab refuses with the read-only copy and stages nothing.
#[test]
fn pty_explorer_reuses_tab_switches_and_refuses_stage() {
    let (_root, workspace) = daily_workspace();
    seed_explorer_files(&workspace);
    let mut tui = launch(&workspace);
    open_explorer_on_edit(&mut tui);

    tui.key('g');
    tui.key('T');
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && explorer_tab_count(screen) == 1
                && tree_cursor_on(screen, "edit.rs")
        },
        "gT from Explorer activates Workspace (cursor kept on src/edit.rs)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| on_workspace(screen) && tree_cursor_on(screen, "fresh.rs"),
        "Workspace `j` moves to src/fresh.rs",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| on_workspace(screen) && tree_cursor_on(screen, "README.md"),
        "Workspace `j` moves to README.md",
        WAIT,
    );

    tui.key('-');
    tui.wait_pred(
        |screen| {
            explorer_cursor_on(screen, "README.md")
                && title_row(screen).contains("┐┌diff")
                && right_pane(screen).contains("app/README.md")
                && right_pane(screen).contains("+dirty")
        },
        "`-` on README.md reuses Explorer · app (one tab) and reveals README.md",
        GIT_WAIT,
    );

    tui.key('g');
    tui.key('t');
    tui.wait_pred(
        |screen| on_workspace(screen) && explorer_tab_count(screen) == 1,
        "gt from the last tab wraps to Workspace",
        WAIT,
    );
    tui.key('g');
    tui.key('t');
    tui.wait_pred(
        |screen| explorer_cursor_on(screen, "README.md"),
        "gt from Workspace activates Explorer, cursor still on README.md",
        WAIT,
    );

    tui.key('s');
    tui.wait_pred(
        |screen| on_explorer(screen) && screen.contains(READ_ONLY),
        "`s` on the Explorer tab shows the read-only refusal",
        WAIT,
    );
    let staged = git_stdout(&workspace.join("app"), &["diff", "--cached", "--name-only"]);
    assert!(
        staged.is_empty(),
        "`s` on the Explorer tab must stage nothing; staged: {staged:?}"
    );
}

/// `Ctrl-w` closes the Explorer tab; on the Workspace tab it refuses with
/// `Workspace tab cannot be closed` and the app keeps running until `q`.
#[test]
fn pty_explorer_ctrl_w_closes_tab_not_workspace() {
    let (_root, workspace) = daily_workspace();
    seed_explorer_files(&workspace);
    let mut tui = launch(&workspace);
    open_explorer_on_edit(&mut tui);

    tui.send_bytes(&[CTRL_W]);
    tui.wait_pred(
        |screen| on_workspace(screen) && explorer_tab_count(screen) == 0,
        "Ctrl-w closes the Explorer tab (strip back to Workspace only)",
        WAIT,
    );

    tui.send_bytes(&[CTRL_W]);
    tui.wait_pred(
        |screen| on_workspace(screen) && screen.contains(WORKSPACE_CANNOT_CLOSE),
        "Ctrl-w on Workspace says the tab cannot be closed",
        WAIT,
    );
    tui.assert_running("Ctrl-w on the Workspace tab must not quit");

    tui.key('q');
    tui.wait_exit(WAIT);
}
