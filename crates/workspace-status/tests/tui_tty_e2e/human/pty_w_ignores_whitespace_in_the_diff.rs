use crate::harness::PtySession;
use crate::seed::{whitespace_workspace, INDENT_LINE, REGIONS_ALPHA};
use crate::support::{
    after_ack, open_command_palette, right_pane, tree_cursor_on, type_palette_filter, WAIT,
};

const PALETTE_ROW: &str = "Ignore / show whitespace";

/// The first right-pane row: the diff header (`{path}  inline|split · …`).
fn diff_header(screen: &str) -> String {
    right_pane(screen).lines().next().unwrap_or("").to_string()
}

/// `indent.txt` focused, its re-indent hunk painted, no `-w` mark.
fn indent_hunk_shown(screen: &str) -> bool {
    let header = diff_header(screen);
    tree_cursor_on(screen, "indent.txt")
        && header.contains("indent.txt")
        && !header.contains("· -w")
        && right_pane(screen).contains(INDENT_LINE)
        && right_pane(screen).contains("UNSTAGED")
        && !screen.contains(PALETTE_ROW)
}

/// `indent.txt` focused, loaded with `-w`: the header has the mark and the
/// whitespace-only hunk is gone (git's empty diff).
fn indent_hunk_hidden(screen: &str) -> bool {
    let header = diff_header(screen);
    tree_cursor_on(screen, "indent.txt")
        && header.contains("indent.txt")
        && header.contains("· -w")
        && !right_pane(screen).contains(INDENT_LINE)
        && right_pane(screen).contains("(no diff)")
        && !screen.contains(PALETTE_ROW)
}

/// `regions.txt` focused with `-w`: its real change still shows.
fn regions_hunk_with_dash_w(screen: &str) -> bool {
    let header = diff_header(screen);
    tree_cursor_on(screen, "regions.txt")
        && header.contains("regions.txt")
        && header.contains("· -w")
        && right_pane(screen).contains(REGIONS_ALPHA)
}

/// `regions.txt` focused without `-w`.
fn regions_hunk_without_dash_w(screen: &str) -> bool {
    let header = diff_header(screen);
    tree_cursor_on(screen, "regions.txt")
        && header.contains("regions.txt")
        && !header.contains("· -w")
        && right_pane(screen).contains(REGIONS_ALPHA)
        && !screen.contains(PALETTE_ROW)
}

/// `:` + `query` + Enter: the palette row the query lands on runs.
fn run_palette_row(tui: &mut PtySession, query: &str) {
    open_command_palette(tui);
    type_palette_filter(tui, query);
    let filter = format!(">{query}");
    tui.wait_pred(
        |screen| screen.contains(PALETTE_ROW) && screen.contains(&filter),
        &format!("palette filter `{query}` shows the Ignore / show whitespace row"),
        WAIT,
    );
    tui.enter();
}

/// `w` ignores whitespace in the diff; a second `w` shows it again. The
/// palette row does the same.
///
/// `indent.txt` changes by indentation only, `regions.txt` by text. On
/// launch the `indent.txt` hunk shows. `w` reloads it with `-w`: the
/// header gets `· -w`, the hunk is gone, and the status says
/// `whitespace hidden`. A second `w` brings the hunk back
/// (`whitespace shown`). `:` `whitespace` Enter hides it again, and with
/// `-w` on the `regions.txt` hunk (a real change) still shows; the palette
/// row then shows whitespace again. `:` `-w` Enter (the alias, which also
/// hits Close tab's `Ctrl-w` chip) flips it the same way twice. A mark
/// without the reload, a reload that drops the real change, or a `-w`
/// that runs Close tab, cannot pass.
#[test]
fn pty_w_ignores_whitespace_in_the_diff() {
    let (_root, workspace) = whitespace_workspace("ws-tui-tty-ignore-whitespace");
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        indent_hunk_shown,
        "launch shows the whitespace-only indent.txt hunk",
        WAIT,
    );

    tui.key('w');
    tui.wait_pred(
        after_ack("whitespace hidden", indent_hunk_hidden),
        "w reloads indent.txt with -w: header · -w, hunk gone",
        WAIT,
    );

    tui.key('w');
    tui.wait_pred(
        after_ack("whitespace shown", indent_hunk_shown),
        "second w shows the indent.txt hunk again, no · -w",
        WAIT,
    );

    run_palette_row(&mut tui, "whitespace");
    tui.wait_pred(
        after_ack("whitespace hidden", indent_hunk_hidden),
        "the palette row hides whitespace like w",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        regions_hunk_with_dash_w,
        "with -w the regions.txt real change still shows",
        WAIT,
    );

    run_palette_row(&mut tui, "whitespace");
    tui.wait_pred(
        after_ack("whitespace shown", regions_hunk_without_dash_w),
        "the palette row shows whitespace again",
        WAIT,
    );

    // `-w` also hits Close tab's `Ctrl-w` chip; the exact alias wins.
    run_palette_row(&mut tui, "-w");
    tui.wait_pred(
        after_ack("whitespace hidden", regions_hunk_with_dash_w),
        "`:` `-w` Enter runs Ignore / show whitespace, not Close tab",
        WAIT,
    );
    run_palette_row(&mut tui, "-w");
    tui.wait_pred(
        after_ack("whitespace shown", regions_hunk_without_dash_w),
        "a second `:` `-w` Enter shows whitespace again",
        WAIT,
    );
}
