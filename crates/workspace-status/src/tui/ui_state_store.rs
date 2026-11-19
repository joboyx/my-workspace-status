//! Persist TUI view state (last-used theme, activity sparkline, commit
//! message rows, ignore whitespace, tree focus per workspace) in a state
//! file.
//!
//! The theme that a palette `Theme: …` row sets, the activity sparkline
//! toggle, the commit-message footer height a mouse drag sets, the
//! last "ignore whitespace" diff setting, and the tree focus query of each
//! workspace root are state, not config: they live in one JSON file
//! (`$XDG_STATE_HOME/my-workspace-status/ui-state.json`) and the user config
//! file is never written. [`crate::settings::Settings`] loads it at launch;
//! see [`super::theme::theme_id_from`] for the theme precedence. The tree
//! focus has its own load and save ([`load_tree_focus`],
//! [`save_tree_focus`]), keyed by [`super::viewed::workspace_store_id`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use workspace_status_graph::{COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN};

use super::persist::persist_with_lock;
use super::theme::{parse_theme_id, ThemeId};
use super::tree_focus::TreeFocus;

/// On-disk store version written by save. Load accepts only this version.
pub const UI_STATE_STORE_VERSION: u32 = 1;

/// File name under the state directory.
const STORE_FILE_NAME: &str = "ui-state.json";

/// On-disk shape:
/// `{"version": 1, "theme": "<slug>", "activitySparkline": false,
/// "commitMessageLines": 6, "ignoreWhitespace": true,
/// "treeFocus": {"<workspace store id>": "<query>"}}`.
///
/// `version` is required; `theme`, `activitySparkline`,
/// `commitMessageLines`, `ignoreWhitespace`, and `treeFocus` are optional.
/// An object with any other key (for example a config file that also has
/// `version` and `theme`) is not a store: load ignores it and save leaves it
/// unchanged. Save keeps the raw `theme` string and the raw
/// `commitMessageLines`, `ignoreWhitespace`, and `treeFocus` JSON values, so
/// an unknown slug or a bad value survives a save of another key.
/// `commitMessageLines`, `ignoreWhitespace`, and `treeFocus` take any JSON
/// value here: a bad value is only that key unset
/// ([`saved_commit_msg_lines`], [`saved_ignore_whitespace`],
/// [`saved_tree_focus`]), never the whole file foreign. `treeFocus` maps a
/// workspace store id to its raw focus query; a bad entry is only that
/// workspace unset.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ignore_whitespace: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tree_focus: Option<serde_json::Value>,
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
    /// Last-used "ignore whitespace" diff setting: `Some(true)` loads diffs
    /// with `git diff -w`, `Some(false)` shows whitespace changes. `None`
    /// when none was saved or the saved value is not a JSON bool.
    pub ignore_whitespace: Option<bool>,
}

/// The saved `ignoreWhitespace` when it is a JSON bool; any other value
/// reads as not saved.
fn saved_ignore_whitespace(value: Option<&serde_json::Value>) -> Option<bool> {
    value?.as_bool()
}

/// The saved focus query of `workspace_id` when `treeFocus` is a JSON
/// object and its entry for that id is a JSON string; any other value reads
/// as not saved.
fn saved_tree_focus(value: Option<&serde_json::Value>, workspace_id: &str) -> Option<String> {
    let query = value?.as_object()?.get(workspace_id)?.as_str()?;
    Some(query.to_string())
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
/// integer in [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]) or
/// `ignoreWhitespace` (not a bool) loads as not saved and keeps the other
/// keys.
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
        ignore_whitespace: saved_ignore_whitespace(parsed.ignore_whitespace.as_ref()),
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

/// Persist the "ignore whitespace" diff setting as the last-used value, and
/// keep the other stored keys as they are (an unknown theme slug or a bad
/// `commitMessageLines` included).
///
/// The user config file (`viewDefaults.whitespace`) is never written. See
/// [`update_store`] for the create, foreign-file, and lock rules.
pub fn save_ignore_whitespace(path: &Path, on: bool) -> io::Result<()> {
    update_store(path, |file| {
        file.ignore_whitespace = Some(serde_json::Value::Bool(on));
    })
}

/// The tree focus query saved for the workspace root `workspace_id` (a
/// [`super::viewed::workspace_store_id`]).
///
/// `None` when the file is missing, unreadable, foreign, or has another
/// version (the [`load_ui_state`] rules), when `treeFocus` is not a JSON
/// object, when the workspace has no entry, when its entry is not a JSON
/// string, or when the query has no terms ([`TreeFocus::is_empty`]). A bad
/// value never changes what the other keys load.
pub fn load_tree_focus(path: &Path, workspace_id: &str) -> Option<String> {
    let parsed = fs::read(path).ok().and_then(|bytes| parse_store(&bytes))?;
    if parsed.version != UI_STATE_STORE_VERSION {
        return None;
    }
    saved_tree_focus(parsed.tree_focus.as_ref(), workspace_id)
        .filter(|query| !TreeFocus::parse(query).is_empty())
}

/// Persist the tree focus `query` of the workspace root `workspace_id` (a
/// [`super::viewed::workspace_store_id`]), and keep the other stored keys
/// and the entries of other workspaces as they are.
///
/// `None`, or a query with no terms ([`TreeFocus::is_empty`]), removes the
/// workspace's entry; when that leaves no entry, the `treeFocus` key is
/// dropped from the file. A `treeFocus` value that is not a JSON object is
/// replaced by a new object on a save of a query, and left as is on a
/// removal. The user config file is never written. See [`update_store`]
/// for the create, foreign-file, and lock rules.
pub fn save_tree_focus(path: &Path, workspace_id: &str, query: Option<&str>) -> io::Result<()> {
    let query = query.filter(|q| !TreeFocus::parse(q).is_empty());
    update_store(path, |file| match query {
        Some(query) => {
            let mut entries = match file.tree_focus.take() {
                Some(serde_json::Value::Object(entries)) => entries,
                _ => serde_json::Map::new(),
            };
            entries.insert(workspace_id.to_string(), query.into());
            file.tree_focus = Some(serde_json::Value::Object(entries));
        }
        None => {
            if let Some(serde_json::Value::Object(entries)) = file.tree_focus.as_mut() {
                entries.remove(workspace_id);
                if entries.is_empty() {
                    file.tree_focus = None;
                }
            }
        }
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
                    ignore_whitespace: None,
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
                ignore_whitespace: None,
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
                ignore_whitespace: None,
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
                ignore_whitespace: None,
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
                ignore_whitespace: None,
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
            |p: &Path| save_ignore_whitespace(p, true),
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
                ignore_whitespace: None,
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
                ignore_whitespace: None,
            },
            "no theme, sparkline on"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_never_overwrites_a_file_that_is_not_the_store() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let saves: [(&str, Save); 6] = [
            ("theme", |p| save_theme(p, ThemeId::Monokai)),
            ("sparkline", |p| save_activity_sparkline(p, true)),
            ("commit message lines", |p| save_commit_msg_lines(p, 6)),
            ("ignore whitespace", |p| save_ignore_whitespace(p, true)),
            ("tree focus", |p| {
                save_tree_focus(p, "ws-a", Some("ABC-123"))
            }),
            ("tree focus clear", |p| save_tree_focus(p, "ws-a", None)),
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
                ignore_whitespace: None,
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
                ignore_whitespace: None,
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
                    ignore_whitespace: None,
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

    /// An `ignoreWhitespace` save round-trips both values and keeps the
    /// theme (an unknown slug too), the sparkline, and the rows; the other
    /// saves keep it; an absent key loads as not saved.
    #[test]
    fn ignore_whitespace_round_trips_and_keeps_the_other_keys() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "{\"version\":1,\"theme\":\"dracula\",\"activitySparkline\":true,\"commitMessageLines\":7}\n",
        )
        .unwrap();
        assert_eq!(load_ui_state(&path).ignore_whitespace, None, "absent key");
        save_ignore_whitespace(&path, true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"dracula\",\n  \"activitySparkline\": true,\n  \"commitMessageLines\": 7,\n  \"ignoreWhitespace\": true\n}\n"
        );
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Dracula),
                activity_sparkline: true,
                commit_msg_lines: Some(7),
                ignore_whitespace: Some(true),
            }
        );
        save_ignore_whitespace(&path, false).unwrap();
        assert_eq!(load_ui_state(&path).ignore_whitespace, Some(false));

        save_theme(&path, ThemeId::Nord).unwrap();
        save_activity_sparkline(&path, false).unwrap();
        save_commit_msg_lines(&path, 3).unwrap();
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Nord),
                activity_sparkline: false,
                commit_msg_lines: Some(3),
                ignore_whitespace: Some(false),
            },
            "the other saves keep the setting"
        );

        fs::write(&path, "{\"version\":1,\"theme\":\"solarized\"}\n").unwrap();
        save_ignore_whitespace(&path, true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"solarized\",\n  \"activitySparkline\": false,\n  \"ignoreWhitespace\": true\n}\n",
            "unknown slug kept as is"
        );

        let (fresh_dir, fresh) = temp_store();
        save_ignore_whitespace(&fresh, true).unwrap();
        assert_eq!(
            fs::read_to_string(&fresh).unwrap(),
            "{\n  \"version\": 1,\n  \"activitySparkline\": false,\n  \"ignoreWhitespace\": true\n}\n",
            "a missing file gets no theme"
        );
        let _ = fs::remove_dir_all(fresh_dir);
        let _ = fs::remove_dir_all(dir);
    }

    /// An `ignoreWhitespace` that is not a JSON bool loads as not saved;
    /// the other keys still load, and a save of another key keeps the bad
    /// value as is (the `commitMessageLines` rule).
    #[test]
    fn bad_ignore_whitespace_is_unset_and_the_rest_still_loads() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bad in ["\"true\"", "1", "0", "null", "[true]", "{}", "\"ignore\""] {
            let body = format!(
                "{{\"version\":1,\"theme\":\"nord\",\"activitySparkline\":true,\"commitMessageLines\":4,\"ignoreWhitespace\":{bad}}}\n"
            );
            fs::write(&path, &body).unwrap();
            assert_eq!(
                load_ui_state(&path),
                UiState {
                    theme: Some(ThemeId::Nord),
                    activity_sparkline: true,
                    commit_msg_lines: Some(4),
                    ignore_whitespace: None,
                },
                "{bad}"
            );
            assert_eq!(check_ui_state_bytes(body.as_bytes()), Ok(()), "{bad}");
            save_activity_sparkline(&path, false).unwrap();
            let kept = fs::read_to_string(&path).unwrap();
            if bad == "null" {
                assert!(!kept.contains("ignoreWhitespace"), "{kept}");
            } else {
                let compact: String = kept.chars().filter(|c| !c.is_whitespace()).collect();
                assert!(
                    compact.contains(&format!("\"ignoreWhitespace\":{bad}")),
                    "{bad}: kept as is: {kept}"
                );
            }
            save_ignore_whitespace(&path, true).unwrap();
            assert_eq!(load_ui_state(&path).ignore_whitespace, Some(true), "{bad}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// The focus of each workspace root is its own entry: two roots share
    /// one file, a clear of one keeps the other, the last clear drops the
    /// `treeFocus` key, and the theme and other keys survive each save.
    #[test]
    fn tree_focus_is_saved_per_workspace_root_and_keeps_the_other_keys() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "{\"version\":1,\"theme\":\"solarized\",\"activitySparkline\":true,\"commitMessageLines\":7}\n",
        )
        .unwrap();
        assert_eq!(load_tree_focus(&path, "ws-a"), None, "absent key");

        save_tree_focus(&path, "ws-a", Some("ABC-123")).unwrap();
        save_tree_focus(&path, "ws-b", Some("app, lib")).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"solarized\",\n  \"activitySparkline\": true,\n  \"commitMessageLines\": 7,\n  \"treeFocus\": {\n    \"ws-a\": \"ABC-123\",\n    \"ws-b\": \"app, lib\"\n  }\n}\n",
            "unknown slug kept as is; raw query kept"
        );
        assert_eq!(load_tree_focus(&path, "ws-a").as_deref(), Some("ABC-123"));
        assert_eq!(load_tree_focus(&path, "ws-b").as_deref(), Some("app, lib"));
        assert_eq!(load_tree_focus(&path, "ws-c"), None);

        save_theme(&path, ThemeId::Nord).unwrap();
        save_ignore_whitespace(&path, true).unwrap();
        assert_eq!(
            load_tree_focus(&path, "ws-a").as_deref(),
            Some("ABC-123"),
            "other saves keep the focus"
        );
        assert_eq!(
            load_ui_state(&path),
            UiState {
                theme: Some(ThemeId::Nord),
                activity_sparkline: true,
                commit_msg_lines: Some(7),
                ignore_whitespace: Some(true),
            },
            "a focus key does not change the view state"
        );

        save_tree_focus(&path, "ws-a", Some("billing")).unwrap();
        assert_eq!(load_tree_focus(&path, "ws-a").as_deref(), Some("billing"));

        save_tree_focus(&path, "ws-a", None).unwrap();
        assert_eq!(load_tree_focus(&path, "ws-a"), None, "cleared");
        assert_eq!(
            load_tree_focus(&path, "ws-b").as_deref(),
            Some("app, lib"),
            "the other root keeps its focus"
        );

        save_tree_focus(&path, "ws-b", Some(" , ")).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"nord\",\n  \"activitySparkline\": true,\n  \"commitMessageLines\": 7,\n  \"ignoreWhitespace\": true\n}\n",
            "a query with no terms clears; the last clear drops the key"
        );
        save_tree_focus(&path, "ws-b", None).unwrap();
        assert_eq!(load_tree_focus(&path, "ws-b"), None, "clear of no entry");

        let (fresh_dir, fresh) = temp_store();
        save_tree_focus(&fresh, "ws-a", Some("ABC-123")).unwrap();
        assert_eq!(
            fs::read_to_string(&fresh).unwrap(),
            "{\n  \"version\": 1,\n  \"activitySparkline\": false,\n  \"treeFocus\": {\n    \"ws-a\": \"ABC-123\"\n  }\n}\n",
            "a missing file (and its parent dirs) gets only the focus"
        );
        let _ = fs::remove_dir_all(fresh_dir);
        let _ = fs::remove_dir_all(dir);
    }

    /// A `treeFocus` that is not an object, or an entry that is not a
    /// string, loads as no focus; the other keys still load, a save of
    /// another key keeps the bad value as is, a focus save replaces it, and
    /// a clear leaves it.
    #[test]
    fn bad_tree_focus_is_no_focus_and_the_rest_still_loads() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bad in [
            "\"ABC-123\"",
            "1",
            "true",
            "null",
            "[\"ws-a\"]",
            "{\"ws-a\":1}",
            "{\"ws-a\":null}",
            "{\"ws-a\":[\"ABC-123\"]}",
            "{\"ws-a\":\" , \"}",
        ] {
            let body = format!(
                "{{\"version\":1,\"theme\":\"nord\",\"activitySparkline\":true,\"treeFocus\":{bad}}}\n"
            );
            fs::write(&path, &body).unwrap();
            assert_eq!(load_tree_focus(&path, "ws-a"), None, "{bad}");
            assert_eq!(
                load_ui_state(&path),
                UiState {
                    theme: Some(ThemeId::Nord),
                    activity_sparkline: true,
                    commit_msg_lines: None,
                    ignore_whitespace: None,
                },
                "{bad}"
            );
            assert_eq!(check_ui_state_bytes(body.as_bytes()), Ok(()), "{bad}");

            save_activity_sparkline(&path, false).unwrap();
            save_tree_focus(&path, "ws-a", None).unwrap();
            let kept = fs::read_to_string(&path).unwrap();
            let compact: String = kept.chars().filter(|c| !c.is_whitespace()).collect();
            if bad == "null" {
                assert!(!kept.contains("treeFocus"), "{kept}");
            } else if bad.starts_with('{') {
                assert!(
                    !kept.contains("treeFocus"),
                    "{bad}: the clear drops it: {kept}"
                );
            } else {
                let bad_compact: String = bad.chars().filter(|c| !c.is_whitespace()).collect();
                assert!(
                    compact.contains(&format!("\"treeFocus\":{bad_compact}")),
                    "{bad}: kept as is: {kept}"
                );
            }

            save_tree_focus(&path, "ws-a", Some("ABC-123")).unwrap();
            assert_eq!(
                load_tree_focus(&path, "ws-a").as_deref(),
                Some("ABC-123"),
                "{bad}"
            );
        }

        fs::write(
            &path,
            "{\"version\":1,\"treeFocus\":{\"ws-a\":7,\"ws-b\":\"lib\"}}\n",
        )
        .unwrap();
        assert_eq!(load_tree_focus(&path, "ws-a"), None);
        assert_eq!(
            load_tree_focus(&path, "ws-b").as_deref(),
            Some("lib"),
            "a bad entry is only that workspace unset"
        );
        save_tree_focus(&path, "ws-b", None).unwrap();
        let compact: String = fs::read_to_string(&path)
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(compact.contains("\"treeFocus\":{\"ws-a\":7}"), "{compact}");

        for body in [
            "{\"version\":2,\"treeFocus\":{\"ws-a\":\"ABC-123\"}}\n",
            "{\"version\":1,\"treeFocus\":{\"ws-a\":\"ABC-123\"},\"ignoredRepos\":[]}\n",
            "{not json",
        ] {
            fs::write(&path, body).unwrap();
            assert_eq!(load_tree_focus(&path, "ws-a"), None, "{body:?}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    /// A focus save never writes the user config file: a config path that
    /// is not a store is left byte-identical.
    #[test]
    fn tree_focus_save_leaves_a_config_file_untouched() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let config = "{\"theme\":\"tokyo-night\",\"hiddenPaths\":[\"vendor\"]}\n";
        fs::write(&path, config).unwrap();
        for query in [Some("ABC-123"), None] {
            let err = save_tree_focus(&path, "ws-a", query).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{query:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), config, "{query:?}");
        }
        assert_eq!(load_tree_focus(&path, "ws-a"), None);
        let _ = fs::remove_dir_all(dir);
    }
}
