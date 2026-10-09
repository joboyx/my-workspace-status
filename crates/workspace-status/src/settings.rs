//! Env-backed settings, resolved once at startup.
//!
//! Each setting has an env var and a config key ([`RuntimeKeys`]).
//! Precedence: CLI flag > env var > config file > built-in default. An env
//! value that the env parser accepts wins. An unset, blank, unknown, or
//! non-numeric env value falls through to the config value, then the
//! default. `cli.rs` resolves [`Settings`] after it loads the config and
//! before the update check and the first git call, then passes it on.

use std::env;
use std::path::PathBuf;

use crate::config::{GlyphSet, RuntimeKeys};
use crate::git::resolve_git_binary;
use crate::parallel::fetch_concurrency;
use crate::tui::comments::comment_store_path_from_env;
use crate::tui::fetch::fetch_interval_ms;
use crate::tui::theme::{theme_id_from, ThemeId};
use crate::tui::viewed::viewed_store_path_from_env;
use crate::tui::watch::watch_interval_ms;
use crate::update_check::{
    update_check_enabled, update_check_store_path_from_env, UPDATE_CHECK_ENV,
};

/// The resolved env-backed settings for one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Launch theme (`WS_STATUS_THEME` / `theme`).
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
        Settings {
            theme: theme_id_from(get("WS_STATUS_THEME").as_deref(), keys.theme),
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
    use crate::parallel::FETCH_CONCURRENCY;
    use crate::tui::fetch::DEFAULT_FETCH_MS;
    use crate::tui::theme::DEFAULT_THEME_ID;
    use crate::tui::watch::DEFAULT_WATCH_MS;
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
            ("WORKSPACE_STATUS_GIT", ""),
        ];
        let got = Settings::resolve(&all_keys(), env_of(&env));
        assert_eq!(got, Settings::resolve(&all_keys(), env_of(&[])));
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
