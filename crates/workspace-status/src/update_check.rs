//! TUI-startup GitHub Release check. Prompt only; never a silent install.
//!
//! Runs only when the process is about to open the ratatui TUI on a TTY.
//! `--plain` / `--json` / `--update` never call this. A failed or current
//! check stays quiet. At most one network fetch per
//! [`CHECK_INTERVAL`]. The fetch is `curl` GET of GitHub Releases `latest`
//! (4s timeout; missing `curl` is a quiet failure). `--update` reuses
//! the same `curl` helper for the Releases list; it does not call this prompt.
//! A dev build ([`crate::DEV_BUILD`]) never runs this check: it must not
//! offer to replace itself with a published release. `WS_STATUS_UPDATE_CHECK=0`
//! turns the check off.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// How long a last-check timestamp stays fresh.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// End of the prompt printed on the primary screen before the TUI mounts.
///
/// The full line is [`update_prompt`]. A blank answer means no.
pub const UPDATE_PROMPT_SUFFIX: &str = "available. Update? [y/N] ";

/// Env var that turns the startup check off when set to `0`, `false`, or `off`.
pub const UPDATE_CHECK_ENV: &str = "WS_STATUS_UPDATE_CHECK";

const STORE_VERSION: u32 = 1;
const STORE_FILE_NAME: &str = "update-check.json";
const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/joboyx/my-workspace-status/releases/latest";

/// GET a GitHub API URL with `curl`. `max_time_secs` is `--max-time`.
///
/// `WORKSPACE_STATUS_GITHUB_TOKEN` adds a Bearer header when set. Missing
/// `curl`, a non-zero exit, or non-UTF-8 stdout is an error.
pub(crate) fn github_get(url: &str, max_time_secs: &str) -> Result<String, String> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsS",
        "--max-time",
        max_time_secs,
        "-H",
        "User-Agent: workspace-status",
        "-H",
        "Accept: application/vnd.github+json",
    ]);
    if let Ok(token) = env::var("WORKSPACE_STATUS_GITHUB_TOKEN") {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            cmd.arg("-H")
                .arg(format!("Authorization: Bearer {trimmed}"));
        }
    }
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = cmd.output().map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err("curl failed".into());
    }
    String::from_utf8(output.stdout).map_err(|err| err.to_string())
}

/// Outcome of the TUI-startup offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupUpdateOffer {
    /// Open the TUI (not due, current, failed, declined, or no TTY).
    Continue,
    /// User accepted. Caller should run the cargo-dist sidecar.
    RunUpdater,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreFile {
    version: u32,
    last_check_unix: u64,
}

/// Hooks for [`offer_startup_update_with`]. Tests inject fetch and prompt.
pub(crate) struct UpdateCheckHooks<F, P> {
    pub stdin_is_tty: bool,
    pub stdout_is_tty: bool,
    pub now: SystemTime,
    pub current_version: &'static str,
    pub store_path: PathBuf,
    pub fetch_latest: F,
    /// Ask the user. Gets the full prompt line; returns the answer.
    pub prompt_yes: P,
}

/// Startup check on or off from [`UPDATE_CHECK_ENV`] and the config
/// `updateCheck` value.
///
/// Env `0`, `false`, or `off` turn it off; `1`, `true`, or `on` turn it on
/// (trimmed, any case). Any other env value, or no env var, falls through
/// to `config`, then to on.
pub(crate) fn update_check_enabled(env_value: Option<&str>, config: Option<bool>) -> bool {
    let from_env = env_value.and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" => Some(false),
        "1" | "true" | "on" => Some(true),
        _ => None,
    });
    from_env.or(config).unwrap_or(true)
}

/// True when this build and the resolved setting may run the TUI-startup check.
///
/// `dev_build` is [`crate::DEV_BUILD`]; a dev build returns false.
/// `enabled` is [`crate::settings::Settings`] `update_check`
/// ([`update_check_enabled`]).
pub(crate) fn startup_check_enabled(dev_build: Option<&str>, enabled: bool) -> bool {
    dev_build.is_none() && enabled
}

/// Startup prompt: `workspace-status 0.1.19 → 0.1.20 available. Update? [y/N] `.
pub fn update_prompt(current: &str, latest_tag: &str) -> String {
    let latest = latest_tag.trim().trim_start_matches(['v', 'V']);
    format!("workspace-status {current} → {latest} {UPDATE_PROMPT_SUFFIX}")
}

/// TUI-startup check using the real clock, store, GitHub fetch, and stdin.
///
/// A dev build, or a resolved `update_check` of false ([`UPDATE_CHECK_ENV`]
/// `0`, or config `"updateCheck": false`), returns
/// [`StartupUpdateOffer::Continue`] without a fetch or a store write. The
/// store path is the resolved `update_check_store`.
pub fn offer_startup_update(settings: &Settings) -> StartupUpdateOffer {
    if !startup_check_enabled(crate::DEV_BUILD, settings.update_check) {
        return StartupUpdateOffer::Continue;
    }
    offer_startup_update_with(UpdateCheckHooks {
        stdin_is_tty: io::stdin().is_terminal(),
        stdout_is_tty: io::stdout().is_terminal(),
        now: SystemTime::now(),
        current_version: crate::APP_VERSION,
        store_path: settings.update_check_store.clone(),
        fetch_latest: fetch_latest_release_tag,
        prompt_yes: prompt_yes_no,
    })
}

/// Same decision as [`offer_startup_update`], with injected I/O.
pub(crate) fn offer_startup_update_with<F, P>(hooks: UpdateCheckHooks<F, P>) -> StartupUpdateOffer
where
    F: FnOnce() -> Result<String, String>,
    P: FnOnce(&str) -> bool,
{
    if !hooks.stdin_is_tty || !hooks.stdout_is_tty {
        return StartupUpdateOffer::Continue;
    }
    let last = load_last_check(&hooks.store_path);
    if !check_is_due(last, hooks.now) {
        return StartupUpdateOffer::Continue;
    }
    let latest = (hooks.fetch_latest)();
    save_last_check(&hooks.store_path, hooks.now);
    let Ok(latest) = latest else {
        return StartupUpdateOffer::Continue;
    };
    if !is_newer_release(hooks.current_version, &latest) {
        return StartupUpdateOffer::Continue;
    }
    if (hooks.prompt_yes)(&update_prompt(hooks.current_version, &latest)) {
        StartupUpdateOffer::RunUpdater
    } else {
        StartupUpdateOffer::Continue
    }
}

/// True when there is no last-check timestamp, or it is older than 6 hours.
pub fn check_is_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    match last {
        None => true,
        Some(checked) => now
            .duration_since(checked)
            .map(|elapsed| elapsed >= CHECK_INTERVAL)
            .unwrap_or(true),
    }
}

/// True when `latest_tag` is a strictly newer `X.Y.Z` than `current`.
///
/// Leading `v` is ignored. Unparseable values are not newer.
pub fn is_newer_release(current: &str, latest_tag: &str) -> bool {
    match (parse_semver_core(current), parse_semver_core(latest_tag)) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    }
}

/// `tag_name` from a GitHub Releases JSON body.
pub fn parse_release_tag_name(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let tag = value.get("tag_name")?.as_str()?.trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// `y` / `yes` → true, `n` / `no` / blank → false, anything else → keep asking.
pub fn interpret_yes_no(line: &str) -> Option<bool> {
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" | "" => Some(false),
        _ => None,
    }
}

/// Resolve the store path from an env lookup and the config `updateCheckStore` value.
///
/// A non-blank `WS_STATUS_UPDATE_CHECK_STORE` wins, then `config`, then
/// `$XDG_STATE_HOME/my-workspace-status/...` (else `$HOME/.local/state/...`).
pub fn update_check_store_path_from_env<F>(mut get: F, config: Option<&Path>) -> PathBuf
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(override_path) = get("WS_STATUS_UPDATE_CHECK_STORE") {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(path) = config {
        return path.to_path_buf();
    }
    let state_home = get("XDG_STATE_HOME")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = get("HOME")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            home.join(".local").join("state")
        });
    state_home.join("my-workspace-status").join(STORE_FILE_NAME)
}

/// Last successful or failed check time. Unknown versions load as missing.
pub fn load_last_check(path: &Path) -> Option<SystemTime> {
    let text = fs::read_to_string(path).ok()?;
    let parsed: StoreFile = serde_json::from_str(&text).ok()?;
    if parsed.version != STORE_VERSION {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(parsed.last_check_unix))
}

/// Persist `now` as the last-check time (success or failure).
pub fn save_last_check(path: &Path, now: SystemTime) {
    let unix = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = StoreFile {
        version: STORE_VERSION,
        last_check_unix: unix,
    };
    let Ok(mut body) = serde_json::to_string_pretty(&file) else {
        return;
    };
    body.push('\n');
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(mut f) = fs::File::create(&tmp) {
        if f.write_all(body.as_bytes()).is_ok() && f.flush().is_ok() {
            let _ = fs::rename(&tmp, path);
            return;
        }
    }
    let _ = fs::write(path, body);
    let _ = fs::remove_file(&tmp);
}

fn parse_semver_core(raw: &str) -> Option<(u64, u64, u64)> {
    let s = raw.trim().trim_start_matches(['v', 'V']);
    let mut nums = s.split(|c: char| !c.is_ascii_digit());
    let major = parse_next_num(&mut nums)?;
    let minor = parse_next_num(&mut nums)?;
    let patch = parse_next_num(&mut nums)?;
    Some((major, minor, patch))
}

fn parse_next_num<'a, I>(parts: &mut I) -> Option<u64>
where
    I: Iterator<Item = &'a str>,
{
    for part in parts {
        if part.is_empty() {
            continue;
        }
        return part.parse().ok();
    }
    None
}

fn fetch_latest_release_tag() -> Result<String, String> {
    let body = github_get(LATEST_RELEASE_URL, "4")?;
    parse_release_tag_name(&body).ok_or_else(|| "missing tag_name".into())
}

fn prompt_yes_no(prompt: &str) -> bool {
    let mut stdout = io::stdout();
    loop {
        if write!(stdout, "{prompt}").is_err() || stdout.flush().is_err() {
            return false;
        }
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => return false,
            Ok(_) => match interpret_yes_no(&line) {
                Some(answer) => return answer,
                None => continue,
            },
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> PathBuf {
        env::temp_dir().join(format!(
            "ws-update-check-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn hooks(
        store: PathBuf,
        last_age: Option<Duration>,
        fetch: fn() -> Result<String, String>,
        yes: bool,
    ) -> UpdateCheckHooks<impl FnOnce() -> Result<String, String>, impl FnOnce(&str) -> bool> {
        let now = SystemTime::now();
        if let Some(age) = last_age {
            save_last_check(&store, now - age);
        }
        UpdateCheckHooks {
            stdin_is_tty: true,
            stdout_is_tty: true,
            now,
            current_version: "0.1.19",
            store_path: store,
            fetch_latest: fetch,
            prompt_yes: move |_: &str| yes,
        }
    }

    fn fetch_newer() -> Result<String, String> {
        Ok("v0.1.20".into())
    }

    fn fetch_current() -> Result<String, String> {
        Ok("v0.1.19".into())
    }

    fn fetch_fail() -> Result<String, String> {
        Err("offline".into())
    }

    #[test]
    fn due_when_missing_or_stale() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert!(check_is_due(None, now));
        assert!(!check_is_due(Some(now - Duration::from_secs(60)), now));
        assert!(!check_is_due(
            Some(now - CHECK_INTERVAL + Duration::from_secs(1)),
            now
        ));
        assert!(check_is_due(Some(now - CHECK_INTERVAL), now));
        assert!(check_is_due(
            Some(now - CHECK_INTERVAL - Duration::from_secs(1)),
            now
        ));
    }

    #[test]
    fn newer_release_compares_semver_core() {
        assert!(is_newer_release("0.1.19", "v0.1.20"));
        assert!(is_newer_release("0.1.19", "0.2.0"));
        assert!(is_newer_release("0.1.19", "1.0.0"));
        assert!(!is_newer_release("0.1.19", "v0.1.19"));
        assert!(!is_newer_release("0.1.19", "0.1.18"));
        assert!(!is_newer_release("0.1.19", "not-a-version"));
        assert!(!is_newer_release("0.1.19", ""));
        assert!(is_newer_release("0.1.19", "v0.1.20-1"));
    }

    #[test]
    fn parse_tag_name_from_github_json() {
        assert_eq!(
            parse_release_tag_name(r#"{"tag_name":"v0.1.20","prerelease":false}"#).as_deref(),
            Some("v0.1.20")
        );
        assert_eq!(parse_release_tag_name(r#"{"name":"nope"}"#), None);
        assert_eq!(parse_release_tag_name("not json"), None);
        assert_eq!(parse_release_tag_name(r#"{"tag_name":"  "}"#), None);
    }

    #[test]
    fn interpret_yes_no_accepts_y_n() {
        assert_eq!(interpret_yes_no("y"), Some(true));
        assert_eq!(interpret_yes_no(" Yes\n"), Some(true));
        assert_eq!(interpret_yes_no("N"), Some(false));
        assert_eq!(interpret_yes_no("no"), Some(false));
        assert_eq!(interpret_yes_no(""), Some(false));
        assert_eq!(interpret_yes_no("\n"), Some(false));
        assert_eq!(interpret_yes_no("maybe"), None);
    }

    #[test]
    fn store_round_trip() {
        let path = temp_store();
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        save_last_check(&path, now);
        assert_eq!(load_last_check(&path), Some(now));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn store_path_env_beats_config_then_xdg() {
        let config = Path::new("/cfg/update-check.json");
        let env_and_xdg = |k: &str| match k {
            "WS_STATUS_UPDATE_CHECK_STORE" => Some("/env/update-check.json".into()),
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            _ => None,
        };
        assert_eq!(
            update_check_store_path_from_env(env_and_xdg, Some(config)),
            PathBuf::from("/env/update-check.json"),
            "env wins"
        );
        for blank in ["", "   "] {
            let blank_env = |k: &str| match k {
                "WS_STATUS_UPDATE_CHECK_STORE" => Some(blank.to_string()),
                "XDG_STATE_HOME" => Some("/xdg/state".into()),
                _ => None,
            };
            assert_eq!(
                update_check_store_path_from_env(blank_env, Some(config)),
                config,
                "blank env {blank:?} falls through to config"
            );
        }
        let xdg_only = |k: &str| match k {
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            _ => None,
        };
        assert_eq!(
            update_check_store_path_from_env(xdg_only, Some(config)),
            config
        );
    }

    #[test]
    fn store_path_override_and_xdg() {
        assert_eq!(
            update_check_store_path_from_env(
                |key| match key {
                    "WS_STATUS_UPDATE_CHECK_STORE" => Some("/tmp/custom-check.json".into()),
                    _ => None,
                },
                None
            ),
            PathBuf::from("/tmp/custom-check.json")
        );
        assert_eq!(
            update_check_store_path_from_env(
                |key| match key {
                    "XDG_STATE_HOME" => Some("/tmp/xdg-state".into()),
                    _ => None,
                },
                None
            ),
            PathBuf::from("/tmp/xdg-state/my-workspace-status/update-check.json")
        );
    }

    #[test]
    fn unknown_store_version_is_missing() {
        let path = temp_store();
        fs::write(&path, "{\"version\":99,\"lastCheckUnix\":1}\n").unwrap();
        assert_eq!(load_last_check(&path), None);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn no_tty_skips_without_fetch_or_store() {
        let path = temp_store();
        let called = std::cell::Cell::new(false);
        let offer = offer_startup_update_with(UpdateCheckHooks {
            stdin_is_tty: false,
            stdout_is_tty: true,
            now: SystemTime::now(),
            current_version: "0.1.19",
            store_path: path.clone(),
            fetch_latest: || {
                called.set(true);
                fetch_newer()
            },
            prompt_yes: |_: &str| true,
        });
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert!(!called.get());
        assert!(!path.exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn stdout_not_tty_skips_without_fetch_or_store() {
        let path = temp_store();
        let called = std::cell::Cell::new(false);
        let offer = offer_startup_update_with(UpdateCheckHooks {
            stdin_is_tty: true,
            stdout_is_tty: false,
            now: SystemTime::now(),
            current_version: "0.1.19",
            store_path: path.clone(),
            fetch_latest: || {
                called.set(true);
                fetch_newer()
            },
            prompt_yes: |_: &str| true,
        });
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert!(!called.get());
        assert!(!path.exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn recent_check_skips_without_fetch() {
        let path = temp_store();
        let now = SystemTime::now();
        save_last_check(&path, now - Duration::from_secs(60));
        let called = std::cell::Cell::new(false);
        let offer = offer_startup_update_with(UpdateCheckHooks {
            stdin_is_tty: true,
            stdout_is_tty: true,
            now,
            current_version: "0.1.19",
            store_path: path.clone(),
            fetch_latest: || {
                called.set(true);
                fetch_newer()
            },
            prompt_yes: |_: &str| true,
        });
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert!(!called.get());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn failed_fetch_stays_quiet_and_persists() {
        let path = temp_store();
        let offer = offer_startup_update_with(hooks(path.clone(), None, fetch_fail, true));
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert!(load_last_check(&path).is_some());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn current_release_stays_quiet_and_persists() {
        let path = temp_store();
        let offer = offer_startup_update_with(hooks(path.clone(), None, fetch_current, true));
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert!(load_last_check(&path).is_some());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn newer_yes_runs_updater() {
        let path = temp_store();
        let offer = offer_startup_update_with(hooks(path.clone(), None, fetch_newer, true));
        assert_eq!(offer, StartupUpdateOffer::RunUpdater);
        assert!(load_last_check(&path).is_some());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn newer_no_opens_tui() {
        let path = temp_store();
        let offer = offer_startup_update_with(hooks(path.clone(), None, fetch_newer, false));
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert!(load_last_check(&path).is_some());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn stale_check_fetches_again() {
        let path = temp_store();
        let offer = offer_startup_update_with(hooks(
            path.clone(),
            Some(CHECK_INTERVAL + Duration::from_secs(5)),
            fetch_newer,
            true,
        ));
        assert_eq!(offer, StartupUpdateOffer::RunUpdater);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn dev_build_skips_startup_check() {
        assert!(startup_check_enabled(None, true));
        assert!(!startup_check_enabled(None, false));
        assert!(!startup_check_enabled(Some("abc1234"), true));
        assert!(!startup_check_enabled(Some("abc1234-dirty"), true));
    }

    #[test]
    fn env_zero_turns_the_startup_check_off() {
        for off in ["0", "false", "OFF", " 0\n"] {
            assert!(!update_check_enabled(Some(off), None), "{off:?}");
            assert!(!update_check_enabled(Some(off), Some(true)), "{off:?}");
        }
        for on in ["1", "", "yes"] {
            assert!(update_check_enabled(Some(on), None), "{on:?}");
        }
        assert!(update_check_enabled(None, None));
    }

    #[test]
    fn config_update_check_false_turns_the_startup_check_off() {
        assert!(
            !update_check_enabled(None, Some(false)),
            "config when env unset"
        );
        assert!(update_check_enabled(None, Some(true)));
        for on in ["1", "true", " On "] {
            assert!(
                update_check_enabled(Some(on), Some(false)),
                "env {on:?} wins"
            );
        }
        for unknown in ["", "yes", "maybe"] {
            assert!(
                !update_check_enabled(Some(unknown), Some(false)),
                "env {unknown:?} falls through to config"
            );
        }
        let settings = crate::settings::Settings::resolve(
            &crate::config::RuntimeKeys {
                update_check: Some(false),
                update_check_store: Some(temp_store()),
                ..Default::default()
            },
            |_| None,
        );
        assert_eq!(
            offer_startup_update(&settings),
            StartupUpdateOffer::Continue
        );
        assert!(
            !settings.update_check_store.exists(),
            "updateCheck false must not write the store"
        );
    }

    #[test]
    fn prompt_names_both_versions_and_defaults_to_no() {
        assert_eq!(
            update_prompt("0.1.19", "v0.1.20"),
            "workspace-status 0.1.19 → 0.1.20 available. Update? [y/N] "
        );
        assert!(update_prompt("0.1.19", "0.1.20").ends_with(UPDATE_PROMPT_SUFFIX));
    }

    #[test]
    fn prompt_hook_gets_the_versioned_line() {
        let path = temp_store();
        let seen = std::cell::RefCell::new(String::new());
        let offer = offer_startup_update_with(UpdateCheckHooks {
            stdin_is_tty: true,
            stdout_is_tty: true,
            now: SystemTime::now(),
            current_version: "0.1.19",
            store_path: path.clone(),
            fetch_latest: fetch_newer,
            prompt_yes: |prompt: &str| {
                *seen.borrow_mut() = prompt.to_string();
                false
            },
        });
        assert_eq!(offer, StartupUpdateOffer::Continue);
        assert_eq!(
            seen.borrow().as_str(),
            "workspace-status 0.1.19 → 0.1.20 available. Update? [y/N] "
        );
        let _ = fs::remove_file(&path);
    }
}
