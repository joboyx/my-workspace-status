//! Env-backed settings, resolved once at startup.
//!
//! Each setting has an env var and a config key ([`RuntimeKeys`]).
//! Precedence: CLI flag > env var > config file > built-in default. An env
//! value that the env parser accepts wins. An unset, blank, unknown, or
//! non-numeric env value falls through to the config value, then the
//! default. `cli.rs` resolves [`Settings`] after it loads the config and
//! before the update check and the first git call, then passes it on.
//!
//! The launch theme also reads the last-used theme state file
//! ([`crate::tui::theme_store`]) between the env var and the config value.
//! Resolve only reads that file; it never creates or writes it.

use std::env;
use std::path::PathBuf;

use crate::config::{GlyphSet, RuntimeKeys};
use crate::git::resolve_git_binary;
use crate::parallel::fetch_concurrency;
use crate::tui::comments::comment_store_path_from_env;
use crate::tui::fetch::fetch_interval_ms;
use crate::tui::theme::{theme_id_from, ThemeId};
use crate::tui::theme_store::{load_last_theme, theme_store_path_from_env};
use crate::tui::viewed::viewed_store_path_from_env;
use crate::tui::watch::watch_interval_ms;
use crate::update_check::{
    update_check_enabled, update_check_store_path_from_env, UPDATE_CHECK_ENV,
};

/// The resolved env-backed settings for one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Launch theme: `WS_STATUS_THEME`, then the last-used theme in
    /// [`Self::theme_store`], then config `theme`, then Slate.
    pub(crate) theme: ThemeId,
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
    /// Last-used theme JSON file (`WS_STATUS_THEME_STORE` / `themeStore`).
    pub(crate) theme_store: PathBuf,
    /// Git binary (`WORKSPACE_STATUS_GIT` / `git`).
    pub(crate) git: PathBuf,
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
        let theme_store = theme_store_path_from_env(&mut get, keys.theme_store.as_deref());
        Settings {
            theme: theme_id_from(
                get("WS_STATUS_THEME").as_deref(),
                load_last_theme(&theme_store),
                keys.theme,
            ),
            ascii: glyphs_ascii(get("WS_STATUS_GLYPHS").as_deref(), keys.glyphs),
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
            theme_store,
            git: resolve_git_binary(get("WORKSPACE_STATUS_GIT").as_deref(), keys.git.as_deref()),
        }
    }
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
    use crate::tui::theme_store::save_last_theme;
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
            glyphs: Some(GlyphSet::Ascii),
            watch_ms: Some(4000),
            fetch_ms: Some(0),
            fetch_concurrency: Some(3),
            update_check: Some(false),
            update_check_store: Some(PathBuf::from("/cfg/update-check.json")),
            comment_store: Some(PathBuf::from("/cfg/comments.json")),
            viewed_store: Some(PathBuf::from("/cfg/viewed.json")),
            theme_store: Some(PathBuf::from("/cfg/theme.json")),
            git: Some(PathBuf::from("/cfg/git")),
        }
    }

    #[test]
    fn no_env_and_no_config_keeps_the_defaults() {
        let got = Settings::resolve(
            &RuntimeKeys::default(),
            env_of(&[("XDG_STATE_HOME", "/xdg/state")]),
        );
        assert_eq!(got.theme, DEFAULT_THEME_ID);
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
            got.theme_store,
            Path::new("/xdg/state/my-workspace-status/theme.json")
        );
        assert_eq!(got.git, resolve_git_binary(None, None));
    }

    #[test]
    fn config_applies_when_env_is_unset() {
        let got = Settings::resolve(&all_keys(), env_of(&[]));
        assert_eq!(
            got,
            Settings {
                theme: ThemeId::Dracula,
                ascii: true,
                watch_ms: 4000,
                fetch_ms: 0,
                fetch_concurrency: 3,
                update_check: false,
                update_check_store: PathBuf::from("/cfg/update-check.json"),
                comment_store: PathBuf::from("/cfg/comments.json"),
                viewed_store: PathBuf::from("/cfg/viewed.json"),
                theme_store: PathBuf::from("/cfg/theme.json"),
                git: PathBuf::from("/cfg/git"),
            }
        );
    }

    #[test]
    fn valid_env_wins_over_config() {
        let env = [
            ("WS_STATUS_THEME", "monokai"),
            ("WS_STATUS_GLYPHS", "nerd"),
            ("WS_STATUS_WATCH_MS", "0"),
            ("WS_STATUS_FETCH_MS", "600000"),
            ("WS_STATUS_FETCH_CONCURRENCY", "7"),
            ("WS_STATUS_UPDATE_CHECK", "1"),
            ("WS_STATUS_UPDATE_CHECK_STORE", "/env/update-check.json"),
            ("WS_STATUS_COMMENT_STORE", "/env/comments.json"),
            ("WS_STATUS_VIEWED_STORE", "/env/viewed.json"),
            ("WS_STATUS_THEME_STORE", "/env/theme.json"),
            ("WORKSPACE_STATUS_GIT", "/env/git"),
        ];
        let got = Settings::resolve(&all_keys(), env_of(&env));
        assert_eq!(
            got,
            Settings {
                theme: ThemeId::Monokai,
                ascii: false,
                watch_ms: 0,
                fetch_ms: 600_000,
                fetch_concurrency: 7,
                update_check: true,
                update_check_store: PathBuf::from("/env/update-check.json"),
                comment_store: PathBuf::from("/env/comments.json"),
                viewed_store: PathBuf::from("/env/viewed.json"),
                theme_store: PathBuf::from("/env/theme.json"),
                git: PathBuf::from("/env/git"),
            }
        );
    }

    #[test]
    fn invalid_env_falls_through_to_config() {
        let env = [
            ("WS_STATUS_THEME", "solarized-light"),
            ("WS_STATUS_GLYPHS", "ASCII"),
            ("WS_STATUS_WATCH_MS", "-1"),
            ("WS_STATUS_FETCH_MS", "soon"),
            ("WS_STATUS_FETCH_CONCURRENCY", "0"),
            ("WS_STATUS_UPDATE_CHECK", "maybe"),
            ("WS_STATUS_UPDATE_CHECK_STORE", "  "),
            ("WS_STATUS_COMMENT_STORE", ""),
            ("WS_STATUS_VIEWED_STORE", " "),
            ("WS_STATUS_THEME_STORE", "  "),
            ("WORKSPACE_STATUS_GIT", ""),
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
        let store = root.join("state").join("theme.json");
        let store_s = store.display().to_string();
        let resolve = |theme_env: Option<&str>| {
            Settings::resolve(&keys, |key| match key {
                "WS_STATUS_THEME_STORE" => Some(store_s.clone()),
                "WS_STATUS_THEME" => theme_env.map(str::to_string),
                _ => None,
            })
        };

        let first = resolve(None);
        assert_eq!(first.theme_store, store);
        assert_eq!(first.theme, ThemeId::Dracula, "no state: config applies");
        assert!(!store.exists(), "resolve never creates the store");

        save_last_theme(&first.theme_store, ThemeId::Nord).unwrap();
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
        save_last_theme(&first.theme_store, ThemeId::Slate).unwrap();
        assert_eq!(
            resolve(None).theme,
            ThemeId::Slate,
            "a saved Slate beats config"
        );
        assert_eq!(
            fs::read(&user_file).unwrap(),
            config_bytes,
            "theme saves never rewrite the config file"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn theme_store_naming_the_config_file_never_rewrites_it() {
        let root = unique_dir("ws-settings-theme-foreign");
        let user_file = root.join("config.json");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let config_json = format!(
            "{}\n",
            serde_json::json!({"version": 1, "theme": "dracula", "themeStore": user_file})
        );
        fs::write(&user_file, &config_json).unwrap();
        let keys = load_config_files(Some(&user_file), &workspace)
            .unwrap()
            .runtime;
        let settings = Settings::resolve(&keys, |_| None);
        assert_eq!(settings.theme_store, user_file);
        assert_eq!(settings.theme, ThemeId::Dracula, "config is not a store");
        let err = save_last_theme(&settings.theme_store, ThemeId::Monokai).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&user_file).unwrap(), config_json);
        let _ = fs::remove_dir_all(root);
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
