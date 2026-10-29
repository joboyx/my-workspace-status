//! CI watch for cargo-dist generate and TTY update-check isolation.
//!
//! `dist generate` rewrites `.github/workflows/release.yml` and drops the
//! host-job git-cliff steps plus `workflow_dispatch`. A TTY demo-clip or e2e
//! launch without `WS_STATUS_UPDATE_CHECK_STORE` writes
//! `$XDG_STATE_HOME/my-workspace-status/update-check.json` (the operator
//! last-check file) and can block mount on the GitHub Release prompt.
//! A TTY spawn that inherits `WS_STATUS_WORKSPACE` retargets config and
//! git writes away from the fixture. A TTY spawn that inherits
//! `XDG_CONFIG_HOME` / `HOME` reads the operator user config file
//! (`my-workspace-status/config.json`), which can change the fixture's
//! settings. A TTY spawn without `WS_STATUS_UI_STORE` reads and, on a theme
//! pick or a sparkline toggle, writes
//! `$XDG_STATE_HOME/my-workspace-status/ui-state.json` (the operator UI
//! state: last-used theme and activity sparkline), which can beat the
//! fixture's config theme or change its paint. The binary ignores the old
//! `WS_STATUS_THEME_STORE`, so a spawn path must not keep naming it. These
//! tests fail until those hazards stay fixed. The tmux driver of the verify
//! skill (`scripts/verify-tui.sh`) is a TTY spawn path too, so it is in every
//! loop below. Recipe:
//! [docs/architecture.md](../../../docs/architecture.md).

const RELEASE_YML: &str = include_str!("../../../.github/workflows/release.yml");
const TAG_RELEASE_YML: &str = include_str!("../../../.github/workflows/tag-release.yml");
const DIST_WORKSPACE: &str = include_str!("../../../dist-workspace.toml");
const STILLS_SH: &str = include_str!("../../../scripts/capture-demo-stills.sh");
const VERIFY_SH: &str = include_str!("../../../scripts/verify-tui.sh");
const PTY_HARNESS: &str = include_str!("tui_tty_e2e/harness.rs");
const DESKTOP_HARNESS: &str = include_str!("tui_tty_e2e/desktop.rs");

fn assigns_update_check_store(src: &str) -> bool {
    src.contains("env(\"WS_STATUS_UPDATE_CHECK_STORE\"")
        || src.contains("export WS_STATUS_UPDATE_CHECK_STORE=")
}

fn assigns_ui_store(src: &str) -> bool {
    src.contains("env(\"WS_STATUS_UI_STORE\"") || src.contains("export WS_STATUS_UI_STORE=")
}

fn assigns_user_config_home(src: &str) -> bool {
    src.contains("env(\"XDG_CONFIG_HOME\"") || src.contains("export XDG_CONFIG_HOME=")
}

/// True when a line of `src` is an `unset` command that names `var`.
fn unsets_var(src: &str, var: &str) -> bool {
    src.lines().any(|line| {
        let mut words = line.split_whitespace();
        words.next() == Some("unset") && words.any(|word| word == var)
    })
}

fn isolates_workspace_env(src: &str) -> bool {
    src.contains("env_remove(\"WS_STATUS_WORKSPACE\")")
        || src.lines().any(|line| {
            line.contains("WS_STATUS_WORKSPACE")
                && (line.contains("env_remove") || line.contains("unset "))
        })
}

#[test]
fn isolated_store_assignment_rejects_comment_only() {
    assert!(!assigns_update_check_store(""));
    assert!(!assigns_update_check_store(
        "# Isolates WS_STATUS_UPDATE_CHECK_STORE under tmp"
    ));
    assert!(!assigns_update_check_store("WS_STATUS_UPDATE_CHECK_STORE"));
    assert!(assigns_update_check_store(
        "export WS_STATUS_UPDATE_CHECK_STORE=/tmp/update-check.json"
    ));
    assert!(assigns_update_check_store(
        "cmd.env(\"WS_STATUS_UPDATE_CHECK_STORE\", &store);"
    ));
    assert!(!unsets_var("", "GH_TOKEN"));
    assert!(!unsets_var("# unset GH_TOKEN", "GH_TOKEN"));
    assert!(!unsets_var("export GH_TOKEN=x", "GH_TOKEN"));
    assert!(!unsets_var("unset GH_TOKEN_OLD", "GH_TOKEN"));
    assert!(unsets_var(
        "unset NO_COLOR GH_TOKEN GITHUB_TOKEN",
        "GH_TOKEN"
    ));
    assert!(!isolates_workspace_env(""));
    assert!(!isolates_workspace_env("# drop WS_STATUS_WORKSPACE"));
    assert!(isolates_workspace_env("unset WS_STATUS_WORKSPACE"));
    assert!(isolates_workspace_env(
        "unset NO_COLOR FORCE_COLOR WS_STATUS_WORKSPACE"
    ));
    assert!(!isolates_workspace_env("export WS_STATUS_WORKSPACE=/tmp"));
    assert!(isolates_workspace_env(
        "cmd.env_remove(\"WS_STATUS_WORKSPACE\");"
    ));
    assert!(!assigns_ui_store(""));
    assert!(!assigns_ui_store("# Isolates WS_STATUS_UI_STORE under tmp"));
    assert!(!assigns_ui_store("WS_STATUS_UI_STORE"));
    assert!(!assigns_ui_store("unset WS_STATUS_UI_STORE"));
    assert!(!assigns_ui_store(
        "export WS_STATUS_THEME_STORE=/tmp/theme.json"
    ));
    assert!(assigns_ui_store(
        "export WS_STATUS_UI_STORE=/tmp/ui-state.json"
    ));
    assert!(assigns_ui_store(
        "cmd.env(\"WS_STATUS_UI_STORE\", &ui_store);"
    ));
    assert!(!assigns_user_config_home(""));
    assert!(!assigns_user_config_home("# Points XDG_CONFIG_HOME at tmp"));
    assert!(!assigns_user_config_home("unset XDG_CONFIG_HOME"));
    assert!(assigns_user_config_home(
        "export XDG_CONFIG_HOME=/tmp/config"
    ));
    assert!(assigns_user_config_home(
        "cmd.env(\"XDG_CONFIG_HOME\", &config_home);"
    ));
}

#[test]
fn dist_workspace_allows_dirty_ci() {
    assert!(
        DIST_WORKSPACE.contains("allow-dirty = [\"ci\"]"),
        "dist-workspace.toml must keep allow-dirty = [\"ci\"] so generate can leave hand-edits on release.yml"
    );
    assert!(
        !DIST_WORKSPACE
            .lines()
            .any(|line| line.trim_start().starts_with("dispatch-releases")),
        "dispatch-releases = true would drop tag-push; tag-release.yml needs tag-push plus workflow_dispatch"
    );
}

#[test]
fn release_workflow_keeps_dispatch_and_git_cliff() {
    assert!(
        RELEASE_YML.contains("\n  workflow_dispatch:\n"),
        "release.yml must keep on.workflow_dispatch after dist generate (tag-release.yml dispatches it)"
    );
    assert!(
        RELEASE_YML.contains("orhun/git-cliff-action"),
        "release.yml host job must keep git-cliff after dist generate"
    );
    assert!(
        RELEASE_YML.contains("fetch-depth: 0"),
        "release.yml host checkout must fetch full history for git-cliff --current"
    );
    assert!(
        RELEASE_YML.contains("fetch-tags: true"),
        "release.yml host checkout must fetch tags for git-cliff --current"
    );
    assert!(
        RELEASE_YML.contains("--current --strip header"),
        "release.yml must run git-cliff --current --strip header"
    );
    assert!(
        RELEASE_YML.contains("CHANGELOG_FILE"),
        "release.yml must prepend the git-cliff notes to the cargo-dist announcement"
    );
    assert!(
        RELEASE_YML.contains("dist generate"),
        "release.yml must document that host git-cliff steps are re-applied after dist generate"
    );
}

#[test]
fn tag_release_dispatches_generated_workflow() {
    assert!(
        TAG_RELEASE_YML.contains("gh workflow run release.yml"),
        "tag-release.yml must dispatch release.yml because a GITHUB_TOKEN tag push does not start other workflows"
    );
    assert!(
        TAG_RELEASE_YML.contains("workflow_dispatch"),
        "tag-release.yml must remind that release.yml keeps on.workflow_dispatch after dist generate"
    );
}

#[test]
fn tty_spawn_paths_isolate_update_check_store() {
    for (label, src) in [
        ("tui_tty_e2e/harness.rs", PTY_HARNESS),
        ("tui_tty_e2e/desktop.rs", DESKTOP_HARNESS),
        ("scripts/capture-demo-stills.sh", STILLS_SH),
        ("scripts/verify-tui.sh", VERIFY_SH),
    ] {
        assert!(
            assigns_update_check_store(src),
            "{label} launches a TTY TUI and must assign WS_STATUS_UPDATE_CHECK_STORE \
             (or the default XDG update-check.json is written)"
        );
    }
    assert!(
        PTY_HARNESS.contains("write_fresh_update_check"),
        "PTY harness must stamp a fresh lastCheckUnix so the 6h window is not due"
    );
    assert!(
        DESKTOP_HARNESS.contains("write_fresh_update_check"),
        "desktop harness must stamp a fresh lastCheckUnix so the 6h window is not due"
    );
    assert!(
        STILLS_SH.contains("lastCheckUnix"),
        "capture-demo-stills.sh must stamp a fresh lastCheckUnix so the 6h window is not due"
    );
    assert!(
        VERIFY_SH.contains("lastCheckUnix"),
        "verify-tui.sh must stamp a fresh lastCheckUnix so the 6h window is not due"
    );
}

#[test]
fn tty_spawn_paths_isolate_user_config() {
    for (label, src) in [
        ("tui_tty_e2e/harness.rs", PTY_HARNESS),
        ("tui_tty_e2e/desktop.rs", DESKTOP_HARNESS),
        ("scripts/capture-demo-stills.sh", STILLS_SH),
        ("scripts/verify-tui.sh", VERIFY_SH),
    ] {
        assert!(
            assigns_user_config_home(src),
            "{label} launches a TTY TUI and must assign XDG_CONFIG_HOME to a temp dir \
             (or the operator user config file changes the fixture's settings)"
        );
    }
}

#[test]
fn tty_spawn_paths_isolate_workspace_env() {
    for (label, src) in [
        ("tui_tty_e2e/harness.rs", PTY_HARNESS),
        ("tui_tty_e2e/desktop.rs", DESKTOP_HARNESS),
        ("scripts/capture-demo-stills.sh", STILLS_SH),
        ("scripts/verify-tui.sh", VERIFY_SH),
    ] {
        assert!(
            isolates_workspace_env(src),
            "{label} launches a TTY TUI and must drop parent WS_STATUS_WORKSPACE \
             (or the spawn uses the operator workspace instead of the fixture)"
        );
    }
}

#[test]
fn tty_spawn_paths_isolate_ui_store() {
    for (label, src) in [
        ("tui_tty_e2e/harness.rs", PTY_HARNESS),
        ("tui_tty_e2e/desktop.rs", DESKTOP_HARNESS),
        ("scripts/capture-demo-stills.sh", STILLS_SH),
        ("scripts/verify-tui.sh", VERIFY_SH),
    ] {
        assert!(
            assigns_ui_store(src),
            "{label} launches a TTY TUI and must assign WS_STATUS_UI_STORE \
             (or the operator UI-state file is read and written)"
        );
    }
}

#[test]
fn tty_spawn_paths_drop_old_theme_store_name() {
    for (label, src) in [
        ("tui_tty_e2e/harness.rs", PTY_HARNESS),
        ("tui_tty_e2e/desktop.rs", DESKTOP_HARNESS),
        ("scripts/capture-demo-stills.sh", STILLS_SH),
        ("scripts/verify-tui.sh", VERIFY_SH),
    ] {
        assert!(
            !src.contains("WS_STATUS_THEME_STORE"),
            "{label} still names WS_STATUS_THEME_STORE; the binary ignores it, \
             so it isolates nothing (use WS_STATUS_UI_STORE)"
        );
    }
}

#[test]
fn tty_spawn_paths_pin_theme_mode() {
    // PTY harness: the baseline user config pins themeMode, and a parent
    // WS_STATUS_THEME_MODE is dropped so it cannot beat the pin.
    assert!(
        PTY_HARNESS.contains(r#"\"themeMode\":\"dark\""#),
        "tui_tty_e2e/harness.rs BASELINE_USER_CONFIG must pin themeMode to dark \
         (auto follows the terminal background and flips colour assertions)"
    );
    assert!(
        PTY_HARNESS.contains("env_remove(\"WS_STATUS_THEME_MODE\")"),
        "tui_tty_e2e/harness.rs must drop a parent WS_STATUS_THEME_MODE"
    );
    // Desktop harness: same baseline file (xterm is white, auto would pick light).
    assert!(
        DESKTOP_HARNESS.contains("write_baseline_user_config(&config_home)"),
        "tui_tty_e2e/desktop.rs must write the harness baseline config (themeMode dark)"
    );
    assert!(
        DESKTOP_HARNESS.contains(r#"WS_STATUS_THEME WS_STATUS_THEME_MODE\n""#),
        "tui_tty_e2e/desktop.rs must unset a parent WS_STATUS_THEME_MODE"
    );
    // Demo clips: the launcher exports the knob, which defaults to dark.
    assert!(
        STILLS_SH.contains("STILLS_THEME_MODE=\"${WS_STATUS_STILLS_THEME_MODE:-dark}\"")
            && STILLS_SH.contains("export WS_STATUS_THEME_MODE="),
        "scripts/capture-demo-stills.sh must export WS_STATUS_THEME_MODE (default dark)"
    );
}

#[test]
fn tty_spawn_paths_drop_parent_pr_poll_period() {
    // A test that leaves WS_STATUS_PR_POLL_MS unset must not poll, so a
    // parent value (left from a manual try) is dropped; a test that wants
    // polling passes its own.
    assert!(
        PTY_HARNESS.contains("env_remove(\"WS_STATUS_PR_POLL_MS\")"),
        "tui_tty_e2e/harness.rs must drop a parent WS_STATUS_PR_POLL_MS"
    );
    assert!(
        DESKTOP_HARNESS.contains(r#"script.push_str("unset WS_STATUS_PR_POLL_MS\n");"#),
        "tui_tty_e2e/desktop.rs must unset a parent WS_STATUS_PR_POLL_MS"
    );
    assert!(
        VERIFY_SH.contains("export WS_STATUS_PR_POLL_MS=0"),
        "scripts/verify-tui.sh must pin WS_STATUS_PR_POLL_MS (off unless a recipe sets it)"
    );
}

#[test]
fn verify_driver_isolates_forge_theme_and_git() {
    // The verify driver (`scripts/verify-tui.sh`) shares the TTY spawn rules
    // above through the loops; these are the pins only it adds.
    for var in [
        "WS_STATUS_THEME",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GITLAB_TOKEN",
    ] {
        assert!(
            unsets_var(VERIFY_SH, var),
            "scripts/verify-tui.sh must `unset {var}` (a parent value would change the look, \
             or give a fake-forge run the operator's forge login)"
        );
    }
    for pin in [
        "export WS_STATUS_THEME_MODE=dark",
        "export GIT_CONFIG_GLOBAL=/dev/null",
        "export GIT_ALLOW_PROTOCOL=file",
    ] {
        assert!(
            VERIFY_SH.contains(pin),
            "scripts/verify-tui.sh must pin `{pin}` (dark theme twin; no operator git config; \
             no git network from a fixture)"
        );
    }
    // The binary comes from the checkout's `target/`. A bare name would let
    // `PATH` pick an installed release that is older than the code under test.
    assert!(
        VERIFY_SH.contains("${CARGO_TARGET_DIR:-$REPO_ROOT/target}")
            && VERIFY_SH.contains("debug/workspace-status")
            && VERIFY_SH.contains("printf 'exec %q\\n' \"$bin\""),
        "scripts/verify-tui.sh must exec the checkout's own target/debug/workspace-status"
    );
    for bare in [
        "command -v workspace-status",
        "which workspace-status",
        "type -p workspace-status",
        "exec workspace-status",
        "exec ws ",
    ] {
        assert!(
            !VERIFY_SH.contains(bare),
            "scripts/verify-tui.sh must not look up or exec the installed binary (`{bare}`)"
        );
    }
    // Both modes get the BROWSER recorder, so a live run never opens a real
    // browser: the export must come before the fake / live split.
    let browser = VERIFY_SH.find("export BROWSER=");
    let split = VERIFY_SH.find("if [[ -n \"$forge\" ]]");
    assert!(
        matches!((browser, split), (Some(b), Some(s)) if b < s)
            && VERIFY_SH.contains("browser/open %s")
            && VERIFY_SH.contains("opened.log")
            && !unsets_var(VERIFY_SH, "BROWSER"),
        "scripts/verify-tui.sh must export the BROWSER recorder for fake and live runs alike"
    );
    assert!(
        VERIFY_SH.contains("export WS_FAKE_FORGE_DIR=") && VERIFY_SH.contains("/fake-forge:$PATH"),
        "scripts/verify-tui.sh must put scripts/fake-forge first on PATH for --fake-forge"
    );
}

#[test]
fn verify_driver_wait_checks_the_pane_before_it_matches() {
    // `wait --gone` passes when the text is missing. A dead TUI or a missing
    // tmux server also shows no text, so `cmd_wait` must prove the pane is
    // alive before it looks at the screen.
    let body = VERIFY_SH
        .split("cmd_wait() {")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("scripts/verify-tui.sh defines cmd_wait");
    let session = body
        .find("has-session")
        .expect("cmd_wait checks the session");
    let dead = body.find("pane_dead").expect("cmd_wait checks pane_dead");
    let matched = body
        .find("screen_matches \"$kind\"")
        .expect("cmd_wait matches the screen");
    assert!(
        session < matched && dead < matched,
        "cmd_wait must check the tmux session and pane_dead before screen_matches, \
         or `wait --gone` passes on a dead TUI"
    );
}

#[test]
fn verify_driver_mouse_checks_the_pane_and_the_cell_first() {
    // `mouse` writes raw SGR reports into the pane. It must prove the TUI is
    // alive before it writes (a dead pane would drop them silently), and refuse
    // a cell that is not a positive integer pair, so a bad recipe fails loudly
    // instead of sending `ESC[<35;;M`.
    let body = VERIFY_SH
        .split("cmd_mouse() {")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("scripts/verify-tui.sh defines cmd_mouse");
    let live = body
        .find("require_live")
        .expect("cmd_mouse checks the pane with require_live");
    let checked = body
        .find("^[1-9][0-9]*$")
        .expect("cmd_mouse refuses a cell that is not a positive integer");
    let sent = body
        .find("send-keys")
        .expect("cmd_mouse writes the report with send-keys");
    assert!(
        live < sent && checked < sent,
        "cmd_mouse must run require_live and check the cell before it writes a report"
    );
    assert!(
        VERIFY_SH.contains("mouse) cmd_mouse \"$@\" ;;"),
        "scripts/verify-tui.sh must dispatch the mouse command"
    );
}

#[test]
fn verify_driver_reset_removes_only_directories() {
    // `scenario --reset <dir>` clears `<dir>/gh` and `<dir>/glab`. In
    // `scripts/fake-forge` those names are the fake executables, so the reset
    // must refuse a regular file before it removes anything, and `rm` only a
    // directory.
    let body = VERIFY_SH
        .split("cmd_scenario() {")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("scripts/verify-tui.sh defines cmd_scenario");
    let refuse = body
        .find("! -d \"$forge/$d\"")
        .expect("scenario --reset must refuse a gh / glab entry that is not a directory");
    let remove = body
        .find("rm -rf")
        .expect("scenario --reset removes the old sets");
    assert!(
        refuse < remove && body.contains("if [[ -d \"$forge/$d\" ]]; then rm -rf"),
        "scenario --reset must check for a file before it removes, and remove directories only"
    );
}
