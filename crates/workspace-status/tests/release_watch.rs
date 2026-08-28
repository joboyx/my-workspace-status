//! CI watch for cargo-dist generate and TTY update-check isolation.
//!
//! `dist generate` rewrites `.github/workflows/release.yml` and drops the
//! host-job git-cliff steps plus `workflow_dispatch`. A TTY stills or e2e
//! launch without `WS_STATUS_UPDATE_CHECK_STORE` writes
//! `$XDG_STATE_HOME/my-workspace-status/update-check.json` (the operator
//! last-check file) and can block mount on the GitHub Release prompt.
//! These tests fail until both hazards stay fixed. Recipe:
//! [docs/architecture.md](../../../docs/architecture.md).

const RELEASE_YML: &str = include_str!("../../../.github/workflows/release.yml");
const TAG_RELEASE_YML: &str = include_str!("../../../.github/workflows/tag-release.yml");
const DIST_WORKSPACE: &str = include_str!("../../../dist-workspace.toml");
const STILLS_SH: &str = include_str!("../../../scripts/capture-demo-stills.sh");
const PTY_HARNESS: &str = include_str!("tui_tty_e2e/harness.rs");
const DESKTOP_HARNESS: &str = include_str!("tui_tty_e2e/desktop.rs");

fn assigns_update_check_store(src: &str) -> bool {
    src.contains("env(\"WS_STATUS_UPDATE_CHECK_STORE\"")
        || src.contains("export WS_STATUS_UPDATE_CHECK_STORE=")
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
}

#[test]
fn dist_workspace_allows_dirty_ci() {
    assert!(
        DIST_WORKSPACE.contains("allow-dirty = [\"ci\"]"),
        "dist-workspace.toml must keep allow-dirty = [\"ci\"] so generate can leave hand-edits on release.yml"
    );
    assert!(
        !DIST_WORKSPACE.contains("dispatch-releases"),
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
}
