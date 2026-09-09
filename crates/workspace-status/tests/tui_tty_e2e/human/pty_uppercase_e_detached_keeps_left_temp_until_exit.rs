use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

fn stub_argv(marker_body: &str) -> Vec<&str> {
    marker_body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn wait_file(path: &Path, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        if let Ok(body) = fs::read_to_string(path) {
            if !body.trim().is_empty() {
                return body;
            }
        }
        if start.elapsed() >= timeout {
            panic!("timed out waiting for {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Detached GUI `E` (`cursor` in the DETACHED name list). TUI stays mounted.
/// LEFT HEAD temp must exist while the child holds, then vanish after the
/// child exits. A spawn that deletes temps when the CLI returns (~2s without
/// `--wait`) fails. Help row is unchanged.
///
/// The stub file name is `cursor` so `is_detached_editor` is true. Config
/// is `cursor --diff --wait`. Launch starts on README.md. The claim uses a
/// second dirty file so a tool that always opens the first path cannot pass.
#[test]
fn pty_uppercase_e_detached_keeps_left_temp_until_exit() {
    let (_root, workspace) = daily_workspace();
    fs::write(
        workspace.join("app").join("edit-target.txt"),
        "unique-edit-target-body\n",
    )
    .unwrap();
    let shim_dir = workspace.join(".e2e-diff-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let stub = shim_dir.join("cursor");
    let marker = shim_dir.join("opened");
    let hold = shim_dir.join("hold");
    fs::write(&hold, "1\n").unwrap();
    fs::write(
        &stub,
        "#!/bin/sh\n\
         marker=\"${WS_STATUS_E2E_DIFF_MARKER:?}\"\n\
         hold=\"${WS_STATUS_E2E_DIFF_HOLD:?}\"\n\
         printf '%s\\n' \"$@\" > \"$marker\"\n\
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
    let tool = format!("{} --diff --wait", stub.display());
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
            screen.contains("opened diff edit-target.txt")
                && !screen.contains("STUB-DIFF-CHROME")
                && !screen.contains("diffed edit-target.txt")
                && !screen.contains("edited edit-target.txt")
                && tree_cursor_on(screen, "edit-target.txt")
                && screen.contains("unique-edit-target-body")
                && screen.contains("? help")
                && !screen.contains(" · full")
        },
        "detached E stays mounted with opened-diff toast (TTY remount or no-op fails)",
        WAIT,
    );
    let marker_body = wait_file(&marker, WAIT);
    let args = stub_argv(&marker_body);
    eprintln!(
        "detached cursor argv ({} paths):\n{marker_body}",
        args.len()
    );
    assert!(
        args.iter().any(|a| *a == "--diff") && args.iter().any(|a| *a == "--wait"),
        "detached config must pass --diff --wait:\n{marker_body}"
    );
    assert!(
        args.len() >= 4,
        "diff tool must receive flags plus two path arguments, got {}:\n{marker_body}",
        args.len()
    );
    let focused = workspace.join("app").join("edit-target.txt");
    assert!(
        args.iter().any(|a| Path::new(a) == focused.as_path()),
        "stub cursor must receive the focused worktree file:\n{marker_body}"
    );
    assert!(
        !marker_body.contains("README.md"),
        "README.md must not be a path:\n{marker_body}"
    );
    let left_temp = args
        .iter()
        .copied()
        .find(|a| Path::new(a) != focused.as_path() && a.contains("workspace-status-ext-diff"))
        .map(Path::new)
        .expect("LEFT temp argv");
    assert!(
        left_temp.exists(),
        "LEFT HEAD temp must survive while the detached child holds:\n{}",
        left_temp.display()
    );
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        left_temp.exists(),
        "LEFT HEAD temp must still exist after 2s (cleanup-on-CLI-return fails):\n{}",
        left_temp.display()
    );
    fs::remove_file(&hold).unwrap();

    let gone_start = Instant::now();
    loop {
        if !left_temp.exists() {
            break;
        }
        if gone_start.elapsed() >= GIT_WAIT {
            panic!(
                "LEFT HEAD temp still exists after the detached child exited:\n{}",
                left_temp.display()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    tui.wait_pred(
        |screen| {
            screen.contains("opened diff edit-target.txt")
                && !screen.contains("diffed edit-target.txt")
                && !screen.contains("edited edit-target.txt")
                && tree_cursor_on(screen, "edit-target.txt")
                && !tree_cursor_on(screen, "README.md")
                && tree_has(screen, "README.md")
                && screen.contains("unique-edit-target-body")
                && screen.contains("/edit-target")
                && screen.contains("? help")
                && !screen.contains(" · full")
                && !screen.contains("[workspace]")
        },
        "TUI stays mounted on the same focused file after the detached child exits",
        WAIT,
    );
    let readme = fs::read_to_string(workspace.join("app").join("README.md")).unwrap();
    let target = fs::read_to_string(&focused).unwrap();
    assert!(
        !readme.contains("e2e-editor-marker") && !readme.contains("STUB-DIFF"),
        "README.md must stay closed:\n{readme}"
    );
    assert_eq!(
        target, "unique-edit-target-body\n",
        "stub cursor must not be required to mutate the focused file:\n{target}"
    );
}
