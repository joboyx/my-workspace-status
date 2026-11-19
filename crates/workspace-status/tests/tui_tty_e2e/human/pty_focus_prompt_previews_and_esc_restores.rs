use crate::harness::PtySession;
use crate::seed::tree_focus_workspace;
use crate::support::{
    after_ack, focus_feature_from_app, open_command_palette, tree_focused_on_abc,
    tree_line_containing, type_palette_filter, workspace_row_has, WAIT,
};

/// The focus prompt is open with `query` as its draft and `count` as its
/// `k/n repos in focus` line.
fn prompt_open(screen: &str, query: &str, count: &str) -> bool {
    screen.contains("Focus tree")
        && screen.contains(&format!("query: {query}"))
        && screen.contains(count)
        && screen.contains("Enter apply · Esc restore · empty clears")
}

/// The palette row `Focus: type a query…` opens a prompt prefilled with the
/// applied focus; typing previews the tree live; Esc restores the focus that
/// was applied before the prompt opened.
///
/// `*` on `app` applies `ABC-123`. The prompt opens with `query: ABC-123`
/// and `2/4 repos in focus`. Typing `, lib` previews `ABC-123, lib`: the
/// chip reads `(o) ABC-123, lib · 3/4 repos` and the outside row
/// `1 outside focus · 1 dirty`. Esc shows `focus unchanged`, closes the
/// prompt, and the `ABC-123` chip and outside row come back.
#[test]
fn pty_focus_prompt_previews_and_esc_restores() {
    let (_root, workspace) = tree_focus_workspace();
    let mut tui = PtySession::open(&workspace);
    focus_feature_from_app(&mut tui);

    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "type a query");
    tui.wait_contains("Focus: type a query…", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| {
            !screen.contains("Enter run")
                && prompt_open(screen, "ABC-123", "2/4 repos in focus")
                && tree_focused_on_abc(screen)
        },
        "Enter opens the focus prompt prefilled with the applied ABC-123",
        WAIT,
    );

    type_palette_filter(&mut tui, ", lib");
    tui.wait_pred(
        |screen| {
            prompt_open(screen, "ABC-123, lib", "3/4 repos in focus")
                && workspace_row_has(screen, "(o) ABC-123, lib · 3/4 repos")
                && tree_line_containing(screen, "1 outside focus · 1 dirty").is_some()
        },
        "typing `, lib` previews the draft: chip and outside row follow it",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        after_ack("focus unchanged", |screen| {
            !screen.contains("Focus tree") && tree_focused_on_abc(screen)
        }),
        "Esc closes the prompt and restores the ABC-123 focus",
        WAIT,
    );
}
