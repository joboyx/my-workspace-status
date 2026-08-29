use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Tokyo Night `pills.filter.bg` (`#bb9af7`). Help `/` highlight uses this.
const HELP_SEARCH_FILTER_BG: (u8, u8, u8) = (0xbb, 0x9a, 0xf7);

/// Pane `/` typing chrome. Distinct from help `search focused pane (Enter arms)`.
fn pane_search_prompt(screen: &str) -> bool {
    screen.contains("SEARCH")
        && screen.contains("Enter arms query")
        && screen.contains("n/N after Enter")
}

fn help_overlay_open(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("GIT")
        && screen.contains("VIEW")
        && screen.contains("stage scope")
        && screen.contains("search focused pane")
        && screen.contains("press twice")
        && screen.contains("never quit")
        && screen.contains("next / prev match")
}

fn help_searching(screen: &str, query: &str) -> bool {
    help_overlay_open(screen)
        && screen.contains(&format!("HELP  /{query}"))
        && screen.contains("Esc clears search")
        && !screen.contains("/ search help")
        && !pane_search_prompt(screen)
}

fn help_quit_rows_highlighted(tui: &PtySession) -> bool {
    let (r, g, b) = HELP_SEARCH_FILTER_BG;
    tui.needle_has_bg("press twice", r, g, b)
        && tui.needle_has_bg("never quit", r, g, b)
        && tui.needle_lacks_bg("stage scope", r, g, b)
}

fn help_quit_rows_unhighlighted(tui: &PtySession) -> bool {
    let (r, g, b) = HELP_SEARCH_FILTER_BG;
    tui.needle_lacks_bg("press twice", r, g, b)
        && tui.needle_lacks_bg("never quit", r, g, b)
        && tui.needle_lacks_bg("stage scope", r, g, b)
}

/// Help `/` highlights matching overlay rows. Enter must not arm pane search.
///
/// Docs: while `?` help is open, `/` is overlay-local (highlight only; rows
/// stay visible; no Enter-arm; no `n`/`N` next/prev). A no-op `/`, an Enter
/// that opens pane SEARCH, or a close that leaves `/{query}` armed cannot
/// pass. Glyphs-only screen delta is not enough: matching `quit` rows must
/// use the filter background, and non-matching rows must stay unhighlighted.
#[test]
fn pty_help_enter_does_not_arm_pane_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_THEME", "tokyo-night")]);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("+dirty", GIT_WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("focus right")
                && !pane_search_prompt(screen)
                && !screen.contains("MOVE")
        },
        "launch focuses README with no SEARCH prompt",
        GIT_WAIT,
    );

    tui.key('?');
    // After the overlay row-budget fix, help can cover the whole tree pane.
    // Assert overlay + Enter-arm here; README cursor only after help closes.
    tui.wait_pred(
        |screen| {
            help_overlay_open(screen)
                && screen.contains("/ search help")
                && !pane_search_prompt(screen)
                && !screen.contains("HELP  /")
        },
        "help overlay lists MOVE/GIT/VIEW and idle `/ search help`",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            help_searching(screen, "")
                && screen.contains("HELP  /▏")
                && !screen.contains("HELP  /quit")
        },
        "help `/` opens overlay search (a no-op keeps `/ search help`; pane `/` paints SEARCH)",
        WAIT,
    );

    tui.keys("quit");
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_searching(&screen, "quit")
                && screen.contains("HELP  /quit▏")
                && !screen.contains("HELP  /quitn")
                && help_quit_rows_highlighted(&tui)
        },
        "typing quit highlights matching help rows; non-matching rows stay visible",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_searching(&screen, "quit")
                && screen.contains("HELP  /quit▏")
                && !screen.contains("HELP  /quitn")
                && help_quit_rows_highlighted(&tui)
                && !screen.contains("[README")
                && !pane_search_prompt(&screen)
        },
        "Enter keeps help highlight only (pane SEARCH / n/N / drill cannot pass)",
        WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_searching(&screen, "quitn")
                && screen.contains("HELP  /quitn▏")
                && help_quit_rows_unhighlighted(&tui)
                && !pane_search_prompt(&screen)
        },
        "n after Enter appends to help search (armed n/N would leave /quit and may move the cursor)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_overlay_open(&screen)
                && screen.contains("/ search help")
                && !screen.contains("HELP  /")
                && !screen.contains("Esc clears search")
                && help_quit_rows_unhighlighted(&tui)
                && !pane_search_prompt(&screen)
        },
        "Esc clears help search; help stays (pane `/` would keep SEARCH or /quitn)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("HELP  /")
                && !screen.contains("/quit")
                && !pane_search_prompt(screen)
                && tree_has(screen, "README.md")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("+dirty")
                && screen.contains("? help")
                && screen.contains("focus right")
        },
        "second Esc closes help with pane search still unarmed",
        WAIT,
    );
}
