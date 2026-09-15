use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git};
use crate::support::{
    no_wrong_overlays, panes_tree_focused_diff_unfocused, right_pane, title_has_files,
    tree_cursor_on, tree_has, GIT_WAIT,
};

const FILE: &str = "pack.json";
const ADD_MARK: &str = "alpha-syntax";
const DEL_MARK: &str = "old-name";
/// Tokyo Night `palette.added` / `deleted` / `diffAddBg` / `diffDelBg`.
/// Keep in sync with `tui/theme.rs` TOKYO_NIGHT.
const ADDED: (u8, u8, u8) = (0x9e, 0xce, 0x6a);
const DELETED: (u8, u8, u8) = (0xf7, 0x76, 0x8e);
const DIFF_ADD_BG: (u8, u8, u8) = (0x3f, 0x4d, 0x39);
const DIFF_DEL_BG: (u8, u8, u8) = (0x58, 0x34, 0x43);

/// Tracked JSON with one replaced string and one replaced number.
fn seed_syntax_json(workspace: &Path) {
    let app = workspace.join("app");
    let path = app.join(FILE);
    fs::write(
        &path,
        "{\n  \"name\": \"old-name\",\n  \"ttlMs\": 5000\n}\n",
    )
    .unwrap();
    git(&app, &["add", FILE]);
    git(&app, &["commit", "-q", "-m", "pack.json base"]);
    fs::write(
        &path,
        "{\n  \"name\": \"alpha-syntax\",\n  \"ttlMs\": 2000\n}\n",
    )
    .unwrap();
}

/// Left tree focused on `pack.json`. Right pane is that file-diff.
///
/// 80 columns force inline so the add sign sits on the same row as the
/// JSON tokens (split would put the del cell on the left).
fn documented_json_syntax_diff(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, FILE)
        && right.contains(&format!("app/{FILE}"))
        && right.contains("UNSTAGED")
        && right.contains(ADD_MARK)
        && right.contains(DEL_MARK)
        && right.contains("inline (too narrow)")
        && !right.contains("WIP on graph")
        && !title_has_files(screen)
        && no_wrong_overlays(screen)
}

/// File-diff syntax keeps +/- signs and add/del row backgrounds.
///
/// Docs: language from path/extension; token foregrounds only; `+` / `-`
/// stay `added` / `deleted`; add/del rows use `diffAddBg` / `diffDelBg`.
/// Cursor overlay must not cover the JSON add/del lines (left pane stays
/// focused). A no-op, a solid add-accent wash, or a syntect background
/// that replaces the row tint is red.
#[test]
fn pty_diff_syntax_highlight_keeps_signs_and_row_bg() {
    let (_root, workspace) = daily_workspace();
    seed_syntax_json(&workspace);
    let mut tui = PtySession::open_size(&workspace, 80, 24);
    tui.search("pack.json");
    tui.wait_pred(
        documented_json_syntax_diff,
        "search loads the JSON file-diff; tree stays focused on pack.json",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        tui.needle_has_bg(ADD_MARK, DIFF_ADD_BG.0, DIFF_ADD_BG.1, DIFF_ADD_BG.2),
        "add-line JSON must keep diffAddBg under syntax fg:\n{screen}"
    );
    assert!(
        tui.needle_has_bg(DEL_MARK, DIFF_DEL_BG.0, DIFF_DEL_BG.1, DIFF_DEL_BG.2),
        "del-line JSON must keep diffDelBg under syntax fg:\n{screen}"
    );
    assert_eq!(
        tui.first_glyph_on_needle_row_fg(ADD_MARK, '+'),
        Some(Some(ADDED)),
        "add sign stays added fg:\n{screen}"
    );
    assert_eq!(
        tui.first_glyph_on_needle_row_fg(DEL_MARK, '-'),
        Some(Some(DELETED)),
        "del sign stays deleted fg:\n{screen}"
    );

    let add_span = tui
        .first_needle_fgs("\"name\": \"alpha-syntax\"")
        .unwrap_or_else(|| panic!("add JSON span missing:\n{screen}"));
    let unique: HashSet<_> = add_span.iter().copied().flatten().collect();
    assert!(
        unique.len() >= 2,
        "JSON tokens on an add line should use more than one fg: {unique:?}\n{screen}"
    );
    assert!(
        unique.iter().any(|fg| *fg != ADDED),
        "syntax fg must not wash the add line with the added accent: {unique:?}\n{screen}"
    );
}
