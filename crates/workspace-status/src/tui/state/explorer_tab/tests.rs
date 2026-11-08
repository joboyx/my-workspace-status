use std::path::PathBuf;

use super::super::super::action::QuickOpenEntry;
use super::super::super::command_palette::PALETTE_COMMANDS;
use super::super::super::quick_open::QuickOpenScope;
use super::super::super::tabs::WORKSPACE_TAB_CANNOT_CLOSE;
use super::*;
use crate::snapshot::{CheckoutKind, FileChange, RepoSnapshot};
use crate::testutil::{app_state, repo_snapshot};
use crate::tui::status::StatusKind;

fn change(path: &str, unstaged: Option<&str>, untracked: bool) -> FileChange {
    FileChange {
        path: path.into(),
        staged_status: None,
        unstaged_status: unstaged.map(str::to_string),
        untracked,
        old_path: None,
    }
}

fn repo(
    name: &str,
    kind: CheckoutKind,
    primary: Option<&str>,
    changes: Vec<FileChange>,
) -> RepoSnapshot {
    RepoSnapshot {
        head: "abc".into(),
        has_unstaged: !changes.is_empty(),
        has_untracked: changes.iter().any(|change| change.untracked),
        changes,
        checkout_kind: kind,
        primary_repo: primary.map(str::to_string),
        default_tip_ref: Some("main".into()),
        ..repo_snapshot(name)
    }
}

/// `app` (with a linked worktree) and `lib`, each with changes.
fn state() -> AppState {
    app_state(&[
        repo(
            "app",
            CheckoutKind::Primary,
            None,
            vec![
                change("src/tui/app.rs", Some("M"), false),
                change("new.txt", None, true),
            ],
        ),
        repo(
            "app/.worktrees/feat",
            CheckoutKind::Linked,
            Some("app"),
            vec![change("README.md", Some("M"), false)],
        ),
        repo(
            "lib",
            CheckoutKind::Primary,
            None,
            vec![change("lib.rs", Some("M"), false)],
        ),
    ])
}

fn focus(app: &mut AppState, kind: NodeKind, repo: Option<&str>, file: Option<&str>) {
    app.cursor = app
        .rows
        .iter()
        .position(|row| {
            row.kind == kind
                && row.repo.as_deref() == repo
                && file.is_none_or(|path| {
                    row.file.as_ref().is_some_and(|change| change.path == path)
                        || row.id.ends_with(&format!(":{path}"))
                })
        })
        .unwrap_or_else(|| panic!("no {kind:?} row for {repo:?} {file:?}"));
}

fn flat(effect: Effect) -> Vec<Effect> {
    match effect {
        Effect::Batch(effects) => effects,
        Effect::None => Vec::new(),
        other => vec![other],
    }
}

fn dir_loads(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadExplorerDir { rel_dir, .. } => Some(rel_dir.clone()),
            _ => None,
        })
        .collect()
}

fn entry(name: &str, is_dir: bool, ignored: bool) -> ExplorerEntry {
    ExplorerEntry {
        name: name.into(),
        is_dir,
        ignored,
    }
}

/// The `app` checkout on disk, per folder.
fn app_listing(rel_dir: &str) -> Vec<ExplorerEntry> {
    match rel_dir {
        "" => vec![
            entry("src", true, false),
            entry("target", true, true),
            entry("new.txt", false, false),
            entry("README.md", false, false),
        ],
        "src" => vec![entry("tui", true, false), entry("lib.rs", false, false)],
        "src/tui" => vec![entry("app.rs", false, false)],
        _ => Vec::new(),
    }
}

/// Land every folder listing of `effect` (and of the listings it
/// leads to). Returns the non-listing follow-ups, in order.
fn land(app: &mut AppState, effect: Effect) -> Vec<Effect> {
    let mut out = Vec::new();
    let mut queue = flat(effect);
    while !queue.is_empty() {
        let effect = queue.remove(0);
        match effect {
            Effect::LoadExplorerDir {
                tab_id,
                req,
                rel_dir,
                ..
            } => {
                let follow = app
                    .apply_explorer_dir(tab_id, req, &rel_dir, Ok(app_listing(&rel_dir)))
                    .expect("fresh listing");
                queue.extend(flat(follow));
            }
            other => out.push(other),
        }
    }
    out
}

fn tab(app: &AppState) -> &ExplorerTab {
    app.tabs.active_explorer().expect("explorer tab")
}

fn cursor_rel(app: &AppState) -> String {
    let (rows, cursor) = app.explorer_rows().expect("explorer tab");
    rows[cursor.expect("cursor row")].rel.clone()
}

/// Open `app` at its root with every listing landed.
fn open_root(app: &mut AppState) {
    focus(app, NodeKind::Repo, Some("app"), None);
    let effect = app.dispatch(Action::ExplorerReveal);
    land(app, effect);
    assert!(app.is_explorer_tab());
}

#[test]
fn one_explorer_tab_per_checkout() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    let first = app.dispatch(Action::ExplorerReveal);
    assert_eq!(dir_loads(&flat(first)), [""], "a new tab lists its root");
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(tab(&app).checkout, "app");
    assert_eq!(app.tabs.labels(0)[1], "Explorer · app");

    app.dispatch(Action::JumpToTab(1));
    focus(
        &mut app,
        NodeKind::File,
        Some("app"),
        Some("src/tui/app.rs"),
    );
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(app.tabs.len(), 2, "a file under the primary reuses it");
    assert_eq!(app.tabs.active, 1);

    app.dispatch(Action::JumpToTab(1));
    focus(
        &mut app,
        NodeKind::Checkout,
        Some("app/.worktrees/feat"),
        None,
    );
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(app.tabs.len(), 3, "a linked worktree has its own tab");
    assert_eq!(tab(&app).checkout, "app/.worktrees/feat");
    assert_eq!(app.tabs.labels(0)[2], "Explorer · feat");

    app.dispatch(Action::JumpToTab(1));
    focus(&mut app, NodeKind::Repo, Some("lib"), None);
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(app.tabs.len(), 4);
    app.dispatch(Action::JumpToTab(1));
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(app.tabs.len(), 4, "the same row twice is one tab");
    assert_eq!(app.tabs.active, 3);
}

#[test]
fn minus_on_a_nested_file_reveals_it_root_first() {
    let mut app = state();
    focus(
        &mut app,
        NodeKind::File,
        Some("app"),
        Some("src/tui/app.rs"),
    );
    let effects = flat(app.dispatch(Action::ExplorerReveal));
    assert_eq!(dir_loads(&effects), ["", "src", "src/tui"]);
    assert!(app.is_explorer_tab());
    let follow = land(&mut app, Effect::Batch(effects));
    assert!(tab(&app).tree.is_expanded("src"));
    assert!(tab(&app).tree.is_expanded("src/tui"));
    assert_eq!(cursor_rel(&app), "src/tui/app.rs");
    let preview = follow
        .into_iter()
        .find(|effect| !matches!(effect, Effect::None))
        .expect("a preview load");
    match preview {
        Effect::LoadExplorerPreview {
            repo, path, change, ..
        } => {
            assert_eq!((repo.as_str(), path.as_str()), ("app", "src/tui/app.rs"));
            assert!(change.is_some(), "a changed file previews its diff");
        }
        other => panic!("{other:?}"),
    }
    assert!(app.explorer_diff_loading());
    assert_eq!(app.focus, FocusPane::Left, "the tree keeps focus");
    assert_eq!(
        app.quick_open_scope(),
        QuickOpenScope::Checkout("app".into()),
        "Quick Open and search in files scope to the checkout"
    );

    // A folder row (here the compact `src/tui`) opens that folder with
    // the cursor on it.
    app.dispatch(Action::JumpToTab(1));
    app.cursor = app
        .rows
        .iter()
        .position(|row| {
            row.kind == NodeKind::Dir
                && dir_path_from_id(&row.id, "app").as_deref() == Some("src/tui")
        })
        .expect("src/tui dir row");
    assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(cursor_rel(&app), "src/tui");
    assert!(tab(&app).tree.is_expanded("src/tui"));
}

/// The Open in Explorer palette row.
fn open_in_explorer_row() -> &'static PaletteCommand {
    PALETTE_COMMANDS
        .iter()
        .find(|command| command.action == Action::ExplorerReveal)
        .expect("Open in Explorer row")
}

/// Run palette row `title` from Quick Open commands mode.
fn run_palette_row(app: &mut AppState, title: &str) -> Effect {
    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
    for c in title.chars() {
        app.dispatch(Action::QuickOpenChar(c));
    }
    let palette = app.command_palette_mut().expect("palette open");
    palette.cursor = palette
        .visible()
        .iter()
        .position(|command| command.title == title)
        .unwrap_or_else(|| panic!("palette row {title}"));
    app.dispatch(Action::QuickOpenSubmit)
}

/// Off the Explorer-open rows `-` and the palette row refuse with the
/// same copy: no Explorer tab, no commit-message resize.
fn assert_minus_refuses(app: &mut AppState, want: &str, what: &str) {
    let lines = app.commit_msg_lines;
    let tabs = app.tabs.len();
    let active = app.tabs.active;
    assert_eq!(
        app.palette_disabled_reason(open_in_explorer_row())
            .as_deref(),
        Some(want),
        "palette: {what}"
    );
    app.status.clear();
    assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None, "{what}");
    assert_eq!(&*app.status, want, "status: {what}");
    assert_eq!(app.status.kind(), StatusKind::Warn, "{what}");
    assert_eq!(app.commit_msg_lines, lines, "no resize: {what}");
    assert_eq!(app.tabs.len(), tabs, "no new tab: {what}");
    assert_eq!(app.tabs.active, active, "same tab: {what}");
}

#[test]
fn minus_off_a_checkout_tree_row_refuses_like_the_palette_row() {
    let mut app = state();
    focus(&mut app, NodeKind::Workspace, None, None);
    assert_minus_refuses(&mut app, FOCUS_A_TREE_ROW_TO_EXPLORE, "workspace row");

    // The graph pane is not the tree.
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    app.focus = FocusPane::Right;
    assert_eq!(app.list_focus_target(), ListFocusTarget::Graph);
    assert_minus_refuses(&mut app, FOCUS_A_TREE_ROW_TO_EXPLORE, "graph pane");
    app.focus = FocusPane::Left;

    app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
    assert!(app.is_compare_tab());
    assert_minus_refuses(&mut app, SWITCH_TO_WORKSPACE_TAB, "compare tab");
}

/// On an Explorer tab `-` is the parent folder, a different action, so
/// the palette row is off there and never dispatches.
#[test]
fn open_in_explorer_row_is_off_on_an_explorer_tab() {
    let mut app = state();
    open_root(&mut app);
    assert_eq!(
        app.palette_disabled_reason(open_in_explorer_row())
            .as_deref(),
        Some(SWITCH_TO_WORKSPACE_TAB)
    );
    let lines = app.commit_msg_lines;
    assert_eq!(run_palette_row(&mut app, "Open in Explorer"), Effect::None);
    assert_eq!(&*app.status, SWITCH_TO_WORKSPACE_TAB);
    assert!(app.quick_open.is_some(), "a disabled row keeps the palette");
    assert_eq!(app.commit_msg_lines, lines);
    assert_eq!(app.tabs.len(), 2);
}

#[test]
fn open_in_explorer_row_opens_the_checkout_from_a_tree_row() {
    let mut app = state();
    focus(
        &mut app,
        NodeKind::Checkout,
        Some("app/.worktrees/feat"),
        None,
    );
    assert_eq!(app.palette_disabled_reason(open_in_explorer_row()), None);
    let effect = run_palette_row(&mut app, "Open in Explorer");
    assert!(app.quick_open.is_none(), "the palette closes");
    assert_eq!(dir_loads(&flat(effect)), [""], "a new tab lists its root");
    assert!(app.is_explorer_tab());
    assert_eq!(tab(&app).checkout, "app/.worktrees/feat");
    assert_eq!(app.tabs.len(), 2);
}

#[test]
fn minus_on_the_explorer_jumps_to_the_parent_folder() {
    let mut app = state();
    focus(
        &mut app,
        NodeKind::File,
        Some("app"),
        Some("src/tui/app.rs"),
    );
    let effect = app.dispatch(Action::ExplorerReveal);
    land(&mut app, effect);
    let lines = app.commit_msg_lines;
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(cursor_rel(&app), "src/tui");
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(cursor_rel(&app), "src");
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(cursor_rel(&app), "src", "a root-level row stays");
    assert_eq!(app.commit_msg_lines, lines, "no commit-message resize");
    assert_eq!(app.tabs.len(), 2, "no second Explorer tab");
}

#[test]
fn workspace_never_closes_and_close_activates_the_left_tab() {
    let mut app = state();
    app.dispatch(Action::CloseTab);
    assert_eq!(&*app.status, WORKSPACE_TAB_CANNOT_CLOSE);
    assert_eq!(app.status.kind(), StatusKind::Warn);

    app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
    app.dispatch(Action::JumpToTab(1));
    open_root(&mut app);
    assert_eq!(app.tabs.active, 2);
    app.dispatch(Action::CloseTab);
    assert_eq!(app.tabs.len(), 2);
    assert!(app.is_compare_tab(), "the left neighbour is active");
    assert!(app.tabs.find_explorer("app").is_none());
}

/// Palette reason of the first row that runs `action`.
fn palette_reason(app: &AppState, action: Action) -> Option<String> {
    let command = PALETTE_COMMANDS
        .iter()
        .find(|command| command.action == action)
        .unwrap_or_else(|| panic!("palette row for {action:?}"));
    app.palette_disabled_reason(command)
}

/// `action` refuses with `reason` on its key and on its palette row
/// (when it has one), as a warning.
fn assert_refused(app: &mut AppState, action: Action, reason: &str) {
    if PALETTE_COMMANDS
        .iter()
        .any(|command| command.action == action)
    {
        assert_eq!(
            palette_reason(app, action.clone()).as_deref(),
            Some(reason),
            "{action:?}"
        );
    }
    app.status.clear();
    assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
    assert_eq!(&*app.status, reason, "{action:?}");
    assert_eq!(app.status.kind(), StatusKind::Warn, "{action:?}");
}

#[test]
fn t_dot_m_msg_rows_and_git_writes_still_refuse() {
    let mut app = state();
    open_root(&mut app);
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    for focus in [FocusPane::Left, FocusPane::Right] {
        app.focus = focus;
        for action in [
            Action::Stage,
            Action::Unstage,
            Action::Revert,
            Action::Push,
            Action::Pull,
            Action::Fetch,
            Action::DefaultBranch,
            Action::Branch,
            Action::RemoveWorktree,
            Action::StashMenu,
            Action::GraphCheckout,
            Action::GraphMerge,
            Action::CompareVsDefault,
            Action::CompareVsBranch,
            Action::CompareCommitVsParent,
        ] {
            assert_refused(&mut app, action, EXPLORER_TAB_READ_ONLY);
        }
        for action in [
            Action::ToggleTreeMode,
            Action::ToggleShowHidden,
            Action::ToggleCommitMsgExpand,
            Action::ResizeCommitMsg(-1),
            Action::ResizeCommitMsg(1),
        ] {
            assert_refused(&mut app, action, SWITCH_TO_WORKSPACE_TAB);
        }
    }
    assert_eq!(app.tabs.len(), 2, "no compare tab opened");
}

#[test]
fn ctrl_o_refuses_and_never_toggles_full_context() {
    let mut app = state();
    app.full_context.insert("file:keep".into());
    let before = app.full_context.clone();
    app.open_loaded_inner_diff(
        "app",
        "src/tui/app.rs",
        super::super::super::diff::DiffContent::from_unified("@@ -1 +1 @@\n-a\n+b\n"),
    );
    for focus in [FocusPane::Left, FocusPane::Right] {
        app.focus = focus;
        assert_refused(
            &mut app,
            Action::ToggleFullContext,
            EXPLORER_DIFFS_SHOW_WHOLE_FILE,
        );
        assert_eq!(app.full_context, before, "{focus:?}");
    }
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    assert_refused(
        &mut app,
        Action::ToggleFullContext,
        EXPLORER_DIFFS_SHOW_WHOLE_FILE,
    );
    assert_eq!(app.full_context, before);
}

/// `state()` with the Workspace tree on `lib`'s changed `lib.rs`, then
/// `app`'s changed `src/tui/app.rs` open in its Explorer (inline
/// rows: 0 label, 1 hunk, 2 ` a` (1,1), 3 `-b` (2,-), 4 `+c` (-,2)).
fn changed_inner_state() -> AppState {
    let mut app = state();
    app.diff_mode = super::super::super::split::DiffMode::Inline;
    focus(&mut app, NodeKind::File, Some("lib"), Some("lib.rs"));
    app.open_loaded_inner_diff(
        "app",
        "src/tui/app.rs",
        super::super::super::diff::DiffContent::from_unified("@@ -1,2 +1,2 @@\n a\n-b\n+c\n"),
    );
    app
}

#[test]
fn space_marks_changed_inner_file_reviewed() {
    use super::super::super::viewed::viewed_identity;
    let mut app = changed_inner_state();
    let identity = viewed_identity("app", "src/tui/app.rs");
    for (focus, marked, status) in [
        (FocusPane::Left, true, "marked reviewed"),
        (FocusPane::Right, false, "reviewed mark cleared"),
    ] {
        app.focus = focus;
        assert_eq!(palette_reason(&app, Action::ToggleReviewed), None);
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert_eq!(
            app.viewed_store.contains_key(&identity),
            marked,
            "{focus:?}"
        );
        assert_eq!(&*app.status, status);
        assert!(
            !app.viewed_store
                .contains_key(&viewed_identity("lib", "lib.rs")),
            "never the parked Workspace row"
        );
    }
    // A clean file has no change to mark.
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    assert_refused(
        &mut app,
        Action::ToggleReviewed,
        FOCUS_A_FILE_TO_MARK_REVIEWED,
    );
    assert!(app.viewed_store.is_empty());
}

#[test]
fn e_diffs_inner_file() {
    let mut app = changed_inner_state();
    for focus in [FocusPane::Left, FocusPane::Right] {
        app.focus = focus;
        assert_eq!(palette_reason(&app, Action::ExternalDiff), None);
        assert_eq!(
            app.dispatch(Action::ExternalDiff),
            Effect::ExternalDiff {
                repo: "app".into(),
                path: "src/tui/app.rs".into(),
                kind: ExternalDiffKind::Worktree,
            },
            "{focus:?}"
        );
    }
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    assert_refused(&mut app, Action::ExternalDiff, FOCUS_A_FILE_TO_DIFF);
    // No inner file at all.
    let mut app = state();
    open_root(&mut app);
    assert!(tab(&app).files.is_empty());
    assert_refused(&mut app, Action::ExternalDiff, FOCUS_A_FILE_TO_DIFF);
    assert_refused(
        &mut app,
        Action::ToggleReviewed,
        FOCUS_A_FILE_TO_MARK_REVIEWED,
    );
}

#[test]
fn comment_and_export_target_inner_file_not_workspace_row() {
    use super::super::super::comments::{format_entity_reference, put_comment_entry, EntityRef};
    use super::super::super::keys::InputMode;
    let readme = |line, end_line| CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "main".into(),
        path: "README.md".into(),
        line,
        end_line,
    };
    let prompt_key = |app: &AppState| app.comment.as_ref().map(|prompt| prompt.key.clone());
    let mut app = state();
    focus(&mut app, NodeKind::File, Some("lib"), Some("lib.rs"));
    app.open_loaded_inner_file("app", "README.md", &["a", "b", "c"]);
    app.dispatch(Action::FocusLeft);

    // The tree has focus: `;` and `V` say to Tab over.
    for action in [Action::CommentStart, Action::DiffVisualStart] {
        assert_refused(&mut app, action, TAB_TO_THE_FILE_PANE);
    }
    assert_eq!(prompt_key(&app), None);
    assert_eq!(app.diff_visual_anchor, None);

    // `;` on the clean file: a working-tree line comment.
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::Move(1));
    assert_eq!(palette_reason(&app, Action::CommentStart), None);
    app.dispatch(Action::CommentStart);
    assert_eq!(prompt_key(&app), Some(readme(2, 2)));
    app.dispatch(Action::CommentCancel);

    // `V` `j` `;`: the highlighted lines.
    assert_eq!(palette_reason(&app, Action::DiffVisualStart), None);
    app.dispatch(Action::DiffVisualStart);
    assert_eq!(app.input_mode(), InputMode::DiffVisual);
    app.dispatch(Action::Move(1));
    assert_eq!(app.file_visual_range(), Some((1, 2)));
    // `'` in the highlight copies its span, in the diff range format.
    let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
        panic!("expected a copy");
    };
    assert_eq!(
        text,
        "kind: diff\nrepo: app\npath: README.md\nlines: 2-3\nsource: worktree\nside: new\n"
    );
    assert_eq!(app.file_visual_range(), Some((1, 2)), "the copy keeps it");
    app.dispatch(Action::CommentStart);
    assert_eq!(prompt_key(&app), Some(readme(2, 3)));
    assert_eq!(
        app.diff_visual_anchor, None,
        "the comment ends the highlight"
    );
    app.dispatch(Action::CommentCancel);

    // `y` exports the inner file's comments only, from either pane.
    app.comment_store = put_comment_entry(&app.comment_store, readme(2, 3), "readme note", false);
    let lib = CommentKey::WorktreeLine {
        repo: "lib".into(),
        branch: "main".into(),
        path: "lib.rs".into(),
        line: 1,
        end_line: 1,
    };
    app.comment_store = put_comment_entry(&app.comment_store, lib, "lib note", false);
    for focus in [FocusPane::Right, FocusPane::Left] {
        app.focus = focus;
        let Effect::CopyClipboard { text, .. } = app.dispatch(Action::ExportComments) else {
            panic!("expected an export");
        };
        assert!(text.contains("readme note"), "{text}");
        assert!(!text.contains("lib note"), "{text}");
        app.dispatch(Action::ExportCommentsCancel);
    }

    // `'` on the file pane names the inner file.
    app.focus = FocusPane::Right;
    let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
        panic!("expected a copy");
    };
    assert_eq!(
        text,
        format_entity_reference(&EntityRef::file("app", None, "README.md"))
    );

    // A changed inner file: `;` and `'` key the diff line.
    let mut app = changed_inner_state();
    app.dispatch(Action::FocusLeft);
    assert_refused(&mut app, Action::CommentStart, TAB_TO_THE_FILE_PANE);
    app.dispatch(Action::FocusRight);
    app.diff_cursor = 4;
    app.dispatch(Action::CommentStart);
    assert_eq!(
        prompt_key(&app),
        Some(CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "src/tui/app.rs".into(),
            line: 2,
            end_line: 2,
        })
    );
    app.dispatch(Action::CommentCancel);
    let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
        panic!("expected a copy");
    };
    assert!(text.starts_with("kind: diff\n"), "{text}");
    assert!(text.contains("path: src/tui/app.rs\n"), "{text}");
    assert!(text.contains("lines: 2\n"), "{text}");
    // `V` highlights diff rows, as on the Workspace.
    app.diff_cursor = 2;
    app.dispatch(Action::DiffVisualStart);
    assert_eq!(app.diff_visual_anchor, Some(2));
    app.dispatch(Action::Move(2));
    app.dispatch(Action::CommentStart);
    assert_eq!(
        prompt_key(&app).map(|key| match key {
            CommentKey::WorktreeLine { line, end_line, .. } => (line, end_line),
            other => panic!("{other:?}"),
        }),
        Some((1, 2))
    );
    // The tree names the tree row, not the Workspace row.
    app.dispatch(Action::CommentCancel);
    app.focus = FocusPane::Left;
    let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
        panic!("expected a copy");
    };
    assert!(text.contains("path: src/tui/app.rs\n"), "{text}");
    assert!(text.starts_with("kind: file\n"), "{text}");
}

/// Open `app` at its root, then open `src` (listing landed).
/// Rows: src/, src/tui/, src/lib.rs, target/, new.txt, README.md.
fn open_src(app: &mut AppState) {
    open_root(app);
    let open = app.dispatch(Action::FoldOpen);
    land(app, open);
    assert_eq!(cursor_rel(app), "src");
}

/// Type `query` into an open `/` prompt; returns the last key's effect.
fn type_query(app: &mut AppState, query: &str) -> Effect {
    query
        .chars()
        .map(|c| app.dispatch(Action::SearchChar(c)))
        .last()
        .unwrap_or(Effect::None)
}

/// The path of a preview load.
fn preview_path(effect: &Effect) -> Option<&str> {
    match effect {
        Effect::LoadExplorerPreview { path, .. } => Some(path.as_str()),
        _ => None,
    }
}

#[test]
fn slash_searches_explorer_rows_and_n_steps_with_preview() {
    let mut app = state();
    open_src(&mut app);
    assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
    assert!(app.search_mode);
    assert_eq!(app.search_target, SearchPane::ExplorerTree);
    assert_eq!(app.search_target.title(), "tree");

    // Typing steps from the `/` origin (src): the first match after it.
    let typed = type_query(&mut app, ".");
    assert_eq!(cursor_rel(&app), "src/lib.rs");
    assert_eq!(preview_path(&typed), Some("src/lib.rs"), "a hit previews");
    assert_eq!(app.search_match_position(), Some((Some(1), 3)));
    app.dispatch(Action::SearchSubmit);
    assert!(app.search_is_armed());

    let next = app.dispatch(Action::SearchNext);
    assert_eq!(cursor_rel(&app), "new.txt");
    assert_eq!(preview_path(&next), Some("new.txt"));
    app.dispatch(Action::SearchNext);
    assert_eq!(cursor_rel(&app), "README.md");
    assert_eq!(app.search_match_position(), Some((Some(3), 3)));
    app.dispatch(Action::SearchNext);
    assert_eq!(cursor_rel(&app), "src/lib.rs", "n wraps to the top");
    assert_eq!(&*app.status, super::super::SEARCH_WRAPPED_TO_TOP);
    app.dispatch(Action::SearchPrev);
    assert_eq!(cursor_rel(&app), "README.md", "N wraps to the bottom");

    // VS Code preview rules: one reusable preview tab, nothing pinned.
    let tab = tab(&app);
    assert_eq!(tab.files.len(), 1);
    assert_eq!(
        tab.active_inner().map(|inner| inner.rel.as_str()),
        Some("README.md")
    );
    assert!(!tab.files[0].pinned);
    assert_eq!(app.focus, FocusPane::Left, "the tree keeps focus");

    // A query with no listed match warns and leaves the cursor.
    app.dispatch(Action::SearchStart);
    type_query(&mut app, "app.rs");
    assert_eq!(cursor_rel(&app), "README.md", "src/tui is not listed yet");
    assert_eq!(&*app.status, super::super::SEARCH_NO_MATCH);
}

#[test]
fn esc_restores_explorer_tree_cursor() {
    let mut app = state();
    open_src(&mut app);
    app.dispatch(Action::Move(2));
    assert_eq!(cursor_rel(&app), "src/lib.rs");
    app.dispatch(Action::SearchStart);
    type_query(&mut app, "readme");
    assert_eq!(cursor_rel(&app), "README.md");
    let esc = app.dispatch(Action::SearchCancel);
    assert_eq!(cursor_rel(&app), "src/lib.rs", "Esc puts the cursor back");
    assert!(!app.search_mode && !app.search_active);
    // The search hit replaced the preview tab; the origin row previews again.
    assert_eq!(preview_path(&esc), Some("src/lib.rs"));
    let tab = tab(&app);
    assert_eq!(
        tab.active_inner().map(|inner| inner.rel.as_str()),
        Some("src/lib.rs")
    );
    assert_eq!(tab.files.len(), 1);
}

#[test]
fn esc_clears_an_armed_explorer_search() {
    let mut app = state();
    app.mouse_enabled = true;
    open_src(&mut app);
    app.dispatch(Action::SearchStart);
    type_query(&mut app, ".");
    app.dispatch(Action::SearchSubmit);
    assert!(app.search_is_armed());
    assert_eq!(cursor_rel(&app), "src/lib.rs");
    assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
    assert!(!app.search_is_armed());
    assert_eq!(&*app.status, "search cleared");
    assert_eq!(app.search_match_position(), None, "no match highlight");
    assert_eq!(cursor_rel(&app), "src/lib.rs", "the cursor stays");
    assert_eq!(app.focus, FocusPane::Left);

    // A right click clears it the same way.
    app.dispatch(Action::SearchStart);
    type_query(&mut app, "readme");
    app.dispatch(Action::SearchSubmit);
    assert!(app.search_is_armed());
    app.dispatch(Action::BackClick);
    assert!(!app.search_is_armed());
    assert_eq!(cursor_rel(&app), "README.md");

    // An armed file search: the first Esc clears it, the next one
    // gives focus back to the tree.
    app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta"]);
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::SearchStart);
    type_query(&mut app, "beta");
    app.dispatch(Action::SearchSubmit);
    assert_eq!(app.search_target, SearchPane::File);
    assert!(app.search_is_armed());
    app.dispatch(Action::NavEsc);
    assert!(!app.search_is_armed());
    assert_eq!(app.focus, FocusPane::Right);
    assert_eq!(app.tabs.active_file_view().map(|file| file.cursor), Some(1));
    app.dispatch(Action::NavEsc);
    assert_eq!(app.focus, FocusPane::Left);
}

#[test]
fn a_tree_search_step_that_stays_put_opens_no_preview_tab() {
    let mut app = state();
    open_root(&mut app);
    app.dispatch(Action::Move(3));
    assert_eq!(cursor_rel(&app), "README.md");
    app.dispatch(Action::CloseTab);
    assert!(tab(&app).files.is_empty(), "Ctrl-w closed the preview");

    app.dispatch(Action::SearchStart);
    assert_eq!(type_query(&mut app, "qqq"), Effect::None);
    assert_eq!(app.dispatch(Action::SearchSubmit), Effect::None);
    assert!(app.search_is_armed());
    assert_eq!(app.dispatch(Action::SearchNext), Effect::None);
    assert_eq!(app.dispatch(Action::SearchPrev), Effect::None);
    assert_eq!(&*app.status, super::super::SEARCH_NO_MATCH);
    assert_eq!(cursor_rel(&app), "README.md");
    assert!(tab(&app).files.is_empty(), "no match opens no preview tab");

    // A step that moves the cursor still previews.
    app.dispatch(Action::SearchStart);
    type_query(&mut app, "new");
    assert_eq!(cursor_rel(&app), "new.txt");
    assert_eq!(tab(&app).files.len(), 1);
}

#[test]
fn slash_binds_the_inner_file_pane_when_the_right_pane_has_focus() {
    let mut app = state();
    app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta", "alphabet"]);
    app.dispatch(Action::FocusRight);
    assert_eq!(app.focus, FocusPane::Right);
    app.dispatch(Action::SearchStart);
    assert_eq!(app.search_target, SearchPane::File);
    type_query(&mut app, "alpha");
    app.dispatch(Action::SearchSubmit);
    assert_eq!(app.tabs.active_file_view().map(|file| file.cursor), Some(2));

    app.dispatch(Action::FocusLeft);
    app.dispatch(Action::SearchStart);
    assert_eq!(app.search_target, SearchPane::ExplorerTree);
    app.dispatch(Action::SearchCancel);

    // A changed file shows its diff: `/` binds the diff.
    let effect = app.open_file_in_explorer("app".into(), "new.txt".into(), None, None);
    let gen = flat(effect)
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadExplorerPreview {
                gen,
                change: Some(_),
                ..
            } => Some(*gen),
            _ => None,
        })
        .expect("a diff load");
    let tab_id = tab(&app).id;
    let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::SearchStart);
    assert_eq!(app.search_target, SearchPane::Diff);
}

#[test]
fn explorer_tree_search_is_parked_per_tab_and_survives_previews() {
    let mut app = state();
    open_src(&mut app);
    app.dispatch(Action::SearchStart);
    type_query(&mut app, ".");
    app.dispatch(Action::SearchSubmit);
    // Previews change the active inner tab; the tree search stays.
    app.dispatch(Action::SearchNext);
    app.dispatch(Action::Move(1));
    assert!(app.search_is_armed());
    assert_eq!(app.search_target, SearchPane::ExplorerTree);

    // Workspace has its own search; the Explorer's comes back.
    app.dispatch(Action::JumpToTab(1));
    assert!(app.tabs.is_workspace());
    assert!(!app.search_active, "the Workspace search was never armed");
    app.dispatch(Action::JumpToTab(2));
    assert!(app.is_explorer_tab());
    assert!(app.search_is_armed());
    assert_eq!(app.search_query, ".");
    assert_eq!(app.search_target, SearchPane::ExplorerTree);
    app.dispatch(Action::SearchNext);
    assert_eq!(cursor_rel(&app), "src/lib.rs", "n wraps from README.md");

    // An in-file search still drops when the active inner file changes.
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::SearchStart);
    type_query(&mut app, "x");
    app.dispatch(Action::SearchSubmit);
    assert_eq!(app.search_target, SearchPane::File);
    assert!(app.search_active);
    app.dispatch(Action::FocusLeft);
    app.dispatch(Action::Move(2));
    assert_eq!(cursor_rel(&app), "new.txt");
    assert!(!app.search_active, "the file search cleared");
}

/// The Explorer `zz` window is the gap between the two `z` presses'
/// arrival, as on the Workspace tree.
#[test]
fn zz_window_reads_the_key_arrival_instant() {
    use crate::tui::keys::InputMode;
    use std::time::Duration;
    let mut app = state();
    open_root(&mut app);
    assert_eq!(cursor_rel(&app), "src");
    let first = crate::testutil::instant_ago(Duration::from_secs(5));
    app.begin_input(first);
    let _ = app.dispatch(Action::FoldToggle);
    app.end_input();
    assert_eq!(app.z_pending_at, Some(first), "z arms at its arrival");

    app.begin_input(first + Duration::from_millis(400));
    assert!(
        matches!(app.input_mode(), InputMode::ZPending { .. }),
        "a z that arrived 400 ms later is the second z of zz"
    );
    app.end_input();
    app.begin_input(first + Duration::from_millis(401));
    assert!(
        !matches!(app.input_mode(), InputMode::ZPending { .. }),
        "a z that arrived 401 ms later is a new single toggle"
    );
    app.end_input();
}

#[test]
fn zz_collapses_and_expands_subtree() {
    let mut app = state();
    open_root(&mut app);
    assert_eq!(cursor_rel(&app), "src");
    // First z opens src (listing landed); the second opens one level more.
    let first = app.dispatch(Action::FoldToggle);
    assert!(app.z_pending_at.is_some(), "z on a folder arms zz");
    land(&mut app, first);
    assert!(tab(&app).tree.is_expanded("src"));
    let deeper = app.dispatch(Action::FoldToggleSubtree);
    assert_eq!(dir_loads(&flat(deeper.clone())), ["src/tui"]);
    land(&mut app, deeper);
    assert!(tab(&app).tree.is_expanded("src/tui"));
    let rows = app.explorer_rows().expect("rows").0;
    assert!(rows.iter().any(|row| row.rel == "src/tui/app.rs"));

    // zz on the open folder: z closes it, the second z closes the subtree.
    app.dispatch(Action::FoldToggle);
    assert!(!tab(&app).tree.is_expanded("src"));
    assert_eq!(app.dispatch(Action::FoldToggleSubtree), Effect::None);
    assert!(!tab(&app).tree.is_expanded("src/tui"));
    assert_eq!(cursor_rel(&app), "src");

    // zz again: every cached folder opens at once, nothing to list.
    app.dispatch(Action::FoldToggle);
    assert_eq!(app.dispatch(Action::FoldToggleSubtree), Effect::None);
    assert!(tab(&app).tree.is_expanded("src/tui"));

    // On the ignored `target/`, zz lists nothing below it.
    app.dispatch(Action::FoldToggle);
    app.dispatch(Action::Move(1));
    assert_eq!(cursor_rel(&app), "target");
    let open = app.dispatch(Action::FoldToggle);
    land(&mut app, open);
    assert_eq!(app.dispatch(Action::FoldToggleSubtree), Effect::None);

    // A file row arms no zz; the right pane says z folds tree rows.
    app.dispatch(Action::Move(1));
    app.dispatch(Action::FoldToggle);
    assert!(app.z_pending_at.is_none());
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::FoldToggleSubtree);
    assert_eq!(&*app.status, super::super::Z_FOLDS_TREE_ROWS);
}

#[test]
fn angle_brackets_resize_shared_tree_fraction() {
    use super::super::super::split::pane_widths;
    let mut app = state();
    app.layout.term_cols = 140;
    let width = |app: &AppState| pane_widths(140, app.tree_fraction).tree_width;
    open_root(&mut app);
    assert_eq!(width(&app), 56);
    assert_eq!(app.dispatch(Action::ResizeTree(1)), Effect::None);
    assert_eq!(width(&app), 63);
    app.dispatch(Action::ResizeTree(-1));
    app.dispatch(Action::ResizeTree(-1));
    assert_eq!(width(&app), 49);
    app.dispatch(Action::JumpToTab(1));
    assert_eq!(width(&app), 49, "one split for every tab");
}

#[test]
fn explorer_palette_enables_search_fold_and_resize_rows() {
    let mut app = state();
    open_root(&mut app);
    let reason = |app: &AppState, action: Action| {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == action)
            .unwrap_or_else(|| panic!("palette row for {action:?}"));
        app.palette_disabled_reason(command)
    };
    for action in [
        Action::SearchStart,
        Action::FoldToggleSubtree,
        Action::ResizeTree(-1),
        Action::ResizeTree(1),
    ] {
        assert_eq!(reason(&app, action.clone()), None, "{action:?}");
    }
    assert_eq!(
        reason(&app, Action::SearchNext).as_deref(),
        Some(NO_SEARCH_ARMED)
    );
    // `t`, `.`, `M`, `+`: the row and the key give the same copy.
    for action in [
        Action::ToggleTreeMode,
        Action::ToggleShowHidden,
        Action::ToggleCommitMsgExpand,
        Action::ResizeCommitMsg(1),
    ] {
        assert_eq!(
            reason(&app, action.clone()).as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB),
            "{action:?}"
        );
        app.status.clear();
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert_eq!(&*app.status, SWITCH_TO_WORKSPACE_TAB, "{action:?}");
        assert_eq!(app.status.kind(), StatusKind::Warn, "{action:?}");
    }
    // `-` stays the parent folder.
    let open = app.dispatch(Action::FoldOpen);
    land(&mut app, open);
    app.dispatch(Action::Move(1));
    assert_eq!(cursor_rel(&app), "src/tui");
    app.dispatch(Action::ExplorerReveal);
    assert_eq!(cursor_rel(&app), "src");
}

#[test]
fn fold_keys_and_tab_move_between_tree_and_preview() {
    let mut app = state();
    open_root(&mut app);
    // Root: src/, target/, new.txt, README.md (folders first).
    assert_eq!(cursor_rel(&app), "src");
    let open = app.dispatch(Action::FoldOpen);
    assert_eq!(dir_loads(&flat(open.clone())), ["src"]);
    land(&mut app, open);
    assert!(tab(&app).tree.is_expanded("src"));
    app.dispatch(Action::FoldClose);
    assert!(
        !tab(&app).tree.is_expanded("src"),
        "h closes an open folder"
    );
    assert_eq!(
        app.dispatch(Action::NavEnter),
        Effect::None,
        "listing cached"
    );
    assert!(tab(&app).tree.is_expanded("src"), "Enter opens it again");

    app.dispatch(Action::Move(1));
    assert_eq!(cursor_rel(&app), "src/tui");
    let preview = app.dispatch(Action::Move(1));
    assert_eq!(cursor_rel(&app), "src/lib.rs");
    let Effect::LoadExplorerPreview {
        tab_id,
        gen,
        change: None,
        ..
    } = preview
    else {
        panic!("a clean file reads its body: {preview:?}");
    };
    // A read for a newer generation never started; the current one lands.
    let body = || ExplorerPreviewBody::File(FileRead::Binary);
    assert!(!app.apply_explorer_preview(tab_id, gen + 1, body()));
    assert!(app.apply_explorer_preview(tab_id, gen, body()));
    assert!(!app.apply_explorer_preview(tab_id + 9, gen, body()));
    app.dispatch(Action::FocusRight);
    assert_eq!(app.focus, FocusPane::Right, "Tab focuses the file");
    app.dispatch(Action::NavEsc);
    assert_eq!(app.focus, FocusPane::Left, "Esc goes back to the tree");
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::FoldClose);
    assert_eq!(app.focus, FocusPane::Left, "h in the preview too");
    assert_eq!(cursor_rel(&app), "src/lib.rs", "without moving the tree");
    app.dispatch(Action::FoldClose);
    assert_eq!(cursor_rel(&app), "src", "h on a file jumps to its folder");
    assert!(
        app.apply_explorer_preview(tab_id, gen, body()),
        "a folder row keeps the file tab: its read still lands"
    );
}

#[test]
fn stale_folder_listing_is_dropped() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    let Effect::LoadExplorerDir { tab_id, req, .. } = app.dispatch(Action::ExplorerReveal) else {
        panic!("a root listing");
    };
    let root =
        |app: &mut AppState, req| app.apply_explorer_dir(tab_id, req, "", Ok(app_listing("")));
    assert!(root(&mut app, req + 1).is_none(), "unknown request");
    assert!(root(&mut app, req).is_some());
    assert!(root(&mut app, req).is_none(), "already landed");

    let reload = flat(app.dispatch(Action::Refresh));
    assert_eq!(dir_loads(&reload), [""], "r re-lists the root");
    assert!(root(&mut app, req).is_none(), "the pre-r listing is stale");
    land(&mut app, Effect::Batch(reload));
    let (rows, _) = app.explorer_rows().unwrap();
    let target = rows.iter().find(|row| row.rel == "target").unwrap();
    assert!(target.ignored, "ignored entries stay listed");
}

#[test]
fn minus_relists_a_folder_whose_listing_lacks_the_file() {
    let mut app = state();
    open_root(&mut app);
    let app_repo = app
        .snapshot
        .repos
        .iter()
        .position(|repo| repo.repo == "app")
        .unwrap();
    app.snapshot.repos[app_repo]
        .changes
        .push(change("fresh.txt", None, true));
    app.rebuild_rows();
    app.dispatch(Action::JumpToTab(1));
    focus(&mut app, NodeKind::File, Some("app"), Some("fresh.txt"));
    let effects = flat(app.dispatch(Action::ExplorerReveal));
    assert_eq!(app.tabs.len(), 2, "the tab is reused");
    assert_eq!(dir_loads(&effects), [""], "the stale root lists again");
    assert_ne!(cursor_rel(&app), "fresh.txt", "not a row yet");
    let Some(Effect::LoadExplorerDir { tab_id, req, .. }) = effects.first().cloned() else {
        panic!("{effects:?}");
    };
    let mut listing = app_listing("");
    listing.push(entry("fresh.txt", false, false));
    let follow = app
        .apply_explorer_dir(tab_id, req, "", Ok(listing))
        .expect("fresh listing");
    assert_eq!(cursor_rel(&app), "fresh.txt", "the cursor lands on it");
    assert!(
        matches!(follow, Effect::LoadExplorerPreview { ref path, .. } if path == "fresh.txt"),
        "{follow:?}"
    );
}

#[test]
fn results_for_a_closed_tab_are_dropped() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    let Effect::LoadExplorerDir { tab_id, req, .. } = app.dispatch(Action::ExplorerReveal) else {
        panic!("a root listing");
    };
    app.dispatch(Action::CloseTab);
    assert!(app.tabs.is_workspace());
    assert!(
        app.apply_explorer_dir(tab_id, req, "", Ok(app_listing("")))
            .is_none(),
        "a listing for a closed tab"
    );

    open_root(&mut app);
    let preview = app.dispatch(Action::Move(3));
    let Effect::LoadExplorerPreview { tab_id, gen, .. } = preview else {
        panic!("{preview:?}");
    };
    app.dispatch(Action::CloseTab);
    assert!(
        app.tabs.get_explorer_id(tab_id).is_some(),
        "the first Ctrl-w closes the file tab"
    );
    app.dispatch(Action::CloseTab);
    assert!(app.tabs.get_explorer_id(tab_id).is_none());
    assert!(!app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(FileRead::Binary)));
}

#[test]
fn two_explorer_tabs_loading_at_once_each_get_their_own_result() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    let Effect::LoadExplorerDir {
        tab_id: app_tab,
        req: app_req,
        ..
    } = app.dispatch(Action::ExplorerReveal)
    else {
        panic!("app root listing");
    };
    app.dispatch(Action::JumpToTab(1));
    focus(&mut app, NodeKind::Repo, Some("lib"), None);
    let Effect::LoadExplorerDir {
        tab_id: lib_tab,
        req: lib_req,
        ..
    } = app.dispatch(Action::ExplorerReveal)
    else {
        panic!("lib root listing");
    };
    assert_ne!(app_tab, lib_tab);
    let lib_listing = vec![entry("lib.rs", false, false)];
    // Land them out of order. The lib tab's cursor lands on lib.rs (a
    // changed file), so its listing asks for that diff.
    let lib_follow = app
        .apply_explorer_dir(lib_tab, lib_req, "", Ok(lib_listing))
        .expect("lib listing");
    let Effect::LoadExplorerPreview {
        tab_id,
        gen: lib_gen,
        change: Some(_),
        ..
    } = lib_follow
    else {
        panic!("lib.rs diff: {lib_follow:?}");
    };
    assert_eq!(tab_id, lib_tab);
    assert!(app
        .apply_explorer_dir(app_tab, app_req, "", Ok(app_listing("")))
        .is_some());
    let names = |app: &AppState, id: u64| -> Vec<String> {
        let tab = app.tabs.get_explorer_id(id).unwrap();
        tab.tree
            .rows(&ExplorerStatus::default())
            .into_iter()
            .map(|row| row.rel)
            .collect()
    };
    assert_eq!(names(&app, lib_tab), ["lib.rs"]);
    assert_eq!(
        names(&app, app_tab),
        ["src", "target", "new.txt", "README.md"]
    );

    // The app tab previews its clean README.md.
    app.dispatch(Action::JumpToTab(2));
    let Effect::LoadExplorerPreview {
        tab_id,
        gen: app_gen,
        change: None,
        ..
    } = app.dispatch(Action::MoveToEnd)
    else {
        panic!("README.md preview");
    };
    assert_eq!(tab_id, app_tab);
    let readme = ExplorerPreviewBody::File(FileRead::Text {
        lines: vec!["app readme".into()],
        max_cols: 10,
    });
    let diff = super::super::super::diff::DiffContent::from_unified("+lib\n");
    assert!(app.apply_explorer_preview(app_tab, app_gen, readme));
    assert!(app.apply_explorer_preview(lib_tab, lib_gen, ExplorerPreviewBody::Diff(diff)));
    let view = |id: u64| {
        &app.tabs
            .get_explorer_id(id)
            .unwrap()
            .active_inner()
            .unwrap()
            .view
    };
    match view(app_tab) {
        ExplorerPreview::File(file) => assert_eq!(file.lines(), ["app readme"]),
        other => panic!("{other:?}"),
    }
    match view(lib_tab) {
        ExplorerPreview::Diff {
            rel,
            content,
            loading,
        } => {
            assert_eq!(rel, "lib.rs");
            assert_eq!(content.unstaged, "+lib\n");
            assert!(!loading);
        }
        other => panic!("{other:?}"),
    }
}

/// A status load that reports a new untracked file under an open,
/// listed folder lists that folder again once; the row then shows
/// `??`. An unchanged status sends nothing, in flight or after.
#[test]
fn a_status_path_missing_from_an_open_listing_relists_its_folder_once() {
    let mut app = state();
    app.snapshot.repos[0]
        .changes
        .push(change("brand-new.txt", None, true));
    assert_eq!(
        app.explorer_status_relist(&app.snapshot.repos[0].repo.clone()),
        None,
        "no Explorer tab: nothing to send"
    );
    app.snapshot.repos[0]
        .changes
        .retain(|change| change.path != "brand-new.txt");
    open_root(&mut app);
    let open = app.dispatch(Action::FoldOpen);
    land(&mut app, open);
    assert!(tab(&app).tree.is_expanded("src"));
    let status_load = |app: &mut AppState, changes: Vec<FileChange>| {
        let snap = repo("app", CheckoutKind::Primary, None, changes);
        crate::tui::app::apply_one_repo_snapshot(app, "app", Some(snap));
        app.explorer_status_relist("app")
    };
    let base = vec![
        change("src/tui/app.rs", Some("M"), false),
        change("new.txt", None, true),
    ];
    assert_eq!(status_load(&mut app, base.clone()), None, "nothing new");

    let mut with_new = base.clone();
    with_new.push(change("src/fresh.rs", None, true));
    with_new.push(change("src/other.rs", None, true));
    let Some(Effect::LoadExplorerDir {
        tab_id,
        req,
        rel_dir,
        ..
    }) = status_load(&mut app, with_new.clone())
    else {
        panic!("a re-list of src");
    };
    assert_eq!(rel_dir, "src", "two new files, one re-list");
    assert_eq!(tab(&app).status_relisted.len(), 1, "one path marked");
    assert_eq!(
        status_load(&mut app, with_new.clone()),
        None,
        "in flight: not asked twice"
    );
    let mut listing = app_listing("src");
    listing.push(entry("fresh.rs", false, false));
    listing.push(entry("other.rs", false, false));
    app.apply_explorer_dir(tab_id, req, "src", Ok(listing))
        .expect("fresh listing");
    assert_eq!(status_load(&mut app, with_new), None, "landed: no churn");
    let (rows, _) = app.explorer_rows().unwrap();
    assert!(rows.iter().any(|row| row.rel == "src/fresh.rs"));
    let status = app.explorer_status("app");
    assert!(
        status.change("src/fresh.rs").is_some_and(|c| c.untracked),
        "paints ??"
    );
    assert_eq!(cursor_rel(&app), "src", "the cursor keeps its row");

    // A new untracked folder at the root re-lists the root; a deleted
    // path never re-lists.
    let mut more = base;
    more.push(change("newdir/", None, true));
    more.push(change("gone.txt", Some("D"), false));
    let relist = flat(status_load(&mut app, more).expect("a root re-list"));
    assert_eq!(dir_loads(&relist), [""]);
}

/// `(rel, pinned)` of each inner file tab, in paint order.
fn inners(app: &AppState) -> Vec<(String, bool)> {
    tab(app)
        .files
        .iter()
        .map(|file| (file.rel.clone(), file.pinned))
        .collect()
}

fn active_rel(app: &AppState) -> Option<String> {
    tab(app).active_inner().map(|file| file.rel.clone())
}

fn pinned(rel: &str) -> (String, bool) {
    (rel.into(), true)
}

fn preview(rel: &str) -> (String, bool) {
    (rel.into(), false)
}

/// `(tab_id, gen, path, changed)` of an inner tab load.
fn load_of(effect: &Effect) -> (u64, u64, String, bool) {
    match effect {
        Effect::LoadExplorerPreview {
            tab_id,
            gen,
            path,
            change,
            ..
        } => (*tab_id, *gen, path.clone(), change.is_some()),
        other => panic!("an inner tab load, got {other:?}"),
    }
}

/// Root rows: src/, target/, new.txt (untracked), README.md (clean).
#[test]
fn tree_move_replaces_the_one_preview_tab() {
    let mut app = state();
    open_root(&mut app);
    assert!(inners(&app).is_empty(), "a folder row opens no file tab");
    let (_, _, path, changed) = load_of(&app.dispatch(Action::Move(2)));
    assert_eq!((path.as_str(), changed), ("new.txt", true));
    assert_eq!(inners(&app), [preview("new.txt")]);
    let (_, _, path, changed) = load_of(&app.dispatch(Action::Move(1)));
    assert_eq!((path.as_str(), changed), ("README.md", false));
    assert_eq!(inners(&app), [preview("README.md")], "replaced in place");
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert_eq!(app.tabs.len(), 2, "no top-level file tab");
    assert_eq!(app.focus, FocusPane::Left);
}

#[test]
fn stale_gen_of_a_replaced_preview_is_dropped() {
    let mut app = state();
    open_root(&mut app);
    let (tab_id, old, ..) = load_of(&app.dispatch(Action::Move(2)));
    let (_, new, ..) = load_of(&app.dispatch(Action::Move(1)));
    assert!(new > old);
    let diff = super::super::super::diff::DiffContent::from_unified("+new\n");
    assert!(
        !app.apply_explorer_preview(tab_id, old, ExplorerPreviewBody::Diff(diff)),
        "the replaced preview's load is stale"
    );
    let body = ExplorerPreviewBody::File(FileRead::Text {
        lines: vec!["readme".into()],
        max_cols: 6,
    });
    assert!(app.apply_explorer_preview(tab_id, new, body));
    assert_eq!(
        app.tabs
            .active_file_view()
            .map(|file| file.lines().to_vec()),
        Some(vec!["readme".to_string()])
    );
}

#[test]
fn enter_pins_and_focus_stays_on_tree() {
    let mut app = state();
    open_root(&mut app);
    app.dispatch(Action::Move(2));
    assert_eq!(
        app.dispatch(Action::NavEnter),
        Effect::None,
        "the previewed file is already loading"
    );
    assert_eq!(inners(&app), [pinned("new.txt")]);
    assert_eq!(app.focus, FocusPane::Left, "Enter keeps the tree focused");
    let (_, _, path, _) = load_of(&app.dispatch(Action::Move(1)));
    assert_eq!(path, "README.md");
    assert_eq!(
        inners(&app),
        [pinned("new.txt"), preview("README.md")],
        "a pinned tab is kept; the new preview opens after it"
    );
    app.dispatch(Action::NavEnter);
    assert_eq!(inners(&app), [pinned("new.txt"), pinned("README.md")]);
    assert_eq!(app.focus, FocusPane::Left);
    app.dispatch(Action::FocusRight);
    assert_eq!(app.focus, FocusPane::Right, "Tab still focuses the file");
}

#[test]
fn move_onto_a_pinned_file_shows_it_without_a_duplicate_preview() {
    let mut app = state();
    open_root(&mut app);
    app.dispatch(Action::Move(2));
    app.dispatch(Action::NavEnter);
    load_of(&app.dispatch(Action::Move(1)));
    assert_eq!(
        app.dispatch(Action::Move(-1)),
        Effect::None,
        "new.txt is a tab: no reload"
    );
    assert_eq!(inners(&app), [pinned("new.txt"), preview("README.md")]);
    assert_eq!(active_rel(&app).as_deref(), Some("new.txt"));
    assert_eq!(
        app.dispatch(Action::Move(1)),
        Effect::None,
        "the preview tab of README.md shows again"
    );
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert_eq!(inners(&app).len(), 2);
}

#[test]
fn folder_row_keeps_the_preview() {
    let mut app = state();
    open_root(&mut app);
    let (tab_id, gen, ..) = load_of(&app.dispatch(Action::Move(3)));
    assert_eq!(app.dispatch(Action::MoveToStart), Effect::None);
    assert_eq!(cursor_rel(&app), "src");
    assert_eq!(inners(&app), [preview("README.md")]);
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(FileRead::Binary)));
    let open = app.dispatch(Action::FoldOpen);
    land(&mut app, open);
    assert_eq!(app.dispatch(Action::Move(1)), Effect::None, "src/tui");
    assert_eq!(inners(&app), [preview("README.md")]);
    assert!(
        app.tabs.active_file_view().is_some(),
        "README.md still paints"
    );
}

#[test]
fn brackets_switch_inner_tabs_and_select_the_tree_row() {
    let mut app = state();
    app.dispatch(Action::NextFileTab);
    assert_eq!(&*app.status, EXPLORER_FILE_TABS_ONLY, "off an Explorer tab");
    open_root(&mut app);
    app.dispatch(Action::NextFileTab);
    assert_eq!(&*app.status, NO_EXPLORER_FILE_TABS);
    let row = |action: Action| {
        PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == action)
            .expect("palette row")
    };
    assert_eq!(
        app.palette_disabled_reason(row(Action::NextFileTab))
            .as_deref(),
        Some(NO_EXPLORER_FILE_TABS)
    );

    // new.txt pinned, then src/lib.rs pinned, then README.md previewed.
    app.dispatch(Action::Move(2));
    app.dispatch(Action::NavEnter);
    app.dispatch(Action::MoveToStart);
    let open = app.dispatch(Action::FoldOpen);
    land(&mut app, open);
    let (_, _, path, _) = load_of(&app.dispatch(Action::Move(2)));
    assert_eq!(path, "src/lib.rs");
    app.dispatch(Action::NavEnter);
    load_of(&app.dispatch(Action::MoveToEnd));
    assert_eq!(
        inners(&app),
        [
            pinned("new.txt"),
            pinned("src/lib.rs"),
            preview("README.md")
        ]
    );
    assert_eq!(
        app.palette_disabled_reason(row(Action::PreviousFileTab)),
        None
    );

    // Close src/ so `[` has to open it again to select src/lib.rs.
    app.dispatch(Action::MoveToStart);
    app.dispatch(Action::FoldClose);
    assert!(!tab(&app).tree.is_expanded("src"));
    app.dispatch(Action::NextFileTab);
    assert_eq!(active_rel(&app).as_deref(), Some("new.txt"), "] wraps");
    assert_eq!(cursor_rel(&app), "new.txt");
    app.dispatch(Action::NextFileTab);
    assert_eq!(active_rel(&app).as_deref(), Some("src/lib.rs"));
    assert!(tab(&app).tree.is_expanded("src"), "revealed");
    assert_eq!(cursor_rel(&app), "src/lib.rs");
    app.dispatch(Action::PreviousFileTab);
    app.dispatch(Action::PreviousFileTab);
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"), "[ wraps");
    assert_eq!(cursor_rel(&app), "README.md");
    assert_eq!(tab(&app).preview_index(), Some(2), "the preview stays");
    assert_eq!(app.focus, FocusPane::Left);

    app.dispatch(Action::JumpToTab(1));
    assert_eq!(
        app.palette_disabled_reason(row(Action::NextFileTab))
            .as_deref(),
        Some(EXPLORER_FILE_TABS_ONLY)
    );
}

#[test]
fn ctrl_w_closes_inner_tabs_then_the_explorer_tab() {
    let mut app = state();
    open_root(&mut app);
    app.dispatch(Action::Move(2));
    app.dispatch(Action::NavEnter);
    load_of(&app.dispatch(Action::Move(1)));
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::MoveToStart);
    assert_eq!(inners(&app), [pinned("new.txt"), preview("README.md")]);
    app.dispatch(Action::CloseTab);
    assert_eq!(inners(&app), [pinned("new.txt")], "the active tab closes");
    assert_eq!(
        active_rel(&app).as_deref(),
        Some("new.txt"),
        "left neighbour"
    );
    assert_eq!(cursor_rel(&app), "new.txt", "selected in the tree");
    assert_eq!(app.tabs.len(), 2);
    app.dispatch(Action::CloseTab);
    assert!(inners(&app).is_empty());
    assert_eq!(app.focus, FocusPane::Left, "nothing left to focus");
    assert!(app.is_explorer_tab(), "the Explorer tab stays");
    app.dispatch(Action::CloseTab);
    assert!(app.tabs.is_workspace(), "then Ctrl-w closes it");
    assert_eq!(app.tabs.len(), 1);
}

#[test]
fn ctrl_w_on_the_first_inner_tab_activates_the_next_one() {
    let mut app = state();
    open_root(&mut app);
    app.dispatch(Action::Move(2));
    app.dispatch(Action::NavEnter);
    app.dispatch(Action::Move(1));
    app.dispatch(Action::NavEnter);
    app.dispatch(Action::PreviousFileTab);
    assert_eq!(active_rel(&app).as_deref(), Some("new.txt"));
    app.dispatch(Action::CloseTab);
    assert_eq!(inners(&app), [pinned("README.md")]);
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert_eq!(cursor_rel(&app), "README.md");
}

#[test]
fn changed_file_preview_shows_the_full_context_marker() {
    let mut app = state();
    open_root(&mut app);
    let (tab_id, gen, _, changed) = load_of(&app.dispatch(Action::Move(2)));
    assert!(changed, "new.txt is untracked: a diff");
    let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    assert!(app.explorer_diff().is_some());
    assert_eq!(
        app.diff_context_lines(),
        Some(crate::git::FULL_DIFF_CONTEXT_LINES)
    );
    assert!(
        app.full_context_active(),
        "the diff header says full context"
    );
}

#[test]
fn clean_file_preview_is_plain_body() {
    let mut app = state();
    open_root(&mut app);
    let (tab_id, gen, path, changed) = load_of(&app.dispatch(Action::Move(3)));
    assert_eq!((path.as_str(), changed), ("README.md", false));
    let body = ExplorerPreviewBody::File(FileRead::Text {
        lines: vec!["# app".into(), "text".into()],
        max_cols: 5,
    });
    assert!(app.apply_explorer_preview(tab_id, gen, body));
    assert!(app.explorer_diff().is_none());
    assert!(!app.full_context_active());
    let file = app.tabs.active_file_view().expect("a file body");
    assert_eq!(file.lines(), ["# app", "text"]);
    assert_eq!(file.generation, gen);
}

/// After Ctrl-w empties the inner tabs, no listing brings a file tab
/// back: not one in flight since before the close, not `r`, not a
/// status re-list.
#[test]
fn listing_after_ctrl_w_empties_the_inner_tabs_previews_nothing() {
    let mut app = state();
    open_root(&mut app);
    // src/ opens, its listing still in flight.
    let in_flight = app.dispatch(Action::FoldOpen);
    assert_eq!(dir_loads(&flat(in_flight.clone())), ["src"]);
    // Root rows now: src, (loading), target, new.txt, README.md.
    load_of(&app.dispatch(Action::MoveToEnd));
    assert_eq!(cursor_rel(&app), "README.md");
    app.dispatch(Action::CloseTab);
    assert!(inners(&app).is_empty());
    assert!(app.is_explorer_tab());
    let no_preview = |effects: &[Effect]| {
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadExplorerPreview { .. }))
    };

    let follow = land(&mut app, in_flight);
    assert!(no_preview(&follow), "in-flight listing: {follow:?}");
    assert!(inners(&app).is_empty());

    let refresh = app.dispatch(Action::Refresh);
    let follow = land(&mut app, refresh);
    assert!(no_preview(&follow), "`r`: {follow:?}");
    assert!(inners(&app).is_empty());

    let mut changes = app.snapshot.repos[0].changes.clone();
    changes.push(change("src/fresh.rs", None, true));
    let snap = repo("app", CheckoutKind::Primary, None, changes);
    crate::tui::app::apply_one_repo_snapshot(&mut app, "app", Some(snap));
    let relist = app.explorer_status_relist("app").expect("a re-list of src");
    let follow = land(&mut app, relist);
    assert!(no_preview(&follow), "status re-list: {follow:?}");
    assert!(inners(&app).is_empty());
    assert_eq!(cursor_rel(&app), "README.md");

    app.dispatch(Action::CloseTab);
    assert!(
        app.tabs.is_workspace(),
        "Ctrl-w now closes the Explorer tab"
    );
}

/// G3: a reveal whose cursor path is not listed yet resolves the
/// cursor to a stand-in row. When that row is a file, nothing previews
/// and the inner tabs stay until the real listing lands; then the
/// revealed file previews.
#[test]
fn reveal_onto_a_fallback_file_row_previews_only_the_revealed_file() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    let Effect::LoadExplorerDir { tab_id, req, .. } = app.dispatch(Action::ExplorerReveal) else {
        panic!("a root listing");
    };
    // A root of files only: the first row is a file.
    let follow = app
        .apply_explorer_dir(tab_id, req, "", Ok(vec![entry("README.md", false, false)]))
        .expect("fresh listing");
    assert_eq!(
        load_of(&follow).2,
        "README.md",
        "no stored cursor: first row"
    );
    assert_eq!(inners(&app), [preview("README.md")]);

    let app_repo = app
        .snapshot
        .repos
        .iter()
        .position(|repo| repo.repo == "app")
        .unwrap();
    app.snapshot.repos[app_repo]
        .changes
        .push(change("fresh.txt", None, true));
    app.rebuild_rows();
    app.dispatch(Action::JumpToTab(1));
    focus(&mut app, NodeKind::File, Some("app"), Some("fresh.txt"));
    let effects = flat(app.dispatch(Action::ExplorerReveal));
    assert_eq!(dir_loads(&effects), [""], "the stale root lists again");
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadExplorerPreview { .. })),
        "the stand-in row README.md must not preview: {effects:?}"
    );
    let (rows, cursor) = app.explorer_rows().unwrap();
    assert_eq!(rows[cursor.unwrap()].rel, "README.md", "the stand-in row");
    assert_eq!(inners(&app), [preview("README.md")], "unchanged");
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));

    let Some(Effect::LoadExplorerDir { req, .. }) = effects.first().cloned() else {
        panic!("{effects:?}");
    };
    let listing = vec![
        entry("fresh.txt", false, false),
        entry("README.md", false, false),
    ];
    let follow = app
        .apply_explorer_dir(tab_id, req, "", Ok(listing))
        .expect("fresh listing");
    assert_eq!(load_of(&follow).2, "fresh.txt");
    assert_eq!(inners(&app), [preview("fresh.txt")]);
}

/// Quick Open (files) over `paths` of `checkout`, with `hits` (entry
/// indices) scored.
fn quick_open_with(app: &mut AppState, checkout: &str, paths: &[&str], hits: &[usize]) {
    use super::super::super::action::QuickOpenEntry;
    use super::super::super::quick_open::{FileIndexState, QuickOpenState};
    use crate::file_index::{FileEntry, FileHit, FileIndex, IndexRoot};
    let mut quick = QuickOpenState::new(QuickOpenEntry::Files, app.quick_open_scope());
    quick.index = FileIndexState::Ready(Arc::new(FileIndex {
        roots: vec![IndexRoot {
            checkout: checkout.into(),
            prefix: String::new(),
        }],
        entries: paths
            .iter()
            .map(|path| FileEntry {
                root: 0,
                display: (*path).to_string(),
                rel_start: 0,
            })
            .collect(),
        truncated: false,
        errors: Vec::new(),
    }));
    quick.hits = hits
        .iter()
        .map(|&entry| FileHit {
            entry,
            score: 1,
            indices: Vec::new(),
        })
        .collect();
    app.quick_open = Some(quick);
}

fn has_file_load(effect: &Effect) -> bool {
    flat(effect.clone())
        .iter()
        .any(|effect| matches!(effect, Effect::LoadExplorerPreview { .. }))
}

/// The inner file load of `effect`: `(tab_id, gen)`.
fn file_load(effect: &Effect) -> (u64, u64) {
    flat(effect.clone())
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadExplorerPreview { tab_id, gen, .. } => Some((*tab_id, *gen)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("an inner file load: {effect:?}"))
}

fn text(lines: &[&str]) -> ExplorerPreviewBody {
    ExplorerPreviewBody::File(FileRead::Text {
        lines: lines.iter().map(|line| (*line).to_string()).collect(),
        max_cols: lines.iter().map(|line| line.len()).max().unwrap_or(0),
    })
}

#[test]
fn quick_open_from_workspace_pins_in_explorer() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    app.focus = FocusPane::Right;
    quick_open_with(&mut app, "app", &["README.md", "src/lib.rs"], &[1, 0]);
    let effect = app.dispatch(Action::QuickOpenSubmit);
    assert!(app.quick_open.is_none(), "Enter closes the overlay");
    assert!(app.is_explorer_tab());
    assert_eq!(tab(&app).checkout, "app");
    assert_eq!(
        dir_loads(&flat(effect.clone())),
        ["", "src"],
        "the tree reveals the file"
    );
    assert!(has_file_load(&effect), "{effect:?}");
    assert_eq!(
        inners(&app),
        [pinned("src/lib.rs")],
        "pinned, not a preview"
    );
    assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");
    assert_eq!(tab(&app).tree.cursor_rel(), Some("src/lib.rs"));
    // Once the listings land the tree row of the file is selected, and
    // the open inner tab is not loaded twice.
    let follow = land(&mut app, effect);
    let loads = |follow: &[Effect]| {
        follow
            .iter()
            .filter(|effect| matches!(effect, Effect::LoadExplorerPreview { .. }))
            .count()
    };
    assert_eq!(loads(&follow), 1, "only the open's own load: {follow:?}");
    assert_eq!(cursor_rel(&app), "src/lib.rs");
    assert_eq!(inners(&app), [pinned("src/lib.rs")]);
    assert_eq!(app.focus, FocusPane::Right, "the listings keep the focus");

    // The file view focus is parked with the tab, like a Tab to it.
    app.dispatch(Action::JumpToTab(1));
    assert!(app.tabs.is_workspace());
    app.focus = FocusPane::Left;
    app.dispatch(Action::JumpToTab(2));
    assert!(app.is_explorer_tab());
    assert_eq!(app.focus, FocusPane::Right, "back on the file view");
}

#[test]
fn quick_open_on_an_explorer_tab_focuses_the_file_view() {
    let mut app = state();
    open_root(&mut app);
    load_of(&app.dispatch(Action::MoveToEnd));
    assert_eq!(inners(&app), [preview("README.md")]);
    assert_eq!(app.focus, FocusPane::Left);

    quick_open_with(&mut app, "app", &["README.md", "new.txt"], &[1]);
    let effect = app.dispatch(Action::QuickOpenSubmit);
    assert!(has_file_load(&effect), "{effect:?}");
    assert_eq!(inners(&app), [preview("README.md"), pinned("new.txt")]);
    assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");
    assert_eq!(cursor_rel(&app), "new.txt", "the tree selects the file");

    // A file already open as an inner tab shows again with the file
    // view focused, without a second tab or a reload.
    app.dispatch(Action::FocusLeft);
    quick_open_with(&mut app, "app", &["README.md", "new.txt"], &[0]);
    let effect = app.dispatch(Action::QuickOpenSubmit);
    assert!(!has_file_load(&effect), "{effect:?}");
    assert_eq!(inners(&app), [pinned("README.md"), pinned("new.txt")]);
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert_eq!(app.focus, FocusPane::Right);
    assert_eq!(cursor_rel(&app), "README.md");
}

#[test]
fn quick_open_from_compare_tab_routes_to_checkout_explorer() {
    let mut app = state();
    app.open_compare_tab("lib".into(), "main".into(), "HEAD".into());
    assert_eq!(
        app.quick_open_scope(),
        QuickOpenScope::Checkout("lib".into())
    );
    quick_open_with(&mut app, "lib", &["lib.rs", "README.md"], &[1]);
    let effect = app.dispatch(Action::QuickOpenSubmit);
    assert!(has_file_load(&effect), "{effect:?}");
    assert_eq!(
        app.tabs.labels(0),
        ["Workspace", "lib ↔ main", "Explorer · lib"]
    );
    assert_eq!(tab(&app).checkout, "lib");
    assert_eq!(inners(&app), [pinned("README.md")]);
    assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");
    assert_eq!(
        app.quick_open_scope(),
        QuickOpenScope::Checkout("lib".into()),
        "Quick Open on the Explorer tab keeps to its checkout"
    );
}

#[test]
fn reopen_same_file_reuses_inner_tab() {
    let mut app = state();
    let first = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
    let (tab_id, gen) = file_load(&first);
    assert!(app.apply_explorer_preview(tab_id, gen, text(&["# app"])));
    app.dispatch(Action::JumpToTab(1));
    let again = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
    assert!(!has_file_load(&again), "already open and loaded: {again:?}");
    assert_eq!(app.tabs.len(), 2, "one Explorer tab");
    assert_eq!(inners(&app), [pinned("README.md")], "one inner tab");

    // A tree preview of the file is promoted, not duplicated.
    let mut app = state();
    open_root(&mut app);
    load_of(&app.dispatch(Action::MoveToEnd));
    assert_eq!(inners(&app), [preview("README.md")]);
    app.dispatch(Action::JumpToTab(1));
    let open = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
    assert!(!has_file_load(&open), "{open:?}");
    assert_eq!(inners(&app), [pinned("README.md")]);
}

#[test]
fn search_hit_lands_on_line_in_clean_file_and_arms_n() {
    let mut app = state();
    let effect = app.open_file_in_explorer(
        "app".into(),
        "README.md".into(),
        Some(3),
        Some("foo".into()),
    );
    let (tab_id, gen) = file_load(&effect);
    // Armed before the load lands, and still armed after the folder
    // listings land first.
    let follow = land(&mut app, effect);
    assert_eq!(
        follow,
        [Effect::LoadExplorerPreview {
            tab_id,
            gen,
            repo: "app".into(),
            path: "README.md".into(),
            change: None,
        }],
        "only the open's own load"
    );
    assert!(app.search_is_armed());
    assert_eq!(app.search_target, SearchPane::File);
    assert_eq!(app.search_query, "foo");
    assert_eq!(tab(&app).active_inner().unwrap().pending_line, Some(3));

    let lines = ["foo", "x", "foo", "foo bar", "y", "foo"];
    assert!(app.apply_explorer_preview(tab_id, gen, text(&lines)));
    let cursor = |app: &AppState| app.tabs.active_file_view().unwrap().cursor;
    assert_eq!(cursor(&app), 3);
    assert_eq!(app.search_hit, Some(3));
    assert_eq!(tab(&app).active_inner().unwrap().pending_line, None);
    assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");
    app.dispatch(Action::SearchNext);
    assert_eq!(cursor(&app), 5, "`n` steps from the hit line");
    app.dispatch(Action::SearchPrev);
    assert_eq!(cursor(&app), 3);
    let next = PALETTE_COMMANDS
        .iter()
        .find(|command| command.action == Action::SearchNext)
        .expect("next match row");
    assert_eq!(app.palette_disabled_reason(next), None);
}

#[test]
fn search_hit_lands_on_diff_row_in_changed_file() {
    let mut app = state();
    // `src/tui/app.rs` is modified in `app`: it opens as a diff.
    let effect = app.open_file_in_explorer(
        "app".into(),
        "src/tui/app.rs".into(),
        Some(1),
        Some("two".into()),
    );
    let (tab_id, gen) = file_load(&effect);
    assert!(app.explorer_diff_loading());
    assert_eq!(app.search_target, SearchPane::Diff);
    assert!(app.search_is_armed());
    // Rows: STAGED 0, hunk 1, `one` 2, `-two` 3; UNSTAGED 4, hunk 5,
    // `one` 6, `+two` 7, `three two` 8.
    let diff = super::super::super::diff::DiffContent {
        staged: "@@ -1,2 +1 @@\n one\n-two\n".into(),
        unstaged: "@@ -1 +1,3 @@\n one\n+two\n+three two\n".into(),
        ..Default::default()
    };
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    assert_eq!(app.diff_cursor, 7, "line 2 of the file, unstaged first");
    assert_eq!(app.search_hit, Some(7));
    assert_eq!(app.focus, FocusPane::Right, "the diff takes focus");
    app.dispatch(Action::SearchNext);
    assert_eq!(app.diff_cursor, 8, "`n` steps from the hit row");
    app.dispatch(Action::SearchPrev);
    assert_eq!(app.diff_cursor, 7);

    // Without a query the cursor still lands; nothing is armed.
    let mut app = state();
    let effect = app.open_file_in_explorer("app".into(), "new.txt".into(), Some(0), None);
    let (tab_id, gen) = file_load(&effect);
    let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    assert_eq!(app.diff_cursor, 2);
    assert!(!app.search_is_armed());
}

#[test]
fn no_session_tab_is_a_file_tab() {
    let mut app = state();
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    quick_open_with(&mut app, "app", &["README.md"], &[0]);
    app.dispatch(Action::QuickOpenSubmit);
    app.dispatch(Action::JumpToTab(1));
    focus(&mut app, NodeKind::Repo, Some("app"), None);
    quick_open_with(&mut app, "app", &["src/lib.rs"], &[0]);
    app.dispatch(Action::QuickOpenSubmit);
    app.open_file_in_explorer("app".into(), "new.txt".into(), Some(0), Some("x".into()));
    assert_eq!(app.tabs.labels(0), ["Workspace", "Explorer · app"]);
    assert_eq!(
        inners(&app),
        [pinned("README.md"), pinned("src/lib.rs"), pinned("new.txt")]
    );
}

#[test]
fn late_checkout_confirm_cannot_write_from_an_explorer_tab() {
    let mut app = state();
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    crate::tui::app::apply_checkout_compute(
        &mut app,
        "app".into(),
        crate::tui::app::CheckoutCompute::Confirm {
            local_branch: "main".into(),
            remote_ref: "origin/main".into(),
            ahead_behind: Some((0, 1)),
        },
    );
    assert!(app.confirm.is_some(), "the late result opens its confirm");
    for action in [Action::ConfirmYes, Action::ConfirmYesClean] {
        app.status.clear();
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert_eq!(&*app.status, EXPLORER_TAB_READ_ONLY, "{action:?}");
        assert!(app.confirm.is_some(), "{action:?} does not consume it");
    }
    assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
    assert!(app.confirm.is_none(), "n still closes it");
}

#[test]
fn closing_the_explorer_tab_restores_the_workspace_session() {
    let mut app = state();
    app.focus = FocusPane::Right;
    app.search_active = true;
    app.search_query = "readme".into();
    app.search_target = SearchPane::Tree;
    app.diff_cursor = 7;
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    assert!(!app.search_active, "the Explorer tab has no search");
    app.dispatch(Action::CloseTab);
    app.dispatch(Action::CloseTab);
    assert!(app.tabs.is_workspace());
    assert_eq!(app.focus, FocusPane::Right);
    assert!(app.search_active);
    assert_eq!(app.search_query, "readme");
    assert_eq!(app.search_target, SearchPane::Tree);
    assert_eq!(app.diff_cursor, 7);
}

/// A state whose cwd is a fresh temp dir with `app/notes.txt`.
fn disk_state(lines: &[&str]) -> (AppState, PathBuf) {
    let cwd = crate::testutil::unique_dir("ws-explorer-live");
    std::fs::create_dir_all(cwd.join("app")).expect("app dir");
    write_notes(&cwd, lines);
    let mut app = state();
    app.cwd = cwd.clone();
    (app, cwd)
}

/// Write `lines` to `app/notes.txt`. Callers change the line count (so
/// the size) on every rewrite, so the disk token moves whatever the
/// mtime granularity.
fn write_notes(cwd: &std::path::Path, lines: &[&str]) {
    std::fs::write(cwd.join("app/notes.txt"), lines.join("\n")).expect("write file");
}

fn file_lines(app: &AppState) -> Vec<String> {
    app.tabs
        .active_file_view()
        .expect("a file view")
        .lines()
        .to_vec()
}

fn set_changes(app: &mut AppState, changes: Vec<FileChange>) {
    let snap = repo("app", CheckoutKind::Primary, None, changes);
    crate::tui::app::apply_one_repo_snapshot(app, "app", Some(snap));
}

#[test]
fn explorer_live_reload_on_disk_and_status_change() {
    let (mut app, cwd) = disk_state(&["alpha", "beta"]);
    let (tab_id, gen) = app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta"]);
    assert!(
        tab(&app).active_inner().unwrap().stamp.is_some(),
        "open stamps"
    );
    for _ in 0..3 {
        assert_eq!(app.explorer_live_reload("app"), None, "unchanged");
    }
    assert_eq!(app.explorer_live_reload("lib"), None, "another checkout");
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::MoveToEnd);

    // A disk edit reloads quietly: the old body stays painted.
    write_notes(&cwd, &["beta", "alpha", "beta"]);
    let reload = app.explorer_live_reload("app").expect("a reload");
    let (_, disk_gen) = file_load(&reload);
    assert!(disk_gen > gen);
    assert_eq!(file_lines(&app), ["alpha", "beta"], "old body stays");
    assert!(app.status.is_empty(), "silent: {}", &*app.status);
    assert_eq!(app.explorer_live_reload("app"), None, "stamp recorded");
    assert!(app.apply_explorer_preview(tab_id, disk_gen, text(&["beta", "alpha", "beta"])));
    assert_eq!(file_lines(&app), ["beta", "alpha", "beta"]);
    assert_eq!(
        app.tabs.active_file_view().unwrap().cursor,
        1,
        "the cursor keeps its line"
    );

    // A failed silent read keeps the body.
    write_notes(&cwd, &["one"]);
    let (_, failed) = file_load(&app.explorer_live_reload("app").expect("a reload"));
    assert!(!app.apply_explorer_preview(
        tab_id,
        failed,
        ExplorerPreviewBody::File(FileRead::Failed("gone".into()))
    ));
    assert_eq!(file_lines(&app), ["beta", "alpha", "beta"]);

    // While an overlay holds input nothing is checked or recorded.
    write_notes(&cwd, &["one", "two"]);
    let stamp = tab(&app).active_inner().unwrap().stamp.clone();
    quick_open_with(&mut app, "app", &["notes.txt"], &[0]);
    assert_eq!(app.explorer_live_reload("app"), None);
    assert_eq!(tab(&app).active_inner().unwrap().stamp, stamp);
    app.dispatch(Action::QuickOpenEscape);

    // A status change that makes the file changed swaps in a diff.
    let mut changes = app.snapshot.repos[0].changes.clone();
    changes.push(change("notes.txt", Some("M"), false));
    set_changes(&mut app, changes);
    let reload = app.explorer_live_reload("app").expect("a reload");
    match &reload {
        Effect::LoadExplorerPreview { change, .. } => assert!(change.is_some()),
        other => panic!("{other:?}"),
    }
    assert!(app.explorer_diff_loading(), "a loading diff view");
    let (_, diff_gen) = file_load(&reload);
    let diff = super::super::super::diff::DiffContent::from_unified("@@ -1 +1,2 @@\n one\n+two\n");
    assert!(app.apply_explorer_preview(tab_id, diff_gen, ExplorerPreviewBody::Diff(diff)));
    assert!(app.explorer_diff().is_some());

    // Background tabs wait; activating the tab checks once.
    app.dispatch(Action::JumpToTab(1));
    write_notes(&cwd, &["one", "two", "three"]);
    assert_eq!(app.explorer_live_reload("app"), None, "not active");
    let back = app.dispatch(Action::JumpToTab(2));
    assert!(has_file_load(&back), "{back:?}");
    assert_eq!(app.dispatch(Action::JumpToTab(1)), Effect::None);
    assert_eq!(app.dispatch(Action::JumpToTab(2)), Effect::None, "once");
    let (_, gen) = file_load(&back);
    let diff =
        super::super::super::diff::DiffContent::from_unified("@@ -1 +1,3 @@\n one\n+two\n+three\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));

    // A deleted file keeps the last content and says so.
    std::fs::remove_file(cwd.join("app/notes.txt")).expect("delete");
    assert_eq!(app.explorer_live_reload("app"), None, "nothing to read");
    assert_eq!(&*app.status, FILE_DELETED);
    assert_eq!(app.status.kind(), StatusKind::Warn);
    assert!(app.explorer_diff().is_some(), "content kept");
    assert_eq!(app.explorer_live_reload("app"), None, "gone recorded");
    let _ = std::fs::remove_dir_all(&cwd);
}

fn edit_of(effect: Effect) -> (String, Option<u32>) {
    match effect {
        Effect::EditFile { repo, path, line } => {
            assert_eq!(repo, "app");
            (path, line)
        }
        other => panic!("an editor launch, got {other:?}"),
    }
}

#[test]
fn e_opens_the_active_inner_file_at_its_line() {
    // Focus right on a clean file: its cursor line.
    let mut app = state();
    app.open_loaded_inner_file("app", "README.md", &["a", "b", "c", "d"]);
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::Move(2));
    assert_eq!(
        edit_of(app.dispatch(Action::Edit)),
        ("README.md".into(), Some(3))
    );
    assert_eq!(&*app.status, "opening README.md…");

    // A binary body opens with no line.
    let effect = app.open_file_in_explorer("app".into(), "logo.png".into(), None, None);
    let (tab_id, gen) = file_load(&effect);
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(FileRead::Binary)));
    app.dispatch(Action::FocusRight);
    assert_eq!(
        edit_of(app.dispatch(Action::Edit)),
        ("logo.png".into(), None)
    );

    // A diff: the new-file line of the cursor row; a hunk header has none.
    let effect = app.open_file_in_explorer("app".into(), "src/tui/app.rs".into(), None, None);
    let (tab_id, gen) = file_load(&effect);
    let diff =
        super::super::super::diff::DiffContent::from_unified("@@ -1,2 +1,2 @@\n one\n-old\n+new\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    app.dispatch(Action::FocusRight);
    // UNSTAGED 0, hunk 1, `one` 2, `-old` 3, `+new` 4.
    app.diff_cursor = 4;
    assert_eq!(
        edit_of(app.dispatch(Action::Edit)),
        ("src/tui/app.rs".into(), Some(2))
    );
    app.diff_cursor = 1;
    assert_eq!(
        edit_of(app.dispatch(Action::Edit)),
        ("src/tui/app.rs".into(), None)
    );
}

#[test]
fn e_with_the_tree_focused_edits_the_tree_row() {
    let mut app = state();
    let open = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
    let (tab_id, gen) = file_load(&open);
    land(&mut app, open);
    assert!(app.apply_explorer_preview(tab_id, gen, text(&["a", "b", "c"])));
    app.dispatch(Action::FocusRight);
    app.dispatch(Action::Move(2));
    app.dispatch(Action::FocusLeft);
    assert_eq!(
        edit_of(app.dispatch(Action::Edit)),
        ("README.md".into(), Some(3)),
        "the tree row is the active inner file: its line"
    );
    // Another file row: that file, with no line.
    app.tabs
        .active_explorer_mut()
        .unwrap()
        .tree
        .set_cursor_rel("new.txt");
    assert_eq!(
        edit_of(app.dispatch(Action::Edit)),
        ("new.txt".into(), None)
    );
    // A folder row refuses.
    app.tabs
        .active_explorer_mut()
        .unwrap()
        .tree
        .set_cursor_rel("src");
    assert_eq!(app.dispatch(Action::Edit), Effect::None);
    assert_eq!(&*app.status, FOCUS_A_FILE_TO_EDIT);
}

#[test]
fn editor_return_reloads_only_the_active_inner_file() {
    let (mut app, cwd) = disk_state(&["alpha", "beta"]);
    assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());
    let (tab_id, gen) = app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta"]);
    app.tabs.active_file_view_mut().expect("a file view").cursor = 1;

    // An exit without a save moves no stamp: nothing reloads.
    assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());

    write_notes(&cwd, &["alpha", "beta", "gamma"]);
    assert!(app.reload_explorer_after_edit("app", "other.txt").is_none());
    assert!(app.reload_explorer_after_edit("lib", "notes.txt").is_none());

    // A save reloads quietly: the old body stays painted and the line
    // cursor stays.
    let reload = app
        .reload_explorer_after_edit("app", "notes.txt")
        .expect("the active inner file reloads");
    let (_, saved) = file_load(&reload);
    assert!(saved > gen, "a new generation");
    assert_eq!(file_lines(&app), ["alpha", "beta"], "old body stays");
    assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());
    assert!(app.apply_explorer_preview(tab_id, saved, text(&["alpha", "beta", "gamma"])));
    assert_eq!(file_lines(&app), ["alpha", "beta", "gamma"]);
    assert_eq!(app.tabs.active_file_view().unwrap().cursor, 1);

    // An inner tab that is not active is left to the live-reload check
    // that runs when it shows again.
    std::fs::write(cwd.join("app/other.txt"), "o1").expect("write other");
    app.open_loaded_inner_file("app", "other.txt", &["o1"]);
    write_notes(&cwd, &["alpha"]);
    assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());
    let _ = std::fs::remove_dir_all(&cwd);
}

#[test]
fn editor_return_keeps_the_inner_diff_cursor() {
    use super::super::super::diff::DiffContent;
    let (mut app, cwd) = disk_state(&["a", "c", "d"]);
    app.diff_mode = super::super::super::split::DiffMode::Inline;
    let mut changes = app.snapshot.repos[0].changes.clone();
    changes.push(change("notes.txt", Some("M"), false));
    set_changes(&mut app, changes);
    // Inline rows: 0 label, 1 hunk, 2 ` a`, 3 `-b`, 4 `+c`, 5 `+d`.
    let (tab_id, _) = app.open_loaded_inner_diff(
        "app",
        "notes.txt",
        DiffContent::from_unified("@@ -1,2 +1,3 @@\n a\n-b\n+c\n+d\n"),
    );
    app.diff_cursor = 4;

    // `e` on the diff row, a save, then the editor returns.
    write_notes(&cwd, &["a", "c", "d", "e"]);
    let reload = app
        .reload_explorer_after_edit("app", "notes.txt")
        .expect("the saved file reloads");
    match &reload {
        Effect::LoadExplorerPreview { change, .. } => assert!(change.is_some()),
        other => panic!("{other:?}"),
    }
    assert!(!app.explorer_diff_loading(), "the old diff stays painted");
    assert_eq!(app.diff_cursor, 4, "the cursor row stays while it loads");
    assert!(app.status.is_empty(), "silent: {}", &*app.status);

    let (_, gen) = file_load(&reload);
    let diff = DiffContent::from_unified("@@ -1,2 +1,4 @@\n a\n-b\n+c\n+d\n+e\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    assert!(app.explorer_diff().is_some());
    assert_eq!(app.diff_cursor, 4, "the cursor row stays on the new diff");
    let _ = std::fs::remove_dir_all(&cwd);
}

#[test]
fn showing_an_open_inner_tab_runs_the_live_reload_check() {
    let (mut app, cwd) = disk_state(&["n1"]);
    std::fs::write(cwd.join("app/other.txt"), "o1").expect("write other");
    app.open_loaded_inner_file("app", "notes.txt", &["n1"]);
    app.open_loaded_inner_file("app", "other.txt", &["o1"]);
    assert_eq!(inners(&app), [pinned("notes.txt"), pinned("other.txt")]);

    // `]` onto notes.txt after a disk edit: a silent reload.
    write_notes(&cwd, &["n1", "n2"]);
    assert_eq!(
        app.explorer_live_reload("app"),
        None,
        "other.txt is unchanged"
    );
    let shown = app.dispatch(Action::NextFileTab);
    assert_eq!(active_rel(&app).as_deref(), Some("notes.txt"));
    assert!(has_file_load(&shown), "{shown:?}");
    assert_eq!(file_lines(&app), ["n1"], "the old body stays painted");
    assert!(
        !has_file_load(&app.dispatch(Action::PreviousFileTab)),
        "other.txt did not move"
    );

    // Ctrl-w onto notes.txt after another edit.
    write_notes(&cwd, &["n1", "n2", "n3"]);
    let shown = app.dispatch(Action::CloseTab);
    assert_eq!(active_rel(&app).as_deref(), Some("notes.txt"));
    assert!(has_file_load(&shown), "{shown:?}");

    // A Quick Open of the open file checks it too.
    app.open_loaded_inner_file("app", "other.txt", &["o1"]);
    write_notes(&cwd, &["n1", "n2", "n3", "n4"]);
    let open = app.open_file_in_explorer("app".into(), "notes.txt".into(), None, None);
    assert!(has_file_load(&open), "{open:?}");
    assert_eq!(inners(&app).len(), 2, "no duplicate");
    let _ = std::fs::remove_dir_all(&cwd);
}

#[test]
fn view_remotely_opens_the_tree_row_or_the_open_file_line() {
    use super::super::super::remote_view::{FileRev, RemoteTarget};
    let mut app = state();
    open_root(&mut app);
    let (tab_id, gen, ..) = load_of(&app.dispatch(Action::MoveToEnd));
    assert!(app.apply_explorer_preview(tab_id, gen, text(&["# app", "", "more"])));
    let page = |path: &str, line: Option<u32>, dir: bool| Effect::ViewRemotely {
        checkout: PathBuf::from("app"),
        target: RemoteTarget::File {
            rev: FileRev::Branch("main".into()),
            path: path.into(),
            line,
            dir,
        },
    };

    // Tree focused: the file row, then a folder row.
    assert_eq!(cursor_rel(&app), "README.md");
    assert_eq!(
        app.dispatch(Action::ViewRemotely),
        page("README.md", None, false)
    );
    app.dispatch(Action::MoveToStart);
    assert_eq!(cursor_rel(&app), "src");
    assert_eq!(app.dispatch(Action::ViewRemotely), page("src", None, true));
    assert_eq!(palette_reason(&app, Action::ViewRemotely), None);

    // File pane focused: the open file at the cursor line (1-based).
    app.dispatch(Action::FocusRight);
    app.tabs.active_file_view_mut().expect("file view").cursor = 2;
    assert_eq!(
        app.dispatch(Action::ViewRemotely),
        page("README.md", Some(3), false)
    );

    // A changed file's diff: the focused row's new-side line.
    let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
    app.open_loaded_inner_diff("app", "new.txt", diff);
    app.focus = FocusPane::Right;
    app.diff_cursor = app
            .current_diff_rows()
            .iter()
            .position(|row| matches!(row, super::super::super::diff::DiffRow::Line { left, .. } if left.text == "new"))
            .expect("added row");
    assert_eq!(
        app.dispatch(Action::ViewRemotely),
        page("new.txt", Some(1), false)
    );
}

#[test]
fn breadcrumb_and_copy_reference_name_the_active_inner_file() {
    use crate::tui::chrome::breadcrumb_segments;
    let mut app = state();
    open_root(&mut app);
    let (tab_id, gen, ..) = load_of(&app.dispatch(Action::MoveToEnd));
    assert!(app.apply_explorer_preview(tab_id, gen, text(&["# app"])));
    app.dispatch(Action::MoveToStart);
    assert_eq!(cursor_rel(&app), "src", "the tree cursor sits on a folder");
    app.tabs.active_explorer_mut().unwrap().painted_rel = Some("src".into());
    let copied = |app: &mut AppState| match app.dispatch(Action::CopyEntityReference) {
        Effect::CopyClipboard { text, .. } => text,
        other => panic!("a copy, got {other:?}"),
    };

    // Tree focused: the breadcrumb names the folder row; a copy names
    // the open file (a folder has no file reference).
    assert_eq!(breadcrumb_segments(&app)[1..], ["app", "src"]);
    let text = copied(&mut app);
    assert!(text.contains("README.md"), "{text}");

    // File pane focused: both name the active inner file.
    app.dispatch(Action::FocusRight);
    assert_eq!(app.focus, FocusPane::Right);
    assert_eq!(breadcrumb_segments(&app)[1..], ["app", "README.md"]);
    let text = copied(&mut app);
    assert!(
        text.contains("README.md") && !text.contains("src"),
        "{text}"
    );

    // Tree focused on a file row: that row.
    app.dispatch(Action::FocusLeft);
    load_of(&app.dispatch(Action::Move(2)));
    assert_eq!(cursor_rel(&app), "new.txt");
    let text = copied(&mut app);
    assert!(text.contains("new.txt"), "{text}");
}

/// Go to line from the Explorer tree jumps in the preview: a changed
/// file's diff by its new-side line, a clean file's body by line. A
/// folder row or a body still loading has no lines.
#[test]
fn go_to_line_jumps_in_the_preview_from_the_tree() {
    use super::super::go_to_line::NO_LINES_TO_GO_TO;
    use crate::tui::diff::{DiffContent, DiffRow};
    let mut app = state();
    open_root(&mut app);
    assert_eq!(cursor_rel(&app), "src");
    assert_eq!(app.go_to_line_refusal().as_deref(), Some(NO_LINES_TO_GO_TO));
    assert_eq!(app.dispatch(Action::GoToLine(2)), Effect::None);
    assert_eq!(app.status, NO_LINES_TO_GO_TO);
    assert_eq!(app.focus, FocusPane::Left, "a refusal moves nothing");

    // new.txt is untracked: its preview is a diff.
    let Effect::LoadExplorerPreview {
        tab_id,
        gen,
        change: Some(_),
        ..
    } = app.dispatch(Action::Move(2))
    else {
        panic!("new.txt diff preview");
    };
    let diff = DiffContent::from_unified("@@ -0,0 +1,3 @@\n+one\n+two\n+three\n");
    assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
    assert_eq!(app.dispatch(Action::GoToLine(2)), Effect::None);
    assert_eq!(app.focus, FocusPane::Right, "the jump focuses the preview");
    let row_text = |app: &AppState| match &app.current_diff_rows()[app.diff_cursor] {
        DiffRow::Line { left, .. } => left.text.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(row_text(&app), "two");
    app.dispatch(Action::GoToLine(40));
    assert_eq!(row_text(&app), "three", "clamped to the last line");

    // README.md is clean: its preview is the file body.
    app.dispatch(Action::FocusLeft);
    let Effect::LoadExplorerPreview {
        tab_id,
        gen,
        change: None,
        ..
    } = app.dispatch(Action::MoveToEnd)
    else {
        panic!("README.md body preview");
    };
    assert_eq!(
        app.go_to_line_refusal().as_deref(),
        Some(NO_LINES_TO_GO_TO),
        "still loading"
    );
    let lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
    let body = ExplorerPreviewBody::File(FileRead::Text { lines, max_cols: 7 });
    assert!(app.apply_explorer_preview(tab_id, gen, body));
    assert_eq!(app.go_to_line_refusal(), None);
    app.dispatch(Action::GoToLine(12));
    assert_eq!(app.focus, FocusPane::Right);
    let cursor = |app: &AppState| app.tabs.active_file_view().expect("body").cursor;
    assert_eq!(cursor(&app), 11);
    app.dispatch(Action::GoToLine(0));
    assert_eq!(cursor(&app), 0, "below 1 is line 1");
    assert_eq!(cursor_rel(&app), "README.md", "the tree did not move");
}

/// A pinned inner file keeps showing while the tree sits on a folder
/// row, so go to line still lands in it; closing the last inner file
/// leaves no lines to go to.
#[test]
fn go_to_line_lands_in_the_active_inner_file_from_a_folder_row() {
    use super::super::go_to_line::NO_LINES_TO_GO_TO;
    let mut app = state();
    open_root(&mut app);
    let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    app.open_loaded_inner_file("app", "README.md", &lines);
    app.dispatch(Action::FocusLeft);
    app.dispatch(Action::MoveToStart);
    assert_eq!(cursor_rel(&app), "src", "a folder row");
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert_eq!(app.go_to_line_refusal(), None);
    assert_eq!(app.dispatch(Action::GoToLine(7)), Effect::None);
    assert_eq!(app.focus, FocusPane::Right, "the jump focuses the file");
    assert_eq!(app.tabs.active_file_view().map(|file| file.cursor), Some(6));

    app.dispatch(Action::CloseTab);
    assert!(tab(&app).files.is_empty(), "no inner file left");
    assert_eq!(app.go_to_line_refusal().as_deref(), Some(NO_LINES_TO_GO_TO));
}

/// A `Theme: …` row previews over an Explorer inner file, Esc puts the
/// open-time theme back, and Enter applies it with the file still open.
#[test]
fn explorer_inner_file_previews_and_applies_a_palette_theme_row() {
    use crate::tui::theme::ThemeId;
    let mut app = state();
    open_root(&mut app);
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    app.focus = FocusPane::Right;
    let before = app.theme;
    assert_ne!(before, ThemeId::Nord);
    let type_nord = |app: &mut AppState| {
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "nord".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
    };
    type_nord(&mut app);
    assert_eq!(app.theme, ThemeId::Nord, "focus previews the row");
    // Esc clears the typed filter, then closes.
    app.dispatch(Action::QuickOpenEscape);
    app.dispatch(Action::QuickOpenEscape);
    assert_eq!(app.theme, before, "Esc restores the open-time theme");

    type_nord(&mut app);
    assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
    assert!(app.quick_open.is_none());
    assert_eq!(app.theme, ThemeId::Nord);
    assert_eq!(app.status, "theme: Nord");
    assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
    assert_eq!(app.focus, FocusPane::Right);
}

/// The Search in files, Go to file, and `Theme: …` palette rows run on
/// an Explorer tab (with or without its file pane focused); Open in
/// Explorer is off.
#[test]
fn explorer_palette_rows_from_main_follow_the_explorer_gates() {
    let mut app = state();
    open_root(&mut app);
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    let reason = |app: &AppState, title: &str| {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == title)
            .unwrap_or_else(|| panic!("palette row {title}"));
        app.palette_disabled_reason(command)
    };
    for focus in [FocusPane::Left, FocusPane::Right] {
        app.focus = focus;
        assert_eq!(reason(&app, "Search in files"), None, "{focus:?}");
        assert_eq!(reason(&app, "Go to file"), None, "{focus:?}");
        assert_eq!(reason(&app, "Theme: Nord"), None, "{focus:?}");
        assert_eq!(
            reason(&app, "Open in Explorer").as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB),
            "{focus:?}"
        );
        for title in ["Shorter commit message", "Taller commit message"] {
            assert_eq!(
                reason(&app, title).as_deref(),
                Some(SWITCH_TO_WORKSPACE_TAB),
                "{title} {focus:?}"
            );
        }
    }
}
