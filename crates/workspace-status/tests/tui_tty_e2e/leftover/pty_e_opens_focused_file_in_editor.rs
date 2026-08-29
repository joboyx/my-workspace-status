use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

fn help_lists_e_open_in_editor(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("GIT")
        && compact.contains("open in editor")
        && compact.contains("Ctrl-o")
        && compact.contains("full-file")
}

/// `e` hands the focused workspace file to `$EDITOR`. Help lists that as
/// open in editor. Ctrl-o is full-file and must not pass as this key.
///
/// Launch starts on README.md. The claim uses a second dirty file so an
/// editor that always opens the first path cannot pass. A TTY stub paints
/// chrome, holds the TTY, then exits 0 (a live vim session would hang
/// `cargo test`). After return, the TUI remounts on the same file. A no-op,
/// a toast with no spawn, or the wrong path fails.
#[test]
fn pty_e_opens_focused_file_in_editor() {
    let (_root, workspace) = daily_workspace();
    fs::write(
        workspace.join("app").join("edit-target.txt"),
        "unique-edit-target-body\n",
    )
    .unwrap();
    let shim_dir = workspace.join(".e2e-editor-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let stub = shim_dir.join("stub-editor");
    let marker = shim_dir.join("opened");
    let hold = shim_dir.join("hold");
    fs::write(&hold, "1\n").unwrap();
    fs::write(
        &stub,
        "#!/bin/sh\n\
         marker=\"${WS_STATUS_E2E_EDITOR_MARKER:?}\"\n\
         hold=\"${WS_STATUS_E2E_EDITOR_HOLD:?}\"\n\
         file=\"\"\n\
         for a in \"$@\"; do\n\
           file=\"$a\"\n\
         done\n\
         printf '%s\\n' \"$@\" > \"$marker\"\n\
         printf '\\n===== STUB-EDITOR-CHROME =====\\nopening %s\\n===== STUB-EDITOR-CHROME =====\\n' \"$file\"\n\
         if [ -n \"$file\" ]; then\n\
           printf '\\ne2e-editor-marker\\n' >> \"$file\"\n\
         fi\n\
         i=0\n\
         while [ -f \"$hold\" ] && [ \"$i\" -lt 300 ]; do\n\
           sleep 0.1\n\
           i=$((i + 1))\n\
         done\n\
         exit 0\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    let editor = stub.display().to_string();
    let marker_s = marker.display().to_string();
    let hold_s = hold.display().to_string();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("EDITOR", editor.as_str()),
            ("WS_STATUS_E2E_EDITOR_MARKER", marker_s.as_str()),
            ("WS_STATUS_E2E_EDITOR_HOLD", hold_s.as_str()),
        ],
    );
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && tree_has(screen, "edit-target.txt")
                && screen.contains("UNSTAGED")
                && !tree_cursor_on(screen, "edit-target.txt")
                && !screen.contains("unique-edit-target-body")
                && !screen.contains(" · full")
                && screen.contains("? help")
        },
        "launch cursor is README, not the unique dirty file",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| help_lists_e_open_in_editor(screen) && screen.contains("MOVE"),
        "help GIT lists e as open in editor; Ctrl-o stays full-file",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("open in editor")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
        },
        "Esc closes help so e is the editor key, not a help query",
        WAIT,
    );

    tui.search("edit-target");
    tui.wait_pred(
        |screen| {
            screen.contains("/edit-target")
                && tree_cursor_on(screen, "edit-target.txt")
                && !tree_cursor_on(screen, "README.md")
                && screen.contains("unique-edit-target-body")
                && screen.contains("NEW")
                && !screen.contains("UNSTAGED")
                && !screen.contains(" · full")
        },
        "search focuses the unique file (e on README would open the wrong path)",
        GIT_WAIT,
    );

    tui.key('e');
    tui.wait_pred(
        |screen| {
            screen.contains("STUB-EDITOR-CHROME")
                && screen.contains("edit-target.txt")
                && !screen.contains("edited edit-target.txt")
                && !screen.contains(" · full")
        },
        "e must paint editor chrome for the focused path (a no-op stays idle; Ctrl-o paints full-file)",
        WAIT,
    );
    let marker_body = fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        marker_body.contains("edit-target.txt") && !marker_body.contains("README.md"),
        "stub EDITOR must receive the focused file, not README.md:\n{marker_body}"
    );
    fs::remove_file(&hold).unwrap();

    tui.wait_pred(
        |screen| {
            screen.contains("edited edit-target.txt")
                && !screen.contains("edited README.md")
                && !screen.contains("STUB-EDITOR-CHROME")
                && tree_cursor_on(screen, "edit-target.txt")
                && !tree_cursor_on(screen, "README.md")
                && tree_has(screen, "README.md")
                && screen.contains("unique-edit-target-body")
                && screen.contains("e2e-editor-marker")
                && screen.contains("/edit-target")
                && screen.contains("? help")
                && !screen.contains(" · full")
                && !screen.contains("[workspace]")
        },
        "TUI remounts on the same focused file after the editor exits (a toast-only no-op never writes the marker line)",
        GIT_WAIT,
    );
    let readme = fs::read_to_string(workspace.join("app").join("README.md")).unwrap();
    let target = fs::read_to_string(workspace.join("app").join("edit-target.txt")).unwrap();
    assert!(
        !readme.contains("e2e-editor-marker"),
        "README.md must stay closed:\n{readme}"
    );
    assert!(
        target.contains("e2e-editor-marker") && target.contains("unique-edit-target-body"),
        "stub EDITOR must append to the focused file:\n{target}"
    );
}
