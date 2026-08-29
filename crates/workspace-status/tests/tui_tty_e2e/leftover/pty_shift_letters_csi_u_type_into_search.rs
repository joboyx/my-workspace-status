use crate::harness::PtySession;
use crate::seed::{daily_workspace, focus_workspace};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// Painted SEARCH status line with this query and the typing cursor.
///
/// The capital glyphs must sit on that line. A lowercase type-in, an
/// armed `/{query}` chip, or a global Shift binding cannot pass.
fn search_prompt_has_query(screen: &str, query: &str) -> bool {
    let Some(status) = screen.lines().last() else {
        return false;
    };
    status.contains("SEARCH")
        && status.contains(&format!("{query}▏"))
        && status.contains("Enter arms query")
        && status.contains("Esc clears")
        && status.contains("n/N after Enter")
        && (query.is_empty() || !status.contains(&format!("/{query}")))
}

/// Global Shift+letter bindings that must not fire while `/` is typing.
fn search_did_not_fire_global_shift(screen: &str) -> bool {
    !screen.contains("Stash ")
        && !screen.contains("theme: Monokai")
        && !screen.contains("theme: Dracula")
        && !screen.contains("theme: Gruvbox")
        && !screen.contains("theme: Catppuccin")
        && !screen.contains("Flat paths")
        && !screen.contains("Focus branches")
        && !screen.contains("full graph")
        && !tree_cursor_on(screen, "No updates")
}

/// CSI-u Shift+letters in an armed `/` query type capitals.
///
/// Docs + help: `/` search, characters append while typing. Global
/// Shift+O clears graph focus, Shift+S opens stash, Shift+G jumps to
/// the last row, Shift+T cycles theme. Those must not fire. Raw `'O'`
/// is a different path. Enter-arm / pane search stay on other tests.
#[test]
fn pty_shift_letters_csi_u_type_into_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("? help")
                && tree_cursor_on(screen, "README.md")
                && !screen.contains("SEARCH")
                && !screen.contains("MOVE")
        },
        "idle chrome before /; SEARCH is closed",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && screen.contains("search focused pane")
                && screen.contains("stash menu")
                && screen.contains("top / bottom")
                && screen.contains("cycle theme")
                && screen.contains("focus branches")
        },
        "help lists / search and the global Shift+O/S/G/T bindings",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("search focused pane")
                && screen.contains("? help")
                && tree_cursor_on(screen, "README.md")
        },
        "Esc closes help so Shift+letters go to pane search, not help",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("O▏")
                && tree_cursor_on(screen, "README.md")
        },
        "/ opens SEARCH; query is empty; Shift bindings have not fired",
        WAIT,
    );

    tui.shift_letter('O');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "O")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("o▏")
        },
        "CSI-u Shift+O types O; it must not clear graph focus",
        WAIT,
    );

    tui.shift_letter('S');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "OS")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("os▏")
        },
        "CSI-u Shift+S types S; it must not open the stash menu",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "OSG")
                && search_did_not_fire_global_shift(screen)
                && !tree_cursor_on(screen, "No updates")
        },
        "CSI-u Shift+G types G; it must not jump to the last tree row",
        WAIT,
    );

    tui.shift_letter('T');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "OSGT")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("/OSGT")
                && !screen.contains("osgt▏")
        },
        "CSI-u Shift+T types T; it must not cycle theme or arm the query",
        WAIT,
    );

    let (_root2, focused) = focus_workspace();
    let mut focused_tui = PtySession::open(&focused);
    focused_tui.wait_contains("focusbox", WAIT);
    focused_tui.tab();
    focused_tui.wait_pred(
        |screen| {
            screen.contains("keep-leaf-commit")
                && screen.contains("noise-leaf-commit")
                && screen.contains("o   focus branches")
        },
        "graph shows both leaves; o opens focus (O clear is hidden until a focus is on)",
        GIT_WAIT,
    );

    focused_tui.key('o');
    focused_tui.wait_contains("Focus branches", GIT_WAIT);
    focused_tui.keys("keep");
    focused_tui.enter();
    focused_tui.wait_pred(
        |screen| {
            !screen.contains("Focus branches")
                && screen.contains("keep-leaf-commit")
                && !screen.contains("noise-leaf-commit")
                && screen.contains("O   clear focus")
                && screen.contains("[+feature/keep]")
        },
        "graph focus applied; O still bound as clear focus",
        GIT_WAIT,
    );

    focused_tui.key('/');
    focused_tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "")
                && screen.contains("keep-leaf-commit")
                && !screen.contains("noise-leaf-commit")
        },
        "/ on the focused graph opens SEARCH; focus filter stays",
        WAIT,
    );
    focused_tui.shift_letter('O');
    focused_tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "O")
                && search_did_not_fire_global_shift(screen)
                && screen.contains("keep-leaf-commit")
                && !screen.contains("noise-leaf-commit")
                && screen.contains("[+feature/keep]")
                && !screen.contains("o▏")
        },
        "CSI-u Shift+O types O into SEARCH; it must not restore --all",
        WAIT,
    );
}
