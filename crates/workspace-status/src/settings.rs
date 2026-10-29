//! Env-backed settings, resolved once at startup.
//!
//! Each setting has an env var and a config key ([`RuntimeKeys`]).
//! Precedence: CLI flag > env var > config file > built-in default. An env
//! value that the env parser accepts wins. An unset, blank, unknown, or
//! non-numeric env value falls through to the config value, then the
//! default. `cli.rs` resolves [`Settings`] after it loads the config and
//! before the update check and the first git call, then passes it on.
//!
//! The launch theme, the activity sparkline, and the commit-message footer
//! rows also read the UI-state file
//! ([`crate::tui::ui_state_store`]); its last-used theme sits between the
//! env var and the config value. Resolve only reads that file; it never
//! creates or writes it.

use std::env;
use std::path::PathBuf;

use crate::config::{GlyphSet, RuntimeKeys, ThemeMode};
use crate::git::resolve_git_binary;
use crate::parallel::fetch_concurrency;
use crate::tui::comments::comment_store_path_from_env;
use crate::tui::fetch::fetch_interval_ms;
use crate::tui::fetch_claim::background_fetch_store_path_from_env;
use crate::tui::pr_poll::{pr_poll_mode, PrPollMode};
use crate::tui::theme::{theme_id_from, ThemeId};
use crate::tui::ui_state_store::{load_ui_state, ui_store_path_from_env};
use crate::tui::undercurl::undercurl_enabled;
use crate::tui::viewed::viewed_store_path_from_env;
use crate::tui::watch::watch_interval_ms;
use crate::update_check::{
    update_check_enabled, update_check_store_path_from_env, UPDATE_CHECK_ENV,
};

/// The resolved env-backed settings for one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Launch theme: `WS_STATUS_THEME`, then the last-used theme in
    /// [`Self::ui_store`], then config `theme`, then Slate.
    pub(crate) theme: ThemeId,
    /// Light or dark pick at startup (`WS_STATUS_THEME_MODE` / `themeMode`),
    /// default [`ThemeMode::Auto`]. See [`crate::tui::term_bg::pick_theme`].
    pub(crate) theme_mode: ThemeMode,
    /// ASCII glyphs instead of Nerd Font glyphs (`WS_STATUS_GLYPHS` / `glyphs`).
    pub(crate) ascii: bool,
    /// Live-refresh poll period in ms, `0` off (`WS_STATUS_WATCH_MS` / `watchMs`).
    pub(crate) watch_ms: u64,
    /// Background fetch period in ms, `0` off (`WS_STATUS_FETCH_MS` / `fetchMs`).
    pub(crate) fetch_ms: u64,
    /// In-flight cap for per-repo git work
    /// (`WS_STATUS_FETCH_CONCURRENCY` / `fetchConcurrency`).
    pub(crate) fetch_concurrency: usize,
    /// TUI-startup release check on (`WS_STATUS_UPDATE_CHECK` / `updateCheck`).
    pub(crate) update_check: bool,
    /// Last-check JSON file (`WS_STATUS_UPDATE_CHECK_STORE` / `updateCheckStore`).
    pub(crate) update_check_store: PathBuf,
    /// Comment JSON file (`WS_STATUS_COMMENT_STORE` / `commentStore`).
    pub(crate) comment_store: PathBuf,
    /// Viewed-marks JSON file (`WS_STATUS_VIEWED_STORE` / `viewedStore`).
    pub(crate) viewed_store: PathBuf,
    /// UI-state JSON file: last-used theme and activity sparkline
    /// (`WS_STATUS_UI_STORE` / `uiStore`).
    pub(crate) ui_store: PathBuf,
    /// Paint the tree commit-activity sparkline: the [`Self::ui_store`]
    /// value, default off. No env var or config key.
    pub(crate) activity_sparkline: bool,
    /// Expanded commit-message footer rows a drag of its edge saved in
    /// [`Self::ui_store`]: `None` when none was saved or the value is bad.
    /// It beats config `viewDefaults.commitMessageLines`
    /// ([`crate::tui::state::AppState::apply_view_defaults`]). No env var.
    pub(crate) commit_msg_lines: Option<usize>,
    /// Shared background-fetch claim and outcome JSON file
    /// ([`crate::tui::fetch_claim`]): `$XDG_STATE_HOME/my-workspace-status/
    /// background-fetch.json`. No env var or config key.
    pub(crate) fetch_store: PathBuf,
    /// Git binary (`WORKSPACE_STATUS_GIT` / `git`).
    pub(crate) git: PathBuf,
    /// `NO_COLOR` is set and non-empty (no-color.org). Env only, no config
    /// key. The new visuals paint without colour; the rest keeps the theme.
    pub(crate) no_color: bool,
    /// Curly coloured underline on commented lines: `COLORTERM` is
    /// `truecolor` / `24bit` and neither `NO_COLOR` nor ASCII glyphs is on
    /// ([`crate::tui::undercurl::undercurl_enabled`]). Env only, no config key.
    pub(crate) undercurl: bool,
    /// OSC 9;4 terminal progress for multi-repo batches
    /// (`WS_STATUS_TERM_PROGRESS` / `termProgress`), default on.
    pub(crate) term_progress: bool,
    /// Ctrl-d / Ctrl-u glide the viewport over a few frames
    /// (`WS_STATUS_SMOOTH_SCROLL` / `smoothScroll`), default on. Off jumps.
    pub(crate) smooth_scroll: bool,
    /// PR polling (`WS_STATUS_PR_POLL_MS` / `prPolling`), default hot / calm
    /// ([`crate::tui::pr_poll::pr_poll_mode`]).
    pub(crate) pr_poll: PrPollMode,
    /// Program that names a ticket key for the PRs tab group titles
    /// (`WS_STATUS_PR_TITLE_CMD` / `prTitleCommand`, user file only). A
    /// non-blank env value wins. `None` runs nothing.
    // Only tests read this until the PRs tab does.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) pr_title_command: Option<PathBuf>,
}

impl Settings {
    /// Resolve every setting from the process environment and `keys` (the
    /// merged config files).
    pub fn from_env(keys: &RuntimeKeys) -> Settings {
        Settings::resolve(keys, |key| env::var(key).ok())
    }

    /// Resolve every setting from an env lookup and `keys`.
    pub fn resolve<F>(keys: &RuntimeKeys, mut get: F) -> Settings
    where
        F: FnMut(&str) -> Option<String>,
    {
        let ui_store = ui_store_path_from_env(&mut get, keys.ui_store.as_deref());
        let ui_state = load_ui_state(&ui_store);
        let ascii = glyphs_ascii(get("WS_STATUS_GLYPHS").as_deref(), keys.glyphs);
        let no_color = no_color(get(NO_COLOR_ENV).as_deref());
        Settings {
            theme: theme_id_from(
                get("WS_STATUS_THEME").as_deref(),
                ui_state.theme,
                keys.theme,
            ),
            theme_mode: theme_mode(get(THEME_MODE_ENV).as_deref(), keys.theme_mode),
            ascii,
            watch_ms: watch_interval_ms(get("WS_STATUS_WATCH_MS").as_deref(), keys.watch_ms),
            fetch_ms: fetch_interval_ms(get("WS_STATUS_FETCH_MS").as_deref(), keys.fetch_ms),
            fetch_concurrency: fetch_concurrency(
                get("WS_STATUS_FETCH_CONCURRENCY").as_deref(),
                keys.fetch_concurrency,
            ),
            update_check: update_check_enabled(get(UPDATE_CHECK_ENV).as_deref(), keys.update_check),
            update_check_store: update_check_store_path_from_env(
                &mut get,
                keys.update_check_store.as_deref(),
            ),
            comment_store: comment_store_path_from_env(&mut get, keys.comment_store.as_deref()),
            viewed_store: viewed_store_path_from_env(&mut get, keys.viewed_store.as_deref()),
            ui_store,
            activity_sparkline: ui_state.activity_sparkline,
            commit_msg_lines: ui_state.commit_msg_lines,
            fetch_store: background_fetch_store_path_from_env(&mut get),
            git: resolve_git_binary(get("WORKSPACE_STATUS_GIT").as_deref(), keys.git.as_deref()),
            no_color,
            undercurl: undercurl_enabled(no_color, ascii, get(COLORTERM_ENV).as_deref()),
            term_progress: env_bool(get(TERM_PROGRESS_ENV).as_deref())
                .or(keys.term_progress)
                .unwrap_or(true),
            smooth_scroll: env_bool(get(SMOOTH_SCROLL_ENV).as_deref())
                .or(keys.smooth_scroll)
                .unwrap_or(true),
            pr_poll: pr_poll_mode(get(PR_POLL_ENV).as_deref(), keys.pr_polling),
            pr_title_command: get(PR_TITLE_CMD_ENV)
                .map(|raw| raw.trim().to_string())
                .filter(|raw| !raw.is_empty())
                .map(PathBuf::from)
                .or_else(|| keys.pr_title_command.clone()),
        }
    }
}

/// Env var that overrides the PR poll period (`0` turns polling off).
const PR_POLL_ENV: &str = "WS_STATUS_PR_POLL_MS";

/// Env var for the `prTitleCommand` key.
const PR_TITLE_CMD_ENV: &str = "WS_STATUS_PR_TITLE_CMD";

/// Boolean env value: `0`, `false`, or `off` is false; `1`, `true`, or `on`
/// is true (trimmed, any case). Unset or any other value is `None`, so the
/// caller falls through to the config value, then the default.
pub(crate) fn env_bool(raw: Option<&str>) -> Option<bool> {
    match raw?.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" => Some(false),
        "1" | "true" | "on" => Some(true),
        _ => None,
    }
}

/// Env var for the `termProgress` key.
const TERM_PROGRESS_ENV: &str = "WS_STATUS_TERM_PROGRESS";

/// Env var for the `smoothScroll` key.
const SMOOTH_SCROLL_ENV: &str = "WS_STATUS_SMOOTH_SCROLL";

/// Terminal colour depth the emulator advertises; `truecolor` or `24bit`
/// turns on the curly underline.
const COLORTERM_ENV: &str = "COLORTERM";

/// The standard no-colour env var (<https://no-color.org>).
const NO_COLOR_ENV: &str = "NO_COLOR";

/// `NO_COLOR` is on when it is set to any non-empty value.
fn no_color(env_value: Option<&str>) -> bool {
    env_value.is_some_and(|value| !value.is_empty())
}

/// Env var for the `themeMode` key.
const THEME_MODE_ENV: &str = "WS_STATUS_THEME_MODE";

/// Theme mode from `WS_STATUS_THEME_MODE` (`env_value`) and the config
/// `themeMode` value. Env `auto`, `dark`, or `light` (exact) wins; any other
/// env value falls through to `config`, then [`ThemeMode::Auto`].
fn theme_mode(env_value: Option<&str>, config: Option<ThemeMode>) -> ThemeMode {
    let from_env = match env_value {
        Some("auto") => Some(ThemeMode::Auto),
        Some("dark") => Some(ThemeMode::Dark),
        Some("light") => Some(ThemeMode::Light),
        _ => None,
    };
    from_env.or(config).unwrap_or(ThemeMode::Auto)
}

/// ASCII glyphs from `WS_STATUS_GLYPHS` (`env_value`) and the config `glyphs`
/// value. Env `ascii` or `nerd` (exact) wins; any other env value falls
/// through to `config`, then Nerd Font glyphs.
fn glyphs_ascii(env_value: Option<&str>, config: Option<GlyphSet>) -> bool {
    let from_env = match env_value {
        Some("ascii") => Some(GlyphSet::Ascii),
        Some("nerd") => Some(GlyphSet::Nerd),
        _ => None,
    };
    from_env.or(config) == Some(GlyphSet::Ascii)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config_files;
    use crate::parallel::FETCH_CONCURRENCY;
    use crate::testutil::unique_dir;
    use crate::tui::fetch::DEFAULT_FETCH_MS;
    use crate::tui::theme::DEFAULT_THEME_ID;
    use crate::tui::ui_state_store::{save_activity_sparkline, save_theme};
    use crate::tui::watch::DEFAULT_WATCH_MS;
    use std::fs;
    use std::path::Path;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl FnMut(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    fn all_keys() -> RuntimeKeys {
        RuntimeKeys {
            theme: Some(ThemeId::Dracula),
            theme_mode: Some(ThemeMode::Light),
            glyphs: Some(GlyphSet::Ascii),
            watch_ms: Some(4000),
            fetch_ms: Some(0),
            fetch_concurrency: Some(3),
            update_check: Some(false),
            update_check_store: Some(PathBuf::from("/cfg/update-check.json")),
            comment_store: Some(PathBuf::from("/cfg/comments.json")),
            viewed_store: Some(PathBuf::from("/cfg/viewed.json")),
            ui_store: Some(PathBuf::from("/cfg/ui-state.json")),
            git: Some(PathBuf::from("/cfg/git")),
            term_progress: Some(false),
            smooth_scroll: Some(false),
            pr_polling: Some(false),
            pr_title_command: Some(PathBuf::from("/cfg/pr-title")),
        }
    }

    #[test]
    fn no_env_and_no_config_keeps_the_defaults() {
        let got = Settings::resolve(
            &RuntimeKeys::default(),
            env_of(&[("XDG_STATE_HOME", "/xdg/state")]),
        );
        assert_eq!(got.theme, DEFAULT_THEME_ID);
        assert_eq!(got.theme_mode, ThemeMode::Auto);
        assert!(!got.ascii);
        assert_eq!(got.watch_ms, DEFAULT_WATCH_MS);
        assert_eq!(got.fetch_ms, DEFAULT_FETCH_MS);
        assert_eq!(got.fetch_concurrency, FETCH_CONCURRENCY);
        assert!(got.update_check);
        assert_eq!(
            got.update_check_store,
            Path::new("/xdg/state/my-workspace-status/update-check.json")
        );
        assert_eq!(
            got.comment_store,
            Path::new("/xdg/state/my-workspace-status/comments.json")
        );
        assert_eq!(
            got.viewed_store,
            Path::new("/xdg/state/my-workspace-status/viewed-files.json")
        );
        assert_eq!(
            got.ui_store,
            Path::new("/xdg/state/my-workspace-status/ui-state.json")
        );
        assert_eq!(
            got.fetch_store,
            Path::new("/xdg/state/my-workspace-status/background-fetch.json")
        );
        assert_eq!(got.git, resolve_git_binary(None, None));
        assert!(got.term_progress, "terminal progress is on by default");
        assert!(got.smooth_scroll, "smooth scroll is on by default");
        assert_eq!(got.pr_poll, PrPollMode::HotCalm, "PR polling is on");
        assert_eq!(got.pr_title_command, None, "no title command");
        assert!(
            !got.activity_sparkline,
            "activity sparkline is off by default"
        );
    }

    #[test]
    fn config_applies_when_env_is_unset() {
        let got = Settings::resolve(&all_keys(), env_of(&[]));
        assert_eq!(
            got,
            Settings {
                theme: ThemeId::Dracula,
                theme_mode: ThemeMode::Light,
                ascii: true,
                watch_ms: 4000,
                fetch_ms: 0,
                fetch_concurrency: 3,
                update_check: false,
                update_check_store: PathBuf::from("/cfg/update-check.json"),
                comment_store: PathBuf::from("/cfg/comments.json"),
                viewed_store: PathBuf::from("/cfg/viewed.json"),
                ui_store: PathBuf::from("/cfg/ui-state.json"),
                activity_sparkline: false,
                commit_msg_lines: None,
                fetch_store: PathBuf::from(
                    "/.local/state/my-workspace-status/background-fetch.json"
                ),
                git: PathBuf::from("/cfg/git"),
                no_color: false,
                undercurl: false,
                term_progress: false,
                smooth_scroll: false,
                pr_poll: PrPollMode::Off,
                pr_title_command: Some(PathBuf::from("/cfg/pr-title")),
            }
        );
    }

    #[test]
    fn valid_env_wins_over_config() {
        let env = [
            ("WS_STATUS_THEME", "monokai"),
            ("WS_STATUS_THEME_MODE", "dark"),
            ("WS_STATUS_GLYPHS", "nerd"),
            ("WS_STATUS_WATCH_MS", "0"),
            ("WS_STATUS_FETCH_MS", "600000"),
            ("WS_STATUS_FETCH_CONCURRENCY", "7"),
            ("WS_STATUS_UPDATE_CHECK", "1"),
            ("WS_STATUS_UPDATE_CHECK_STORE", "/env/update-check.json"),
            ("WS_STATUS_COMMENT_STORE", "/env/comments.json"),
            ("WS_STATUS_VIEWED_STORE", "/env/viewed.json"),
            ("WS_STATUS_UI_STORE", "/env/ui-state.json"),
            ("WORKSPACE_STATUS_GIT", "/env/git"),
            ("WS_STATUS_TERM_PROGRESS", "on"),
            ("WS_STATUS_SMOOTH_SCROLL", "1"),
            ("WS_STATUS_PR_POLL_MS", "300"),
            ("WS_STATUS_PR_TITLE_CMD", " /env/pr-title "),
        ];
        let got = Settings::resolve(&all_keys(), env_of(&env));
        assert_eq!(
            got,
            Settings {
                theme: ThemeId::Monokai,
                theme_mode: ThemeMode::Dark,
                ascii: false,
                watch_ms: 0,
                fetch_ms: 600_000,
                fetch_concurrency: 7,
                update_check: true,
                update_check_store: PathBuf::from("/env/update-check.json"),
                comment_store: PathBuf::from("/env/comments.json"),
                viewed_store: PathBuf::from("/env/viewed.json"),
                ui_store: PathBuf::from("/env/ui-state.json"),
                activity_sparkline: false,
                commit_msg_lines: None,
                fetch_store: PathBuf::from(
                    "/.local/state/my-workspace-status/background-fetch.json"
                ),
                git: PathBuf::from("/env/git"),
                no_color: false,
                undercurl: false,
                term_progress: true,
                smooth_scroll: true,
                pr_poll: PrPollMode::Every(300),
                pr_title_command: Some(PathBuf::from("/env/pr-title")),
            }
        );
    }

    #[test]
    fn invalid_env_falls_through_to_config() {
        let env = [
            ("WS_STATUS_THEME", "solarized"),
            ("WS_STATUS_THEME_MODE", "Dark"),
            ("WS_STATUS_GLYPHS", "ASCII"),
            ("WS_STATUS_WATCH_MS", "-1"),
            ("WS_STATUS_FETCH_MS", "soon"),
            ("WS_STATUS_FETCH_CONCURRENCY", "0"),
            ("WS_STATUS_UPDATE_CHECK", "maybe"),
            ("WS_STATUS_UPDATE_CHECK_STORE", "  "),
            ("WS_STATUS_COMMENT_STORE", ""),
            ("WS_STATUS_VIEWED_STORE", " "),
            ("WS_STATUS_UI_STORE", "  "),
            ("WORKSPACE_STATUS_GIT", ""),
            ("WS_STATUS_TERM_PROGRESS", "maybe"),
            ("WS_STATUS_SMOOTH_SCROLL", "maybe"),
            ("WS_STATUS_PR_POLL_MS", "soon"),
            ("WS_STATUS_PR_TITLE_CMD", "  "),
        ];
        let got = Settings::resolve(&all_keys(), env_of(&env));
        assert_eq!(got, Settings::resolve(&all_keys(), env_of(&[])));
    }

    #[test]
    fn last_used_theme_beats_config_and_a_known_env_beats_it() {
        let root = unique_dir("ws-settings-theme");
        let user_file = root.join("config").join("config.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        let config_bytes = b"{\"theme\":\"dracula\"}\n";
        fs::write(&user_file, config_bytes).unwrap();
        let keys = load_config_files(Some(&user_file), &workspace)
            .unwrap()
            .runtime;
        assert_eq!(keys.theme, Some(ThemeId::Dracula));
        let store = root.join("state").join("ui-state.json");
        let store_s = store.display().to_string();
        let resolve = |theme_env: Option<&str>| {
            Settings::resolve(&keys, |key| match key {
                "WS_STATUS_UI_STORE" => Some(store_s.clone()),
                "WS_STATUS_THEME" => theme_env.map(str::to_string),
                _ => None,
            })
        };

        let first = resolve(None);
        assert_eq!(first.ui_store, store);
        assert_eq!(first.theme, ThemeId::Dracula, "no state: config applies");
        assert!(!store.exists(), "resolve never creates the store");

        save_theme(&first.ui_store, ThemeId::Nord).unwrap();
        assert_eq!(resolve(None).theme, ThemeId::Nord, "state beats config");
        assert_eq!(
            resolve(Some("nope")).theme,
            ThemeId::Nord,
            "unknown env falls through to state"
        );
        assert_eq!(
            resolve(Some("monokai")).theme,
            ThemeId::Monokai,
            "known env beats state"
        );
        save_theme(&first.ui_store, ThemeId::Slate).unwrap();
        assert_eq!(
            resolve(None).theme,
            ThemeId::Slate,
            "a saved Slate beats config"
        );
        assert!(
            !resolve(None).activity_sparkline,
            "a theme save keeps it off"
        );
        save_activity_sparkline(&first.ui_store, true).unwrap();
        let on = resolve(None);
        assert!(on.activity_sparkline, "the stored sparkline loads");
        assert_eq!(on.theme, ThemeId::Slate, "a sparkline save keeps the theme");
        assert_eq!(
            fs::read(&user_file).unwrap(),
            config_bytes,
            "store saves never rewrite the config file"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ui_store_naming_the_config_file_never_rewrites_it() {
        let root = unique_dir("ws-settings-theme-foreign");
        let user_file = root.join("config.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let config_json = format!(
            "{}\n",
            serde_json::json!({"version": 1, "theme": "dracula", "uiStore": user_file})
        );
        fs::write(&user_file, &config_json).unwrap();
        let keys = load_config_files(Some(&user_file), &workspace)
            .unwrap()
            .runtime;
        let settings = Settings::resolve(&keys, |_| None);
        assert_eq!(settings.ui_store, user_file);
        assert_eq!(settings.theme, ThemeId::Dracula, "config is not a store");
        assert!(!settings.activity_sparkline, "config is not a store");
        let err = save_theme(&settings.ui_store, ThemeId::Monokai).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&user_file).unwrap(), config_json);
        let err = save_activity_sparkline(&settings.ui_store, true).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&user_file).unwrap(), config_json);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_color_env_non_empty_turns_on() {
        for value in ["1", "0", "true", "anything"] {
            let got = Settings::resolve(&all_keys(), env_of(&[("NO_COLOR", value)]));
            assert!(got.no_color, "NO_COLOR={value:?} turns no-colour on");
        }
    }

    #[test]
    fn no_color_empty_is_off() {
        assert!(!Settings::resolve(&all_keys(), env_of(&[("NO_COLOR", "")])).no_color);
        assert!(!Settings::resolve(&all_keys(), env_of(&[])).no_color);
    }

    #[test]
    fn undercurl_needs_truecolor_and_no_fallback() {
        let keys = |glyphs| RuntimeKeys {
            glyphs,
            ..RuntimeKeys::default()
        };
        let on = |keys: &RuntimeKeys, env: &[(&str, &str)]| {
            Settings::resolve(keys, env_of(env)).undercurl
        };
        let nerd = keys(None);
        assert!(on(&nerd, &[("COLORTERM", "truecolor")]));
        assert!(on(&nerd, &[("COLORTERM", "24bit")]));
        assert!(!on(&nerd, &[]), "no COLORTERM");
        assert!(!on(&nerd, &[("COLORTERM", "256color")]));
        assert!(!on(&nerd, &[("COLORTERM", "truecolor"), ("NO_COLOR", "1")]));
        assert!(!on(
            &nerd,
            &[("COLORTERM", "truecolor"), ("WS_STATUS_GLYPHS", "ascii")]
        ));
        assert!(!on(
            &keys(Some(GlyphSet::Ascii)),
            &[("COLORTERM", "truecolor")]
        ));
    }

    #[test]
    fn theme_mode_env_beats_config_and_defaults_to_auto() {
        let config = |mode: Option<ThemeMode>| RuntimeKeys {
            theme_mode: mode,
            ..RuntimeKeys::default()
        };
        let resolve = |keys: &RuntimeKeys, env: &[(&str, &str)]| {
            Settings::resolve(keys, env_of(env)).theme_mode
        };
        assert_eq!(resolve(&config(None), &[]), ThemeMode::Auto, "default");
        assert_eq!(
            resolve(&config(Some(ThemeMode::Light)), &[]),
            ThemeMode::Light,
            "config applies without env"
        );
        for (env, want) in [
            ("auto", ThemeMode::Auto),
            ("dark", ThemeMode::Dark),
            ("light", ThemeMode::Light),
        ] {
            assert_eq!(
                resolve(
                    &config(Some(ThemeMode::Light)),
                    &[("WS_STATUS_THEME_MODE", env)]
                ),
                want,
                "env {env} beats config light"
            );
            assert_eq!(
                resolve(&config(None), &[("WS_STATUS_THEME_MODE", env)]),
                want,
                "env {env} without config"
            );
        }
        assert_eq!(
            resolve(
                &config(Some(ThemeMode::Dark)),
                &[("WS_STATUS_THEME_MODE", "light")]
            ),
            ThemeMode::Light,
            "env light beats config dark"
        );
    }

    #[test]
    fn term_progress_env_beats_config_default_on() {
        let resolve = |config: Option<bool>, env: &[(&str, &str)]| {
            let keys = RuntimeKeys {
                term_progress: config,
                ..RuntimeKeys::default()
            };
            Settings::resolve(&keys, env_of(env)).term_progress
        };
        assert!(resolve(None, &[]), "default on");
        assert!(!resolve(Some(false), &[]), "config off applies without env");
        for off in ["0", "false", "OFF", " off "] {
            assert!(
                !resolve(Some(true), &[("WS_STATUS_TERM_PROGRESS", off)]),
                "env {off:?} beats config on"
            );
        }
        for on in ["1", "true", "On"] {
            assert!(
                resolve(Some(false), &[("WS_STATUS_TERM_PROGRESS", on)]),
                "env {on:?} beats config off"
            );
        }
        for unknown in ["", "yes", "2"] {
            assert!(
                !resolve(Some(false), &[("WS_STATUS_TERM_PROGRESS", unknown)]),
                "env {unknown:?} falls through to config"
            );
            assert!(
                resolve(None, &[("WS_STATUS_TERM_PROGRESS", unknown)]),
                "env {unknown:?} falls through to the default"
            );
        }
    }

    #[test]
    fn smooth_scroll_env_beats_config() {
        let resolve = |config: Option<bool>, env: &[(&str, &str)]| {
            let keys = RuntimeKeys {
                smooth_scroll: config,
                ..RuntimeKeys::default()
            };
            Settings::resolve(&keys, env_of(env)).smooth_scroll
        };
        assert!(resolve(None, &[]), "default on");
        assert!(!resolve(Some(false), &[]), "config off applies without env");
        for off in ["0", "false", "OFF"] {
            assert!(
                !resolve(Some(true), &[("WS_STATUS_SMOOTH_SCROLL", off)]),
                "env {off:?} beats config on"
            );
        }
        for on in ["1", "true", "on"] {
            assert!(
                resolve(Some(false), &[("WS_STATUS_SMOOTH_SCROLL", on)]),
                "env {on:?} beats config off"
            );
        }
        assert!(
            !resolve(Some(false), &[("WS_STATUS_SMOOTH_SCROLL", "yes")]),
            "an unknown env value falls through to config"
        );
    }

    #[test]
    fn pr_poll_env_beats_config_and_title_command_env_beats_config() {
        let resolve = |polling: Option<bool>, title: Option<&str>, env: &[(&str, &str)]| {
            let keys = RuntimeKeys {
                pr_polling: polling,
                pr_title_command: title.map(PathBuf::from),
                ..RuntimeKeys::default()
            };
            let got = Settings::resolve(&keys, env_of(env));
            (got.pr_poll, got.pr_title_command)
        };
        assert_eq!(resolve(None, None, &[]), (PrPollMode::HotCalm, None));
        assert_eq!(
            resolve(Some(false), None, &[]).0,
            PrPollMode::Off,
            "config prPolling false"
        );
        assert_eq!(
            resolve(Some(false), None, &[("WS_STATUS_PR_POLL_MS", "300")]).0,
            PrPollMode::Every(300),
            "a valid env period wins over config off"
        );
        assert_eq!(
            resolve(Some(true), None, &[("WS_STATUS_PR_POLL_MS", "0")]).0,
            PrPollMode::Off,
            "env 0 wins over config on"
        );
        for bad in ["", "-1", "abc"] {
            assert_eq!(
                resolve(Some(false), None, &[("WS_STATUS_PR_POLL_MS", bad)]).0,
                PrPollMode::Off,
                "env {bad:?} falls through to config"
            );
        }
        assert_eq!(
            resolve(None, Some("/cfg/pr-title"), &[]).1,
            Some(PathBuf::from("/cfg/pr-title")),
            "config applies"
        );
        assert_eq!(
            resolve(
                None,
                Some("/cfg/pr-title"),
                &[("WS_STATUS_PR_TITLE_CMD", "pr-title")]
            )
            .1,
            Some(PathBuf::from("pr-title")),
            "env wins"
        );
        assert_eq!(
            resolve(None, None, &[("WS_STATUS_PR_TITLE_CMD", " ")]).1,
            None,
            "blank env is unset"
        );
    }

    #[test]
    fn unknown_theme_mode_env_falls_through() {
        for env in ["", " dark", "Light", "system", "1"] {
            let got = Settings::resolve(
                &RuntimeKeys {
                    theme_mode: Some(ThemeMode::Dark),
                    ..RuntimeKeys::default()
                },
                env_of(&[("WS_STATUS_THEME_MODE", env)]),
            );
            assert_eq!(got.theme_mode, ThemeMode::Dark, "env {env:?} -> config");
            let got = Settings::resolve(
                &RuntimeKeys::default(),
                env_of(&[("WS_STATUS_THEME_MODE", env)]),
            );
            assert_eq!(got.theme_mode, ThemeMode::Auto, "env {env:?} -> default");
        }
    }

    #[test]
    fn glyphs_env_and_config() {
        assert!(glyphs_ascii(Some("ascii"), None));
        assert!(glyphs_ascii(Some("ascii"), Some(GlyphSet::Nerd)));
        assert!(!glyphs_ascii(Some("nerd"), Some(GlyphSet::Ascii)));
        assert!(glyphs_ascii(None, Some(GlyphSet::Ascii)));
        assert!(glyphs_ascii(Some("1"), Some(GlyphSet::Ascii)));
        assert!(!glyphs_ascii(Some("1"), None));
        assert!(!glyphs_ascii(None, None));
    }
}
