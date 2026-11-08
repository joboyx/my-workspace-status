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
//! loop below. The skill's controller
//! (`.agents/skills/verify-workspace-status/scripts/wsctl`) spawns only
//! through that driver, so the driver's guards cover it; it must never start
//! the binary or a tmux session itself (`wsctl_routes_spawns_through_the_driver`).
//! Recipe: [docs/architecture.md](../../../docs/architecture.md).

const RELEASE_YML: &str = include_str!("../../../.github/workflows/release.yml");
const TAG_RELEASE_YML: &str = include_str!("../../../.github/workflows/tag-release.yml");
const TAG_RELEASE_SH: &str = include_str!("../../../scripts/tag-release.sh");
const DIST_WORKSPACE: &str = include_str!("../../../dist-workspace.toml");
const STILLS_SH: &str = include_str!("../../../scripts/capture-demo-stills.sh");
const VERIFY_SH: &str = include_str!("../../../scripts/verify-tui.sh");
const WSCTL: &str = include_str!("../../../.agents/skills/verify-workspace-status/scripts/wsctl");
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

/// `src` without its comment lines (first non-blank character `#`), so no
/// guard passes or fails on what a comment says. Trailing comments stay.
fn strip_comment_lines(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `src` without the bodies of its here-documents (`<<'H'` ... `H`): help
/// text, not commands. The line that opens one stays.
fn strip_heredoc_bodies(src: &str) -> String {
    let mut out = Vec::new();
    let mut end: Option<String> = None;
    for line in src.lines() {
        if let Some(delim) = &end {
            if line.trim() == delim {
                end = None;
            }
            continue;
        }
        out.push(line);
        if let Some(at) = line.find("<<") {
            let rest = &line[at + 2..];
            if !rest.starts_with('<') {
                let delim: String = rest
                    .trim_start_matches('-')
                    .trim_start()
                    .trim_start_matches(['\'', '"'])
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !delim.is_empty() {
                    end = Some(delim);
                }
            }
        }
    }
    out.join("\n")
}

/// True when a line of `src` starts a program directly: `exec`, or the binary
/// by name. Give it text without comment lines.
fn runs_a_binary_directly(src: &str) -> bool {
    src.lines().any(|line| {
        let first = line.split_whitespace().next().unwrap_or("");
        matches!(first, "exec" | "workspace-status" | "ws")
    })
}

/// True when a line of `src` runs the run's binary: `$RUN_BIN` or
/// `${RUN_BIN}` (quoted or not) in command position — at the line start or
/// after `;`, `&&`, `||`, `|`, `$(`, `(`, `{`, a backquote, an `env` style
/// assignment, or one of `if then else elif do while until ! exec env command
/// time nohup eval xargs` — or anywhere on a `send-keys` line. A reference
/// inside a word (`x="$RUN_BIN"`) or after another word (`[[ -x "$RUN_BIN"`,
/// `json_str "$RUN_BIN"`) is data. Give it text without comment lines.
fn runs_run_bin(src: &str) -> bool {
    src.lines().any(|line| {
        let refs: Vec<usize> = ["$RUN_BIN", "${RUN_BIN}"]
            .iter()
            .flat_map(|form| line.match_indices(form).map(|(at, _)| at))
            .collect();
        if refs.is_empty() {
            return false;
        }
        if line.contains("send-keys") {
            return true;
        }
        refs.into_iter().any(|at| {
            let start = if line[..at].ends_with('"') {
                at - 1
            } else {
                at
            };
            let head = &line[..start];
            let in_word = head
                .chars()
                .last()
                .is_some_and(|prev| !(prev.is_whitespace() || "(;|&`!{".contains(prev)));
            !in_word && in_command_position(&head.split_whitespace().collect::<Vec<_>>())
        })
    })
}

/// Shell words after which the next word is a command.
const COMMAND_KEYWORDS: [&str; 15] = [
    "if", "then", "else", "elif", "do", "while", "until", "!", "exec", "env", "command", "time",
    "nohup", "eval", "xargs",
];

/// Word endings after which the next word is a command.
const COMMAND_OPENERS: [&str; 9] = [";", "&&", "||", "|", "$(", "(", "{", "`", "!"];

/// True when `word` is a `NAME=value` assignment.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// True when the word after `before` (the words before it on its line) is a
/// command: nothing before it but assignments, or an opener or a keyword
/// right before it.
fn in_command_position(before: &[&str]) -> bool {
    let mut words = before;
    while let [rest @ .., last] = words {
        if !is_assignment(last) {
            break;
        }
        words = rest;
    }
    match words.last() {
        None => true,
        Some(last) => {
            COMMAND_KEYWORDS.contains(last) || COMMAND_OPENERS.iter().any(|op| last.ends_with(op))
        }
    }
}

/// The tmux subcommands wsctl may run, each on a run's private server: check
/// the session, read pane fields and the screen, and resize the window. Keys
/// go through the driver (`drv keys`), so `send-keys` is not here: a wsctl
/// `send-keys` could type a command line that starts the binary. Nothing here
/// makes a session, a window or a pane (`new`, `new-session`, `new-window`,
/// `split-window`, `respawn-pane`).
const WSCTL_TMUX_SUBCOMMANDS: [&str; 4] = [
    "has-session",
    "display-message",
    "set-option",
    "resize-window",
];

/// The only `set-option` form wsctl may use: hooks are options in tmux 3.x,
/// so any other option could run a command.
const WSCTL_SET_OPTION: &str = "set-option -g window-size manual";

/// The tmux subcommand of every tmux call in `src` (no comment lines, no
/// here-document bodies). A call is `tm` (wsctl's helper) or `tmux` in
/// command position (see `in_command_position`; `"$(tmux` is one too); the
/// subcommand is the next word, or the word after `tmux -L <socket>`. `tmux
/// -V` has none. Other `tmux` words are prose (`command -v tmux`, `check tmux
/// 1`, `"... without tmux ..."`). Err names the line of a call that is not on
/// a private socket, chains subcommands (`\;`), or forwards `"$@"` outside the
/// `tm()` helper.
fn tmux_subcommands(src: &str) -> Result<Vec<String>, String> {
    let mut subs = Vec::new();
    for line in src.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let mut on_line = false;
        for (i, word) in words.iter().enumerate() {
            // `"$(tm x` and `(tmux` start a command; a prefix left on the word
            // (`"tmux`) makes it prose.
            let core = word
                .rsplit("$(")
                .next()
                .unwrap_or(word)
                .trim_start_matches(['(', '!', '`', '{', ';', '|', '&']);
            if !matches!(core, "tm" | "tmux") {
                continue;
            }
            if core.len() == word.len() && !in_command_position(&words[..i]) {
                continue;
            }
            let sub = match (core, words.get(i + 1).copied()) {
                ("tmux", Some("-V")) => continue,
                ("tmux", Some("-L")) => words.get(i + 3).copied(),
                ("tmux", _) => return Err(format!("tmux call without -L <socket>: {line}")),
                (_, next) => next,
            };
            let sub = sub
                .ok_or_else(|| format!("tmux call with no subcommand: {line}"))?
                .trim_matches(['"', '\'', ';']);
            if sub == "$@" {
                if !line.trim_start().starts_with("tm() {") {
                    return Err(format!("tmux forwards \"$@\" outside tm(): {line}"));
                }
                continue;
            }
            on_line = true;
            subs.push(sub.to_string());
        }
        if on_line && line.contains("\\;") {
            return Err(format!("tmux call chains subcommands: {line}"));
        }
    }
    Ok(subs)
}

/// Every way `src` (wsctl's text) breaks the rule that it spawns only through
/// the driver; empty when it keeps it. Comment lines are stripped first; the
/// `$RUN_BIN` and tmux checks also skip here-document bodies (help text).
fn wsctl_spawn_problems(src: &str) -> Vec<String> {
    let code = strip_comment_lines(src);
    let commands = strip_heredoc_bodies(&code);
    let mut problems = Vec::new();
    if !code.contains("DRIVER=\"$REPO_ROOT/scripts/verify-tui.sh\"") {
        problems.push("no DRIVER=\"$REPO_ROOT/scripts/verify-tui.sh\"".to_string());
    }
    if !code.contains("drv() { \"$DRIVER\" \"$@\"") {
        problems.push("no drv() { \"$DRIVER\" \"$@\" ... }".to_string());
    }
    if !code.contains("tm() { tmux -L \"$RUN_SOCK\" \"$@\"; }") {
        problems.push("no tm() { tmux -L \"$RUN_SOCK\" \"$@\"; }".to_string());
    }
    for verb in ["build", "launch", "keys", "wait", "capture", "cli", "stop"] {
        if !code.contains(&format!("drv {verb}")) && !code.contains(&format!("args=({verb})")) {
            problems.push(format!(
                "`{verb}` not routed through the driver (drv {verb})"
            ));
        }
    }
    for banned in [
        "new-session",
        "new -d",
        "cargo run",
        "cargo build",
        "target/debug",
        "debug/workspace-status",
    ] {
        if code.contains(banned) {
            problems.push(format!(
                "starts a TUI, a session or a build itself: `{banned}`"
            ));
        }
    }
    if runs_a_binary_directly(&code) {
        problems.push("execs workspace-status / ws directly".to_string());
    }
    // Here-document bodies are skipped below, so they may only be printed.
    for line in code.lines() {
        let opens = line
            .match_indices("<<")
            .any(|(at, _)| !line[at + 2..].starts_with('<') && !line[..at].ends_with('<'));
        if opens && line.split_whitespace().next() != Some("cat") {
            problems.push(format!(
                "here-document fed to something other than cat: {line}"
            ));
        }
    }
    if runs_run_bin(&commands) {
        problems.push("runs $RUN_BIN as a command".to_string());
    }
    match tmux_subcommands(&commands) {
        Err(err) => problems.push(err),
        Ok(subs) => {
            for sub in subs {
                if !WSCTL_TMUX_SUBCOMMANDS.contains(&sub.as_str()) {
                    problems.push(format!("tmux subcommand `{sub}` is not allowed"));
                }
            }
        }
    }
    for line in code.lines().filter(|line| line.contains("set-option")) {
        if !line.contains(WSCTL_SET_OPTION) {
            problems.push(format!("only `{WSCTL_SET_OPTION}` is allowed: {line}"));
        }
    }
    problems
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

/// dist 0.32 builds `aarch64-apple-darwin` on `macos-14`, an image GitHub no
/// longer runs. Without the pin the Release build of that target never starts,
/// so the host job never publishes. A `dist init` that rewrites the config
/// must keep it.
#[test]
fn dist_workspace_pins_a_supported_apple_silicon_runner() {
    let section = DIST_WORKSPACE
        .split("[dist.github-custom-runners]")
        .nth(1)
        .expect("dist-workspace.toml must keep [dist.github-custom-runners]");
    let runners: Vec<&str> = section
        .lines()
        .map(str::trim)
        .take_while(|line| !line.starts_with('['))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert!(
        runners.contains(&"aarch64-apple-darwin = \"macos-15\""),
        "aarch64-apple-darwin must build on macos-15, got {runners:?}"
    );
    assert!(
        !runners.iter().any(|line| line.contains("\"macos-14")),
        "macOS 14 images are retired: {runners:?}"
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
fn tag_release_runs_the_tested_script_before_dispatch() {
    // `tests/tag_release.rs` tests the script's behaviour. These pins keep the
    // workflow on that script and keep the parts a temp origin cannot prove
    // are needed on GitHub: remote tags count as taken, and the push is atomic.
    let run = TAG_RELEASE_YML
        .find("version=$(bash scripts/tag-release.sh)")
        .expect("tag-release.yml must take the version from scripts/tag-release.sh");
    let skip = TAG_RELEASE_YML
        .find("if [[ -z \"$version\" ]]")
        .expect("tag-release.yml must not dispatch when the script skips");
    let dispatch = TAG_RELEASE_YML
        .find("gh workflow run release.yml --ref \"v${version}\"")
        .expect("tag-release.yml must dispatch release.yml for the new tag");
    assert!(
        run < skip && skip < dispatch,
        "tag-release.yml must run the script, then skip on no version, then dispatch"
    );
    for pin in [
        "git fetch --tags origin",
        "git push --atomic origin HEAD:main \"v${version}\"",
    ] {
        assert!(
            TAG_RELEASE_SH.contains(pin),
            "scripts/tag-release.sh must keep `{pin}`"
        );
    }
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

/// The smallest text that keeps every wsctl spawn rule, for the guard's
/// self-tests.
const WSCTL_MINIMAL: &str = "DRIVER=\"$REPO_ROOT/scripts/verify-tui.sh\"
drv() { \"$DRIVER\" \"$@\" 2>\"$ERRF\"; }
tm() { tmux -L \"$RUN_SOCK\" \"$@\"; }
drv build; args=(launch); drv keys; drv wait; drv capture; drv cli; drv stop
command -v tmux >/dev/null 2>&1 || fail x
check tmux 1 \"$(tmux -V 2>/dev/null || echo tmux)\"
tm has-session -t \"$SESSION\" 2>/dev/null
pane_field() { tm display-message -p -t \"$SESSION\" \"#{$1}\" 2>/dev/null; }
tm set-option -g window-size manual >/dev/null 2>&1
tm resize-window -t \"$SESSION\" -x 1 -y 1
check binary 0 \"no binary at $RUN_BIN: wsctl stop\"
if [[ ! -x \"$RUN_BIN\" ]]; then :; fi
elif [[ \"$RUN_BIN\" -nt \"$RUN_DIR/launch.sh\" ]]; then :
want=\"$RUN_BIN\"; printf '%s' \"$(json_str \"$RUN_BIN\")\"
# exec \"$RUN_BIN\"; tmux new-session; tm send-keys \"$RUN_BIN\" Enter
";

#[test]
fn wsctl_spawn_guard_rejects_bad_snippets() {
    assert_eq!(
        wsctl_spawn_problems(WSCTL_MINIMAL),
        Vec::<String>::new(),
        "the minimal wsctl text keeps every rule (comment lines do not count)"
    );
    // A required line that is only a comment does not count.
    let commented = WSCTL_MINIMAL.replace("DRIVER=", "# DRIVER=");
    assert!(
        !wsctl_spawn_problems(&commented).is_empty(),
        "a commented-out DRIVER= must fail"
    );
    let commented = WSCTL_MINIMAL.replace("drv build;", "# drv build;");
    assert!(
        !wsctl_spawn_problems(&commented).is_empty(),
        "a commented-out drv build must fail"
    );
    for bad in [
        // The binary started directly.
        "exec \"$bin\"",
        "workspace-status --plain",
        "ws -i",
        "\"$RUN_BIN\" --json",
        "  $RUN_BIN --json",
        "x=1; \"$RUN_BIN\" --plain",
        "ok && $RUN_BIN --json",
        "false || \"${RUN_BIN}\"",
        "echo | \"$RUN_BIN\"",
        "v=\"$(\"$RUN_BIN\" --version)\"",
        "out=$($RUN_BIN --json)",
        "( \"$RUN_BIN\" )",
        "if \"$RUN_BIN\" --version; then :; fi",
        "then ${RUN_BIN} -i",
        "! \"$RUN_BIN\" --plain",
        "exec_it; exec \"$RUN_BIN\"",
        "env FOO=1 \"$RUN_BIN\"",
        "FOO=1 \"$RUN_BIN\"",
        "command \"$RUN_BIN\" --plain",
        // A session, window or pane of its own, or keys that type a command.
        "tm new-window -t ws",
        "tm split-window -t ws",
        "tm respawn-pane -k -t ws",
        "tmux -L \"$RUN_SOCK\" new -d",
        "tmux -L \"$RUN_SOCK\" new-session -d",
        "tm send-keys -t ws q",
        "tm send-keys -t ws \"$RUN_BIN\" Enter",
        "tmux -L s send-keys -t ws \"exec ${RUN_BIN}\" Enter",
        "tm has-session -t ws \\; new-window",
        // The default server, another socket, or a forward outside tm().
        "tmux new-session -d",
        "tmux ls",
        "tmux -S /tmp/sock ls",
        "tmux -f /dev/null new",
        "x() { tmux -L s \"$@\"; }",
        // A here-document run as a script hides its body from the checks.
        "bash <<'X'\n\"$RUN_BIN\"\nX",
        // A build of its own.
        "cargo build -p workspace-status",
        "bin=\"$REPO_ROOT/target/debug/workspace-status\"",
    ] {
        let src = format!("{WSCTL_MINIMAL}{bad}\n");
        assert!(
            !wsctl_spawn_problems(&src).is_empty(),
            "the wsctl spawn guard must reject `{bad}`"
        );
    }
}

#[test]
fn wsctl_routes_spawns_through_the_driver() {
    // wsctl is the agent-facing layer of the verify skill. Every spawn must go
    // through scripts/verify-tui.sh, so the isolation pinned above for the
    // driver covers wsctl too: no tmux session, window, pane, keys, build or
    // binary run of its own. It may only check, read and resize the run's
    // private tmux server (WSCTL_TMUX_SUBCOMMANDS).
    assert_eq!(
        wsctl_spawn_problems(WSCTL),
        Vec::<String>::new(),
        "wsctl must spawn only through scripts/verify-tui.sh"
    );
}
