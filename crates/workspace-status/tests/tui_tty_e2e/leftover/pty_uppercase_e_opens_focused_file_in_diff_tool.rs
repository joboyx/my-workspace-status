use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

fn help_lists_e_e_diff_tool(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("GIT")
        && compact.contains("e E")
        && (compact.contains("open in diff tool") || compact.contains("open in editor"))
        && compact.contains("Ctrl-o")
        && compact.contains("full-file")
}

fn stub_argv(marker_body: &str) -> Vec<&str> {
    marker_body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// `E` hands LEFT+RIGHT to config `diffTool`. Help lists that next to `e`.
/// Ctrl-o is full-file and must not pass as this key. `e` stays editor.
///
/// Launch starts on README.md. The claim uses a second dirty file so a
/// tool that always opens the first path cannot pass. A TTY stub paints
/// chrome, holds the TTY, then exits 0 (a live vimdiff session would hang
/// `cargo test`). After return, the TUI remounts on the same file. A
/// no-op, a toast with no spawn, one path, or README-only argv fails.
#[test]
fn pty_uppercase_e_opens_focused_file_in_diff_tool() {
    let (_root, workspace) = daily_workspace();
    fs::write(
        workspace.join("app").join("edit-target.txt"),
        "unique-edit-target-body\n",
    )
    .unwrap();
    let shim_dir = workspace.join(".e2e-diff-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let stub = shim_dir.join("stub-diff-tool");
    let marker = shim_dir.join("opened");
    let hold = shim_dir.join("hold");
    fs::write(&hold, "1\n").unwrap();
    fs::write(
        &stub,
        "#!/bin/sh\n\
         marker=\"${WS_STATUS_E2E_DIFF_MARKER:?}\"\n\
         hold=\"${WS_STATUS_E2E_DIFF_HOLD:?}\"\n\
         printf '%s\\n' \"$@\" > \"$marker\"\n\
         printf '\\n===== STUB-DIFF-CHROME =====\\nopening %s\\n===== STUB-DIFF-CHROME =====\\n' \"$*\"\n\
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
    let tool = stub.display().to_string();
    assert!(
        !tool.contains('"') && !tool.contains('\\'),
        "stub path must be JSON-safe: {tool}"
    );
    fs::write(
        workspace.join(".workspace-status-config.json"),
        format!("{{\n  \"ignoredRepos\": [\"notes\"],\n  \"diffTool\": \"{tool}\"\n}}\n"),
    )
    .unwrap();
    let marker_s = marker.display().to_string();
    let hold_s = hold.display().to_string();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_E2E_DIFF_MARKER", marker_s.as_str()),
            ("WS_STATUS_E2E_DIFF_HOLD", hold_s.as_str()),
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
        |screen| help_lists_e_e_diff_tool(screen) && screen.contains("MOVE"),
        "help GIT lists e E / open in diff tool; Ctrl-o stays full-file",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("open in editor")
                && !screen.contains("open in diff tool")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
        },
        "Esc closes help so E is the diff-tool key, not a help query",
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
        "search focuses the unique file (E on README would open the wrong path)",
        GIT_WAIT,
    );

    tui.key('E');
    tui.wait_pred(
        |screen| {
            screen.contains("STUB-DIFF-CHROME")
                && screen.contains("edit-target.txt")
                && !screen.contains("diffed edit-target.txt")
                && !screen.contains("edited edit-target.txt")
                && !screen.contains(" · full")
        },
        "E must paint diff-tool chrome for the focused path (a no-op stays idle; Ctrl-o paints full-file)",
        WAIT,
    );
    let marker_body = fs::read_to_string(&marker).unwrap_or_default();
    let args = stub_argv(&marker_body);
    eprintln!("diffTool argv ({} paths):\n{marker_body}", args.len());
    assert!(
        args.len() >= 2,
        "diff tool must receive at least two path arguments, got {}:\n{marker_body}",
        args.len()
    );
    let focused = workspace.join("app").join("edit-target.txt");
    assert!(
        args.iter().any(|a| Path::new(a) == focused.as_path()),
        "stub diffTool must receive the focused worktree file:\n{marker_body}"
    );
    assert!(
        !marker_body.contains("README.md"),
        "README.md must not be the only path (or any path):\n{marker_body}"
    );
    let other = args
        .iter()
        .copied()
        .find(|a| Path::new(a) != focused.as_path());
    assert!(
        other.is_some_and(|a| a.contains("workspace-status-ext-diff")),
        "the other argv path must be a temp under workspace-status-ext-diff:\n{marker_body}"
    );
    fs::remove_file(&hold).unwrap();

    tui.wait_pred(
        |screen| {
            screen.contains("diffed edit-target.txt")
                && !screen.contains("edited edit-target.txt")
                && !screen.contains("STUB-DIFF-CHROME")
                && tree_cursor_on(screen, "edit-target.txt")
                && !tree_cursor_on(screen, "README.md")
                && tree_has(screen, "README.md")
                && screen.contains("unique-edit-target-body")
                && screen.contains("/edit-target")
                && screen.contains("? help")
                && !screen.contains(" · full")
                && !screen.contains("[workspace]")
        },
        "TUI remounts on the same focused file after the diff tool exits (a toast-only no-op never spawns chrome)",
        GIT_WAIT,
    );
    let readme = fs::read_to_string(workspace.join("app").join("README.md")).unwrap();
    let target = fs::read_to_string(&focused).unwrap();
    assert!(
        !readme.contains("e2e-editor-marker") && !readme.contains("STUB-DIFF"),
        "README.md must stay closed:\n{readme}"
    );
    assert_eq!(
        target, "unique-edit-target-body\n",
        "stub diffTool must not be required to mutate the focused file:\n{target}"
    );
}
