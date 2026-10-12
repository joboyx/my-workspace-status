//! Persist the last-used TUI theme in a state file.
//!
//! The theme that `T` picks is state, not config: it lives in its own JSON
//! file (`$XDG_STATE_HOME/my-workspace-status/theme.json`) and the user
//! config file is never written. [`crate::settings::Settings`] loads it at
//! launch; see [`super::theme::theme_id_from`] for the precedence.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::persist::persist_with_lock;
use super::theme::{parse_theme_id, ThemeId};

/// On-disk store version written by save. Load accepts only this version.
pub const THEME_STORE_VERSION: u32 = 1;

/// File name under the state directory.
const STORE_FILE_NAME: &str = "theme.json";

/// On-disk shape: `{"version": 1, "theme": "<slug>"}`.
///
/// Exactly these two keys. An object with any other key (for example a
/// config file that also has `version` and `theme`) is not a store: load
/// ignores it and save leaves it unchanged.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreFile {
    version: u32,
    theme: String,
}

/// Resolve the store path from an env lookup and the config `themeStore` value.
///
/// A non-blank `WS_STATUS_THEME_STORE` wins, then `config`, then
/// `$XDG_STATE_HOME/my-workspace-status/theme.json` (else
/// `$HOME/.local/state/...`).
pub fn theme_store_path_from_env<F>(mut get: F, config: Option<&Path>) -> PathBuf
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(override_path) = get("WS_STATUS_THEME_STORE") {
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

/// Last-used theme from the store at `path`.
///
/// `None` when the file is missing or unreadable, is not valid store JSON
/// (extra keys included), has another version, or names an unknown theme
/// slug.
pub fn load_last_theme(path: &Path) -> Option<ThemeId> {
    let text = fs::read_to_string(path).ok()?;
    let parsed: StoreFile = serde_json::from_str(&text).ok()?;
    if parsed.version != THEME_STORE_VERSION {
        return None;
    }
    parse_theme_id(&parsed.theme)
}

/// True when `bytes` are a UTF-8 JSON object with exactly the store fields
/// (any version, any slug).
fn is_store_file(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .filter(serde_json::Value::is_object)
        .is_some_and(|value| serde_json::from_value::<StoreFile>(value).is_ok())
}

/// Persist `theme` as the last-used theme.
///
/// A missing file (and its parent directories) is created. An existing file
/// must parse as this store (exactly `version` and `theme`, any version, any
/// slug); otherwise it is left byte-identical and the result is an
/// [`io::ErrorKind::InvalidData`] error (like the viewed and comment stores,
/// a config path that names some other file never overwrites it). The write
/// takes the sibling lock and replaces the file atomically.
pub fn save_last_theme(path: &Path, theme: ThemeId) -> io::Result<()> {
    persist_with_lock(path, |existing| {
        if existing.is_some_and(|bytes| !is_store_file(bytes)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a theme store, left unchanged",
            ));
        }
        let file = StoreFile {
            version: THEME_STORE_VERSION,
            theme: theme.as_str().to_string(),
        };
        let mut body = serde_json::to_string_pretty(&file).map_err(io::Error::other)?;
        body.push('\n');
        Ok(body.into_bytes())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::unique_dir;
    use crate::tui::theme::THEME_IDS;
    use std::fs;

    fn temp_store() -> (PathBuf, PathBuf) {
        let dir = unique_dir("ws-theme-store");
        let path = dir.join("state").join(STORE_FILE_NAME);
        (dir, path)
    }

    #[test]
    fn store_path_env_beats_config_then_xdg_then_home() {
        let config = Path::new("/cfg/theme.json");
        let env_and_xdg = |k: &str| match k {
            "WS_STATUS_THEME_STORE" => Some("/env/theme.json".into()),
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            "HOME" => Some("/home/demo".into()),
            _ => None,
        };
        assert_eq!(
            theme_store_path_from_env(env_and_xdg, Some(config)),
            PathBuf::from("/env/theme.json"),
            "env wins"
        );
        for blank in ["", "   "] {
            let blank_env = |k: &str| match k {
                "WS_STATUS_THEME_STORE" => Some(blank.to_string()),
                "XDG_STATE_HOME" => Some("/xdg/state".into()),
                _ => None,
            };
            assert_eq!(
                theme_store_path_from_env(blank_env, Some(config)),
                config,
                "blank env {blank:?} falls through to config"
            );
            assert_eq!(
                theme_store_path_from_env(blank_env, None),
                PathBuf::from("/xdg/state/my-workspace-status/theme.json"),
                "blank env {blank:?} without config falls through to XDG"
            );
        }
        let xdg_and_home = |k: &str| match k {
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            "HOME" => Some("/home/demo".into()),
            _ => None,
        };
        assert_eq!(
            theme_store_path_from_env(xdg_and_home, Some(config)),
            config
        );
        assert_eq!(
            theme_store_path_from_env(xdg_and_home, None),
            PathBuf::from("/xdg/state/my-workspace-status/theme.json"),
            "XDG beats HOME"
        );
        for blank_xdg in [None, Some(""), Some("  ")] {
            let home_only = |k: &str| match k {
                "XDG_STATE_HOME" => blank_xdg.map(str::to_string),
                "HOME" => Some("/home/demo".into()),
                _ => None,
            };
            assert_eq!(
                theme_store_path_from_env(home_only, None),
                PathBuf::from("/home/demo/.local/state/my-workspace-status/theme.json"),
                "XDG {blank_xdg:?} falls through to HOME"
            );
        }
    }

    #[test]
    fn round_trip_every_theme_as_pretty_json() {
        let (dir, path) = temp_store();
        assert!(THEME_IDS.contains(&ThemeId::Slate));
        for id in THEME_IDS {
            save_last_theme(&path, id).unwrap();
            assert_eq!(load_last_theme(&path), Some(id), "{id:?}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!(
                    "{{\n  \"version\": 1,\n  \"theme\": \"{}\"\n}}\n",
                    id.as_str()
                ),
                "{id:?}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn store_round_trips_a_light_slug() {
        let (dir, path) = temp_store();
        save_last_theme(&path, ThemeId::CatppuccinLatte).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"catppuccin-latte\"\n}\n"
        );
        assert_eq!(load_last_theme(&path), Some(ThemeId::CatppuccinLatte));
        fs::write(&path, "{\"version\":1,\"theme\":\"solarized-light\"}\n").unwrap();
        assert_eq!(load_last_theme(&path), Some(ThemeId::SolarizedLight));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let (dir, path) = temp_store();
        let path = path
            .parent()
            .unwrap()
            .join("a")
            .join("b")
            .join("theme.json");
        assert!(!dir.exists());
        save_last_theme(&path, ThemeId::Nord).unwrap();
        assert_eq!(load_last_theme(&path), Some(ThemeId::Nord));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_falls_through_to_none() {
        let (dir, path) = temp_store();
        assert_eq!(load_last_theme(&path), None, "missing file");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for body in [
            "{not json",
            "",
            "[1, 2]\n",
            "{\"theme\":\"dracula\"}\n",
            "{\"version\":1}\n",
            "{\"version\":1,\"theme\":\"solarized\"}\n",
            "{\"version\":1,\"theme\":\"Dracula\"}\n",
            "{\"version\":2,\"theme\":\"dracula\"}\n",
            "{\"version\":0,\"theme\":\"dracula\"}\n",
            "{\"version\":1,\"theme\":\"dracula\",\"ignoredRepos\":[]}\n",
        ] {
            fs::write(&path, body).unwrap();
            assert_eq!(load_last_theme(&path), None, "{body:?}");
        }
        fs::write(&path, "{\"version\":1,\"theme\":\"dracula\"}").unwrap();
        assert_eq!(load_last_theme(&path), Some(ThemeId::Dracula), "compact");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_never_overwrites_a_file_that_is_not_the_store() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for body in [
            "{\"theme\":\"dracula\"}\n",
            "{\"ignoredRepos\":[],\"theme\":\"monokai\",\"watchMs\":0}\n",
            "export PATH=$HOME/bin:$PATH\n",
            "",
            "[1, 2]\n",
            "{\"version\":\"1\",\"theme\":\"dracula\"}\n",
            "{\"version\":1,\"theme\":\"dracula\",\"ignoredRepos\":[]}\n",
        ] {
            fs::write(&path, body).unwrap();
            let err = save_last_theme(&path, ThemeId::Monokai).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{body:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), body, "byte-identical");
            assert!(!path.with_extension("json.tmp").exists());
        }
        let bytes = [0xff_u8, 0xfe, 0x00];
        fs::write(&path, bytes).unwrap();
        let err = save_last_theme(&path, ThemeId::Monokai).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), bytes, "non-UTF-8 left as is");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_replaces_an_own_store_with_an_unknown_slug_or_version() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for body in [
            "{\"version\":1,\"theme\":\"solarized\"}\n",
            "{\"version\":99,\"theme\":\"dracula\"}\n",
        ] {
            fs::write(&path, body).unwrap();
            assert_eq!(load_last_theme(&path), None, "{body:?}");
            save_last_theme(&path, ThemeId::Kanagawa).unwrap();
            assert_eq!(load_last_theme(&path), Some(ThemeId::Kanagawa), "{body:?}");
        }
        let _ = fs::remove_dir_all(dir);
    }
}
