use crate::harness::PtySession;
use crate::seed::{daily_workspace, seed_repo};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Armed `/main` chip, idle hints, left focus. Tab paints `drill` / `[app]`.
fn armed_tree_search_left(screen: &str) -> bool {
    screen.contains("/main")
        && screen.contains("? help")
        && screen.contains("focus right")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("drill")
        && !screen.contains("[app]")
        && !screen.contains("[lib]")
        && !screen.contains("[tools]")
        && !screen.contains("[workspace]")
}

/// Cursor, breadcrumb, and graph subject for one `/main` hit.
///
/// `seed {name}` is unique per checkout. `Working tree clean` is not
/// (`lib` and `tools` are both clean). A no-op, a skipped hit, or Tab
/// (`[name]` / `drill`) cannot pass.
fn search_hit_on(screen: &str, name: &str, graph_subject: &str) -> bool {
    let crumb = format!("workspace › {name}");
    let right_crumb = format!("workspace › [{name}]");
    armed_tree_search_left(screen)
        && tree_cursor_on(screen, name)
        && screen.contains(&crumb)
        && !screen.contains(&right_crumb)
        && screen.contains(graph_subject)
}

/// Help `n N`, then armed `/` `n` / CSI-u `N` next / prev on that pane.
///
/// Docs + MOVE: next / prev match after Enter. Tab is other pane. While
/// typing, `n` appends (`mainn`) and must not next. `/` steps from the
/// cursor like vim: from the launch README (under `app`) the first `main`
/// hit below is `lib`, so `app` is only reached by a wrap. Three `main`
/// checkouts so wrap-`n` cannot pass as `N`. Cursor bar, breadcrumb, and
/// `seed {name}` must all move. Stay armed and left (`/main`,
/// `focus right`, no `[…]`).
#[test]
fn pty_n_and_n_pane_next_prev() {
    let (_root, workspace) = daily_workspace();
    // Third clean `main` checkout so next and prev from `lib` diverge.
    seed_repo(&workspace, "tools", "main", false);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "No updates")
                && !tree_has(screen, "lib")
                && !tree_has(screen, "tools")
                && tree_cursor_on(screen, "README.md")
        },
        "lib and tools stay under the folded No-updates group",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && screen.contains("n   N")
                && screen.contains("next / prev match")
                && screen.contains("search focused")
                && screen.contains("Tab")
                && screen.contains("other pane")
        },
        "help MOVE lists n/N next/prev after Enter; Tab is other pane",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("next / prev match")
                && tree_has(screen, "README.md")
                && screen.contains("focus right")
        },
        "Esc closes help so n/N are pane search, not help keys",
        WAIT,
    );

    tui.key('/');
    tui.keys("main");
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH")
                && screen.contains("Enter arms query")
                && screen.contains("n/N after Enter")
                && !screen.contains("/main")
                && tree_cursor_on(screen, "lib")
                && !tree_cursor_on(screen, "app")
                && screen.contains("workspace › lib")
                && screen.contains("seed lib")
                && !screen.contains("[lib]")
        },
        "typing /main jumps to lib, the first hit below README (not app above)",
        GIT_WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH")
                && screen.contains("mainn")
                && screen.contains("no match")
                && tree_cursor_on(screen, "README.md")
                && !tree_has(screen, "lib")
                && !tree_cursor_on(screen, "tools")
                && !screen.contains("[app]")
                && !screen.contains("drill")
        },
        "n while typing appends; no hit puts the cursor back on README, not next",
        WAIT,
    );
    tui.send_bytes(b"\x7f");
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH") && !screen.contains("mainn") && tree_cursor_on(screen, "lib")
        },
        "Backspace drops the extra n so Enter can arm /main",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "lib", "seed lib")
                && screen.contains("Working tree clean")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "tools")
        },
        "Enter arms /main on lib, where the preview landed",
        GIT_WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "tools", "seed tools")
                && !tree_cursor_on(screen, "lib")
                && !tree_cursor_on(screen, "app")
                && !screen.contains("seed lib")
                && !screen.contains("workspace › lib")
        },
        "n jumps to tools (a no-op stays on lib; wrap-n would land on app)",
        GIT_WAIT,
    );

    tui.key('n');
    // The wrap notice is a timed toast: check it before the graph loads.
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && screen.contains("search wrapped to top"),
        "second n wraps to app and toasts search wrapped to top",
        WAIT,
    );
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "app", "seed app")
                && screen.contains("Uncommitted changes")
                && !tree_cursor_on(screen, "lib")
                && !tree_cursor_on(screen, "tools")
                && !screen.contains("seed tools")
                && !screen.contains("workspace › tools")
        },
        "second n lands on app (a no-op stays on tools)",
        GIT_WAIT,
    );

    tui.shift_letter('N');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "tools") && screen.contains("search wrapped to bottom"),
        "CSI-u N wraps back to tools and toasts search wrapped to bottom",
        WAIT,
    );
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "tools", "seed tools")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "lib")
                && !screen.contains("seed app")
                && !screen.contains("workspace › app")
        },
        "CSI-u N lands on tools (a no-op stays on app; n would land on lib)",
        GIT_WAIT,
    );

    tui.shift_letter('N');
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "lib", "seed lib")
                && !tree_cursor_on(screen, "tools")
                && !tree_cursor_on(screen, "app")
                && !screen.contains("seed tools")
                && !screen.contains("workspace › tools")
                && !screen.contains("workspace › app")
        },
        "second CSI-u N returns to lib (a no-op stays on tools; wrap would land on app)",
        GIT_WAIT,
    );
}
