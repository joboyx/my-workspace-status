//! Persist TUI view state (last-used theme, activity sparkline, commit
//! message rows) in a state file.
//!
//! The theme that a palette `Theme: …` row sets, the activity sparkline
//! toggle, and the commit-message footer height a mouse drag sets are
//! state, not config: they live in one JSON file
//! (`$XDG_STATE_HOME/my-workspace-status/ui-state.json`) and the user config
//! file is never written. [`crate::settings::Settings`] loads it at launch;
//! see [`super::theme::theme_id_from`] for the theme precedence.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use workspace_status_graph::{COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN};

use super::persist::persist_with_lock;
use super::theme::{parse_theme_id, ThemeId};

/// On-disk store version written by save. Load accepts only this version.
pub const UI_STATE_STORE_VERSION: u32 = 1;

/// File name under the state directory.
const STORE_FILE_NAME: &str = "ui-state.json";

/// On-disk shape:
/// `{"version": 1, "theme": "<slug>", "activitySparkline": false,
/// "commitMessageLines": 6}`.
///
/// `version` is required; `theme`, `activitySparkline`, and
/// `commitMessageLines` are optional. An object with any other key (for
/// example a config file that also has `version` and `theme`) is not a
/// store: load ignores it and save leaves it unchanged. Save keeps the raw
/// `theme` string and the raw `commitMessageLines` JSON value, so an unknown
/// slug or a bad row count survives a save of another key.
/// `commitMessageLines` takes any JSON value here: a bad value is only that
/// key unset ([`saved_commit_msg_lines`]), never the whole file foreign.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StoreFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(default)]
    activity_sparkline: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit_message_lines: Option<serde_json::Value>,
}

/// The view state the TUI restores at launch.
///
/// The default (no theme, sparkline off, no commit-message rows) is what a
/// missing, unreadable, or foreign file loads as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiState {
    /// Last-used theme; `None` when none was saved or the slug is unknown.
    pub theme: Option<ThemeId>,
    /// Paint the tree commit-activity sparkline. Off unless saved on.
    pub activity_sparkline: bool,
    /// Message rows of the expanded commit-message footer that the last
    /// mouse drag of its edge saved. `None` when none was saved or the
    /// saved value is not an integer in
    /// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`].
    pub commit_msg_lines: Option<usize>,
}

/// The saved `commitMessageLines` when it is a JSON integer in
/// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]; any other value
/// reads as not saved.
fn saved_commit_msg_lines(value: Option<&serde_json::Value>) -> Option<usize> {
    let n = value?.as_u64()?;
    let range = COMMIT_MSG_LINES_MIN as u64..=COMMIT_MSG_LINES_MAX as u64;
    range.contains(&n).then_some(n as usize)
}

/// Resolve the store path from an env lookup and the config `uiStore` value.
///
/// A non-blank `WS_STATUS_UI_STORE` wins, then `config`, then
/// `$XDG_STATE_HOME/my-workspace-status/ui-state.json` (else
/// `$HOME/.local/state/...`).
pub fn ui_store_path_from_env<F>(mut get: F, config: Option<&Path>) -> PathBuf
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(override_path) = get("WS_STATUS_UI_STORE") {
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

/// Parse `bytes` as this store (any version, any slug).
///
/// `None` unless the bytes are a UTF-8 JSON object with only the store keys
/// and a numeric `version`. The object check keeps a JSON array out, which
/// serde would otherwise accept as a struct.
fn parse_store(bytes: &[u8]) -> Option<StoreFile> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .filter(serde_json::Value::is_object)
        .and_then(|value| serde_json::from_value::<StoreFile>(value).ok())
}

/// Check the bytes of a present store file the way [`update_store`] does
/// before a save: they must parse as this store (any version). `Err` says
/// why a save would refuse the file and leave it unchanged. Read-only.
pub(crate) fn check_ui_state_bytes(bytes: &[u8]) -> Result<(), String> {
    parse_store(bytes)
        .map(|_| ())
        .ok_or_else(|| "not a UI-state store".to_string())
}

/// View state from the store at `path`.
///
/// [`UiState::default`] when the file is missing or unreadable, is not valid
/// store JSON (extra keys included), or has another version. An unknown
/// theme slug loads as no theme and keeps the other keys; a missing
/// `activitySparkline` is off; a missing or bad `commitMessageLines` (not an
/// integer in [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]) loads as
/// not saved and keeps the other keys.
pub fn load_ui_state(path: &Path) -> UiState {
    let Some(parsed) = fs::read(path).ok().and_then(|bytes| parse_store(&bytes)) else {
        return UiState::default();
    };
    if parsed.version != UI_STATE_STORE_VERSION {
        return UiState::default();
    }
    UiState {
        theme: parsed.theme.as_deref().and_then(parse_theme_id),
        activity_sparkline: parsed.activity_sparkline,
        commit_msg_lines: saved_commit_msg_lines(parsed.commit_message_lines.as_ref()),
    }
}

/// Read-modify-write the store at `path` under its lock.
///
/// A missing file (and its parent directories) is created from an empty
/// store. An existing file must parse as this store (any version); otherwise
/// it is left byte-identical and the result is an
/// [`io::ErrorKind::InvalidData`] error (like the viewed and comment stores,
/// a config path that names some other file never overwrites it). `update`
/// changes one field; the other fields keep their raw values. The write
/// takes the sibling lock and replaces the file atomically as pretty JSON
/// with a trailing newline.
fn update_store(path: &Path, update: impl FnOnce(&mut StoreFile)) -> io::Result<()> {
    persist_with_lock(path, |existing| {
        let mut file = match existing {
            None => StoreFile::default(),
            Some(bytes) => parse_store(bytes).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "not a UI-state store, left unchanged",
                )
            })?,
        };
        file.version = UI_STATE_STORE_VERSION;
        update(&mut file);
        let mut body = serde_json::to_string_pretty(&file).map_err(io::Error::other)?;
        body.push('\n');
        Ok(body.into_bytes())
    })
}

/// Persist `theme` as the last-used theme and keep the stored
/// `activitySparkline`.
///
/// See [`update_store`] for the create, foreign-file, and lock rules.
pub fn save_theme(path: &Path, theme: ThemeId) -> io::Result<()> {
    update_store(path, |file| file.theme = Some(theme.as_str().to_string()))
}

/// Persist the activity sparkline toggle and keep the stored `theme` (an
/// unknown slug included).
///
/// With no stored theme the file holds no `theme` key, so the session theme
/// is not pinned. See [`update_store`] for the create, foreign-file, and lock
/// rules.
pub fn save_activity_sparkline(path: &Path, on: bool) -> io::Result<()> {
    update_store(path, |file| file.activity_sparkline = on)
}

/// Persist the commit-message footer rows a drag of its edge set, and keep
/// the stored `theme` (an unknown slug included) and `activitySparkline`.
///
/// `lines` is clamped to [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`].
/// See [`update_store`] for the create, foreign-file, and lock rules.
pub fn save_commit_msg_lines(path: &Path, lines: usize) -> io::Result<()> {
    let lines = lines.clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX);
    update_store(path, |file| {
        file.commit_message_lines = Some(serde_json::Value::from(lines));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::unique_dir;
    use crate::tui::theme::THEME_IDS;
    use std::fs;

    /// One of the store saves, for tests that run each.
    type Save = fn(&Path) -> io::Result<()>;

    fn temp_store() -> (PathBuf, PathBuf) {
        let dir = unique_dir("ws-ui-state-store");
        let path = dir.join("state").join(STORE_FILE_NAME);
        (dir, path)
    }

    #[test]
    fn store_path_env_beats_config_then_xdg_then_home() {
        let config = Path::new("/cfg/ui-state.json");
        let env_and_xdg = |k: &str| match k {
            "WS_STATUS_UI_STORE" => Some("/env/ui-state.json".into()),
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            "HOME" => Some("/home/demo".into()),
            _ => None,
        };
        assert_eq!(
            ui_store_path_from_env(env_and_xdg, Some(config)),
            PathBuf::from("/env/ui-state.json"),
            "env wins"
        );
        for blank in ["", "   "] {
            let blank_env = |k: &str| match k {
                "WS_STATUS_UI_STORE" => Some(blank.to_string()),
                "XDG_STATE_HOME" => Some("/xdg/state".into()),
                _ => None,
            };
            assert_eq!(
                ui_store_path_from_env(blank_env, Some(config)),
                config,
                "blank env {blank:?} falls through to config"
            );
            assert_eq!(
                ui_store_path_from_env(blank_env, None),
                PathBuf::from("/xdg/state/my-workspace-status/ui-state.json"),
                "blank env {blank:?} without config falls through to XDG"
            );
        }
        let xdg_and_home = |k: &str| match k {
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            "HOME" => Some("/home/demo".into()),
            _ => None,
        };
        assert_eq!(ui_store_path_from_env(xdg_and_home, Some(config)), config);
        assert_eq!(
            ui_store_path_from_env(xdg_and_home, None),
            PathBuf::from("/xdg/state/my-workspace-status/ui-state.json"),
            "XDG beats HOME"
        );
        for blank_xdg in [None, Some(""), Some("  ")] {
            let home_only = |k: &str| match k {
                "XDG_STATE_HOME" => blank_xdg.map(str::to_string),
                "HOME" => Some("/home/demo".into()),
                _ => None,
            };
            assert_eq!(
                ui_store_path_from_env(home_only, None),
                PathBuf::from("/home/demo/.local/state/my-workspace-status/ui-state.json"),
                "XDG {blank_xdg:?} falls through to HOME"
            );
        }
    }

    #[test]
    fn round_trip_every_theme_as_pretty_json() {
        let (dir, path) = temp_store();
        assert!(THEME_IDS.contains(&ThemeId::Slate));
        for id in THEME_IDS {
            save_theme(&path, id).unwrap();
            assert_eq!(
                load_ui_state(&path),
                UiState {
                    theme: Some(id),
                    activity_sparkline: false,
                    commit_msg_lines: None,
                },
                "{id:?}"
            );
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!(
                    "{{\n  \"version\": 1,\n  \"theme\": \"{}\",\n  \"activitySparkline\": false\n}}\n",
                    id.as_str()
                ),
                "{id:?}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn theme_save_keeps_a_stored_sparkline() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"version\":1,\"activitySparkline\":true}\n").unwrap();
        save_theme(&path, ThemeId::Nord).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"nord\",\n  \"activitySparkline\": true\n}\n"
        );
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Nord),
                activity_sparkline: true,
                commit_msg_lines: None,
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sparkline_save_keeps_a_stored_theme_even_an_unknown_slug() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"version\":1,\"theme\":\"dracula\"}\n").unwrap();
        save_activity_sparkline(&path, true).unwrap();
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Dracula),
                activity_sparkline: true,
                commit_msg_lines: None,
            }
        );
        save_activity_sparkline(&path, false).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"dracula\",\n  \"activitySparkline\": false\n}\n"
        );

        fs::write(&path, "{\"version\":1,\"theme\":\"solarized\"}\n").unwrap();
        save_activity_sparkline(&path, true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"solarized\",\n  \"activitySparkline\": true\n}\n",
            "unknown slug kept as is"
        );
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: None,
                activity_sparkline: true,
                commit_msg_lines: None,
            },
            "unknown slug is no theme; the sparkline still loads"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sparkline_save_on_a_missing_file_writes_no_theme() {
        let (dir, path) = temp_store();
        assert!(!path.exists());
        save_activity_sparkline(&path, true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"activitySparkline\": true\n}\n"
        );
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: None,
                activity_sparkline: true,
                commit_msg_lines: None,
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn store_round_trips_a_light_slug() {
        let (dir, path) = temp_store();
        save_theme(&path, ThemeId::CatppuccinLatte).unwrap();
        assert_eq!(load_ui_state(&path).theme, Some(ThemeId::CatppuccinLatte));
        fs::write(&path, "{\"version\":1,\"theme\":\"solarized-light\"}\n").unwrap();
        assert_eq!(load_ui_state(&path).theme, Some(ThemeId::SolarizedLight));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        for save in [
            (|p: &Path| save_theme(p, ThemeId::Nord)) as Save,
            |p: &Path| save_activity_sparkline(p, true),
            |p: &Path| save_commit_msg_lines(p, 6),
        ] {
            let (dir, path) = temp_store();
            let path = path
                .parent()
                .unwrap()
                .join("a")
                .join("b")
                .join(STORE_FILE_NAME);
            assert!(!dir.exists());
            save(&path).unwrap();
            assert_ne!(load_ui_state(&path), UiState::default());
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn load_falls_through_to_the_default() {
        let (dir, path) = temp_store();
        assert_eq!(load_ui_state(&path), UiState::default(), "missing file");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for body in [
            "{not json",
            "",
            "[1, 2]\n",
            "[1, \"dracula\", true]\n",
            "{\"theme\":\"dracula\"}\n",
            "{\"version\":1}\n",
            "{\"version\":1,\"theme\":\"solarized\"}\n",
            "{\"version\":1,\"theme\":\"Dracula\"}\n",
            "{\"version\":2,\"theme\":\"dracula\",\"activitySparkline\":true}\n",
            "{\"version\":0,\"theme\":\"dracula\",\"activitySparkline\":true}\n",
            "{\"version\":1,\"theme\":\"dracula\",\"activitySparkline\":true,\"ignoredRepos\":[]}\n",
            "{\"version\":1,\"activitySparkline\":\"yes\"}\n",
        ] {
            fs::write(&path, body).unwrap();
            assert_eq!(load_ui_state(&path), UiState::default(), "{body:?}");
        }
        fs::write(&path, "{\"version\":1,\"theme\":\"dracula\"}").unwrap();
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Dracula),
                activity_sparkline: false,
                commit_msg_lines: None,
            },
            "compact, missing activitySparkline is off"
        );
        fs::write(&path, "{\"version\":1,\"activitySparkline\":true}").unwrap();
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: None,
                activity_sparkline: true,
                commit_msg_lines: None,
            },
            "no theme, sparkline on"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_never_overwrites_a_file_that_is_not_the_store() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let saves: [(&str, Save); 3] = [
            ("theme", |p| save_theme(p, ThemeId::Monokai)),
            ("sparkline", |p| save_activity_sparkline(p, true)),
            ("commit message lines", |p| save_commit_msg_lines(p, 6)),
        ];
        for (name, save) in saves {
            for body in [
                "{\"theme\":\"dracula\"}\n",
                "{\"ignoredRepos\":[],\"theme\":\"monokai\",\"watchMs\":0}\n",
                "export PATH=$HOME/bin:$PATH\n",
                "",
                "[1, 2]\n",
                "{\"version\":\"1\",\"theme\":\"dracula\"}\n",
                "{\"version\":1,\"theme\":\"dracula\",\"ignoredRepos\":[]}\n",
                "{\"version\":1,\"activitySparkline\":\"yes\"}\n",
            ] {
                fs::write(&path, body).unwrap();
                let err = save(&path).unwrap_err();
                assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{name} {body:?}");
                assert_eq!(
                    fs::read_to_string(&path).unwrap(),
                    body,
                    "{name}: byte-identical"
                );
                assert!(!path.with_extension("json.tmp").exists(), "{name}");
            }
            let bytes = [0xff_u8, 0xfe, 0x00];
            fs::write(&path, bytes).unwrap();
            let err = save(&path).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{name}");
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "{name}: non-UTF-8 left as is"
            );
        }
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
            assert_eq!(load_ui_state(&path).theme, None, "{body:?}");
            save_theme(&path, ThemeId::Kanagawa).unwrap();
            assert_eq!(
                load_ui_state(&path).theme,
                Some(ThemeId::Kanagawa),
                "{body:?}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// A drag's save writes `commitMessageLines` and keeps the theme (an
    /// unknown slug too) and the sparkline; a theme or sparkline save keeps
    /// the saved rows.
    #[test]
    fn commit_msg_lines_round_trip_and_keep_the_other_keys() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "{\"version\":1,\"theme\":\"dracula\",\"activitySparkline\":true}\n",
        )
        .unwrap();
        save_commit_msg_lines(&path, 7).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"dracula\",\n  \"activitySparkline\": true,\n  \"commitMessageLines\": 7\n}\n"
        );
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Dracula),
                activity_sparkline: true,
                commit_msg_lines: Some(7),
            }
        );
        save_theme(&path, ThemeId::Nord).unwrap();
        save_activity_sparkline(&path, false).unwrap();
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Nord),
                activity_sparkline: false,
                commit_msg_lines: Some(7),
            },
            "theme and sparkline saves keep the rows"
        );

        fs::write(&path, "{\"version\":1,\"theme\":\"solarized\"}\n").unwrap();
        save_commit_msg_lines(&path, 3).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"solarized\",\n  \"activitySparkline\": false,\n  \"commitMessageLines\": 3\n}\n",
            "unknown slug kept as is"
        );

        for (lines, saved) in [(1, 1), (20, 20), (0, 1), (99, 20)] {
            save_commit_msg_lines(&path, lines).unwrap();
            assert_eq!(
                load_ui_state(&path).commit_msg_lines,
                Some(saved),
                "{lines}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// A `commitMessageLines` that is not an integer in 1..=20 loads as not
    /// saved; the theme and sparkline still load, and a save of another key
    /// keeps the bad value as is.
    #[test]
    fn bad_commit_msg_lines_is_unset_and_the_rest_still_loads() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bad in ["0", "21", "-1", "4.5", "\"6\"", "null", "true", "[6]", "{}"] {
            let body = format!(
                "{{\"version\":1,\"theme\":\"nord\",\"activitySparkline\":true,\"commitMessageLines\":{bad}}}\n"
            );
            fs::write(&path, &body).unwrap();
            assert_eq!(
                load_ui_state(&path),
                UiState {
                    theme: Some(ThemeId::Nord),
                    activity_sparkline: true,
                    commit_msg_lines: None,
                },
                "{bad}"
            );
            assert_eq!(check_ui_state_bytes(body.as_bytes()), Ok(()), "{bad}");
            save_activity_sparkline(&path, false).unwrap();
            let kept = fs::read_to_string(&path).unwrap();
            if bad == "null" {
                assert!(!kept.contains("commitMessageLines"), "{kept}");
            } else {
                let compact: String = kept.chars().filter(|c| !c.is_whitespace()).collect();
                assert!(
                    compact.contains(&format!("\"commitMessageLines\":{bad}")),
                    "{bad}: kept as is: {kept}"
                );
            }
            save_commit_msg_lines(&path, 5).unwrap();
            assert_eq!(load_ui_state(&path).commit_msg_lines, Some(5), "{bad}");
        }
        let _ = fs::remove_dir_all(dir);
    }
}
