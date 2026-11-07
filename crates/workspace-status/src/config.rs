//! Workspace-status config: the user config file
//! (`$XDG_CONFIG_HOME/my-workspace-status/config.json`) under the workspace
//! file (`.workspace-status-config.json`). Both files use the same schema.
//!
//! The keys in [`RuntimeKeys`] also have an env var. [`crate::settings`]
//! resolves them once at startup (env var > config file > default).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::helpers::normalize_filter_repo;
use crate::tui::theme::{BackgroundMode, ThemeId, THEME_IDS};
use serde::Deserialize;
use workspace_status_graph::{COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN};

pub const CONFIG_FILENAME: &str = ".workspace-status-config.json";
pub const DEFAULT_MAX_DEPTH: u32 = 3;
/// Directory under `$XDG_CONFIG_HOME` (or `$HOME/.config`) that holds the user config file.
pub const USER_CONFIG_DIR: &str = "my-workspace-status";
/// File name of the user config file inside [`USER_CONFIG_DIR`].
pub const USER_CONFIG_FILENAME: &str = "config.json";

#[derive(Debug, Clone, Default)]
pub struct WorkspaceStatusConfig {
    /// `hiddenPaths`: workspace-relative paths whose repos (at or under the
    /// path) are discovered but hidden until `.` in the TUI or `-a` on the
    /// CLI. Callers that show hidden repos clear this list before discovery.
    pub hidden_paths: Vec<String>,
    /// `ignoredPaths`: workspace-relative paths whose repos (at or under the
    /// path) are never discovered, unless a repo is named on the command line.
    pub ignored_paths: Vec<String>,
    pub max_depth: u32,
    pub default_branches: BTreeMap<String, String>,
    pub editor: Option<String>,
    /// External diff command (`diffTool`). Blank/omit means default `vimdiff` at resolve time.
    pub diff_tool: Option<String>,
    /// TUI launch view modes (`viewDefaults`). Omitted keys keep the in-app defaults.
    pub view_defaults: ViewDefaults,
    /// Top-level keys that an env var can override (`theme`, `glyphs`,
    /// `watchMs`, ...). See [`RuntimeKeys`].
    pub runtime: RuntimeKeys,
}

/// Glyph set from the `glyphs` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSet {
    /// `"nerd"`: Nerd Font glyphs (the default).
    Nerd,
    /// `"ascii"`: one-column ASCII markers.
    Ascii,
}

/// Light or dark theme pick from the `themeMode` key.
///
/// The pick swaps the resolved theme for its twin of the other polarity
/// ([`ThemeId::twin`]). The store still saves the slug last applied from a
/// palette `Theme: …` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// `"auto"` (the default): ask the terminal for its background colour at
    /// startup and pick the light or dark variant to match. A terminal that
    /// does not answer keeps the resolved theme.
    Auto,
    /// `"dark"`: always the dark variant of the resolved theme.
    Dark,
    /// `"light"`: always the light variant of the resolved theme.
    Light,
}

/// Config keys that an env var can also set.
///
/// Each field is `None` when no file sets the key. [`crate::settings::Settings`]
/// resolves each one: a valid env var wins, then this value, then the
/// built-in default. Path values are already expanded (`~/` with `HOME`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeKeys {
    /// `theme` (`WS_STATUS_THEME`): a built-in theme id.
    pub theme: Option<ThemeId>,
    /// `themeMode` (`WS_STATUS_THEME_MODE`): `"auto"`, `"dark"`, or `"light"`.
    pub theme_mode: Option<ThemeMode>,
    /// `glyphs` (`WS_STATUS_GLYPHS`): `"nerd"` or `"ascii"`.
    pub glyphs: Option<GlyphSet>,
    /// `watchMs` (`WS_STATUS_WATCH_MS`): live-refresh poll period, `0` turns it off.
    pub watch_ms: Option<u64>,
    /// `fetchMs` (`WS_STATUS_FETCH_MS`): background fetch period, `0` turns it off.
    pub fetch_ms: Option<u64>,
    /// `fetchConcurrency` (`WS_STATUS_FETCH_CONCURRENCY`): in-flight cap, at least 1.
    pub fetch_concurrency: Option<usize>,
    /// `updateCheck` (`WS_STATUS_UPDATE_CHECK`): TUI-startup release check on or off.
    pub update_check: Option<bool>,
    /// `updateCheckStore` (`WS_STATUS_UPDATE_CHECK_STORE`): last-check JSON file.
    pub update_check_store: Option<PathBuf>,
    /// `commentStore` (`WS_STATUS_COMMENT_STORE`): TUI comment JSON file.
    pub comment_store: Option<PathBuf>,
    /// `viewedStore` (`WS_STATUS_VIEWED_STORE`): TUI viewed-marks JSON file.
    pub viewed_store: Option<PathBuf>,
    /// `uiStore` (`WS_STATUS_UI_STORE`): TUI view-state JSON file (last-used
    /// theme, activity sparkline).
    pub ui_store: Option<PathBuf>,
    /// `git` (`WORKSPACE_STATUS_GIT`): git binary path or command name.
    pub git: Option<PathBuf>,
    /// `termProgress` (`WS_STATUS_TERM_PROGRESS`): OSC 9;4 terminal progress
    /// for multi-repo batches on or off.
    pub term_progress: Option<bool>,
    /// `smoothScroll` (`WS_STATUS_SMOOTH_SCROLL`): Ctrl-d / Ctrl-u glide the
    /// viewport (on) or jump (off).
    pub smooth_scroll: Option<bool>,
    /// `prPolling`: PR polling on or off. `WS_STATUS_PR_POLL_MS` (a period,
    /// `0` off) wins over it.
    pub pr_polling: Option<bool>,
    /// `prTitleCommand` (`WS_STATUS_PR_TITLE_CMD`): program that names a
    /// ticket key for the PRs tab group titles.
    pub pr_title_command: Option<PathBuf>,
}

impl RuntimeKeys {
    /// Per-key merge: each key that `over` sets wins; the others keep `self`.
    fn overlay(self, over: RuntimeKeys) -> RuntimeKeys {
        RuntimeKeys {
            theme: over.theme.or(self.theme),
            theme_mode: over.theme_mode.or(self.theme_mode),
            glyphs: over.glyphs.or(self.glyphs),
            watch_ms: over.watch_ms.or(self.watch_ms),
            fetch_ms: over.fetch_ms.or(self.fetch_ms),
            fetch_concurrency: over.fetch_concurrency.or(self.fetch_concurrency),
            update_check: over.update_check.or(self.update_check),
            update_check_store: over.update_check_store.or(self.update_check_store),
            comment_store: over.comment_store.or(self.comment_store),
            viewed_store: over.viewed_store.or(self.viewed_store),
            ui_store: over.ui_store.or(self.ui_store),
            git: over.git.or(self.git),
            term_progress: over.term_progress.or(self.term_progress),
            smooth_scroll: over.smooth_scroll.or(self.smooth_scroll),
            pr_polling: over.pr_polling.or(self.pr_polling),
            pr_title_command: over.pr_title_command.or(self.pr_title_command),
        }
    }
}

/// TUI launch view modes from `viewDefaults`.
///
/// Each field is `None` when its key is omitted, so the in-app default
/// (set where the TUI state is built) applies. Session toggles never write
/// these back to the config file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewDefaults {
    /// `tree`: `Some(true)` for `"tree"`, `Some(false)` for `"flat"` (workspace tree at depth 0).
    pub tree: Option<bool>,
    /// `commitTree`: `Some(true)` for `"tree"`, `Some(false)` for `"flat"` (commit file list at depth >= 1).
    pub commit_tree: Option<bool>,
    /// `diff`: `Some(true)` for `"split"` (side-by-side), `Some(false)` for `"inline"`.
    pub diff_split: Option<bool>,
    /// `wrap`: `Some(true)` for `"wrap"`, `Some(false)` for `"unwrap"`.
    pub wrap: Option<bool>,
    /// `commitMessage`: `Some(true)` for `"expand"`, `Some(false)` for `"collapse"`.
    pub commit_message_expand: Option<bool>,
    /// `lineBlame`: `Some(true)` for `"show"`, `Some(false)` for `"hide"`.
    pub line_blame: Option<bool>,
    /// `commitMessageLines`: message rows of the expanded commit-message
    /// footer, an integer from [`COMMIT_MSG_LINES_MIN`] to
    /// [`COMMIT_MSG_LINES_MAX`].
    pub commit_message_lines: Option<usize>,
    /// `background`: [`BackgroundMode::Paint`] for `"paint"`,
    /// [`BackgroundMode::Terminal`] for `"terminal"`.
    pub background: Option<BackgroundMode>,
}

impl WorkspaceStatusConfig {
    pub fn with_defaults() -> Self {
        Self {
            hidden_paths: Vec::new(),
            ignored_paths: Vec::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            default_branches: BTreeMap::new(),
            editor: None,
            diff_tool: None,
            view_defaults: ViewDefaults::default(),
            runtime: RuntimeKeys::default(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    /// Removed key. Kept only to fail with a pointer to the new keys; `null`
    /// counts as set.
    #[serde(default, deserialize_with = "deserialize_present")]
    ignored_repos: Option<serde_json::Value>,
    hidden_paths: Option<serde_json::Value>,
    ignored_paths: Option<serde_json::Value>,
    max_depth: Option<serde_json::Value>,
    default_branches: Option<serde_json::Value>,
    editor: Option<serde_json::Value>,
    diff_tool: Option<serde_json::Value>,
    view_defaults: Option<serde_json::Value>,
    theme: Option<serde_json::Value>,
    theme_mode: Option<serde_json::Value>,
    glyphs: Option<serde_json::Value>,
    watch_ms: Option<serde_json::Value>,
    fetch_ms: Option<serde_json::Value>,
    fetch_concurrency: Option<serde_json::Value>,
    update_check: Option<serde_json::Value>,
    update_check_store: Option<serde_json::Value>,
    comment_store: Option<serde_json::Value>,
    viewed_store: Option<serde_json::Value>,
    ui_store: Option<serde_json::Value>,
    git: Option<serde_json::Value>,
    term_progress: Option<serde_json::Value>,
    smooth_scroll: Option<serde_json::Value>,
    pr_polling: Option<serde_json::Value>,
    pr_title_command: Option<serde_json::Value>,
}

/// Deserialize a key that is present, `null` included, as `Some`.
fn deserialize_present<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

/// True when a path-list entry is not relative to the workspace root:
/// it starts with `/`, `\`, `~`, or a Windows drive (`C:`), or it has a
/// `..` segment.
fn path_entry_is_invalid(raw: &str) -> bool {
    let unified = raw.replace('\\', "/");
    let mut chars = unified.chars();
    let drive = matches!(
        (chars.next(), chars.next()),
        (Some(c), Some(':')) if c.is_ascii_alphabetic()
    );
    unified.starts_with('/')
        || unified.starts_with('~')
        || drive
        || unified.split('/').any(|segment| segment == "..")
}

/// Normalize path-list entries: trim, strip `./`, unify separators, drop
/// trailing `/`, drop blanks, then sort and dedupe.
fn normalize_path_list(entries: &[String]) -> Vec<String> {
    let mut out: Vec<String> = entries
        .iter()
        .map(|r| normalize_filter_repo(r.trim()))
        .filter(|r| !r.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn normalize_default_branches(map: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (raw_repo, raw_branch) in map {
        let repo = normalize_filter_repo(raw_repo.trim());
        let branch = raw_branch.trim();
        if repo.is_empty() || branch.is_empty() {
            continue;
        }
        out.insert(repo, branch.to_string());
    }
    out
}

pub fn default_branch_override_for(
    repo_path: &str,
    default_branches: &BTreeMap<String, String>,
) -> Option<String> {
    let normalized = normalize_filter_repo(repo_path);
    default_branches
        .get(&normalized)
        .filter(|b| !b.is_empty())
        .cloned()
}

/// Parse one two-value `viewDefaults` key: `Some(true)` for `on`, `Some(false)` for `off`.
///
/// `file` labels the config file in the error.
fn parse_view_choice(
    file: &str,
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    on: &str,
    off: &str,
) -> Result<Option<bool>, String> {
    let Some(v) = obj.get(key) else {
        return Ok(None);
    };
    match v.as_str().map(str::trim) {
        Some(s) if s == on => Ok(Some(true)),
        Some(s) if s == off => Ok(Some(false)),
        _ => Err(format!(
            "{file} viewDefaults.{key} must be \"{on}\" or \"{off}\""
        )),
    }
}

/// Parse `viewDefaults.commitMessageLines`: a JSON integer from
/// [`COMMIT_MSG_LINES_MIN`] to [`COMMIT_MSG_LINES_MAX`]. Omitted is `None`.
fn parse_view_msg_lines(
    file: &str,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<usize>, String> {
    const KEY: &str = "commitMessageLines";
    let Some(v) = obj.get(KEY) else {
        return Ok(None);
    };
    match v.as_u64() {
        Some(n) if (COMMIT_MSG_LINES_MIN as u64..=COMMIT_MSG_LINES_MAX as u64).contains(&n) => {
            Ok(Some(n as usize))
        }
        _ => Err(format!(
            "{file} viewDefaults.{KEY} must be an integer from {COMMIT_MSG_LINES_MIN} to {COMMIT_MSG_LINES_MAX}"
        )),
    }
}

fn parse_view_defaults(
    file: &str,
    value: Option<serde_json::Value>,
) -> Result<ViewDefaults, String> {
    let Some(v) = value else {
        return Ok(ViewDefaults::default());
    };
    let Some(obj) = v.as_object() else {
        return Err(format!("{file} viewDefaults must be an object"));
    };
    const KEYS: [&str; 8] = [
        "tree",
        "commitTree",
        "diff",
        "wrap",
        "commitMessage",
        "commitMessageLines",
        "lineBlame",
        "background",
    ];
    if let Some(key) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("{file} viewDefaults has unknown key \"{key}\""));
    }
    Ok(ViewDefaults {
        tree: parse_view_choice(file, obj, "tree", "tree", "flat")?,
        commit_tree: parse_view_choice(file, obj, "commitTree", "tree", "flat")?,
        diff_split: parse_view_choice(file, obj, "diff", "split", "inline")?,
        wrap: parse_view_choice(file, obj, "wrap", "wrap", "unwrap")?,
        commit_message_expand: parse_view_choice(file, obj, "commitMessage", "expand", "collapse")?,
        line_blame: parse_view_choice(file, obj, "lineBlame", "show", "hide")?,
        commit_message_lines: parse_view_msg_lines(file, obj)?,
        background: parse_view_choice(file, obj, "background", "paint", "terminal")?.map(|paint| {
            if paint {
                BackgroundMode::Paint
            } else {
                BackgroundMode::Terminal
            }
        }),
    })
}

impl ViewDefaults {
    /// Per-key merge: each key that `over` sets wins; the others keep `self`.
    fn overlay(self, over: ViewDefaults) -> ViewDefaults {
        ViewDefaults {
            tree: over.tree.or(self.tree),
            commit_tree: over.commit_tree.or(self.commit_tree),
            diff_split: over.diff_split.or(self.diff_split),
            wrap: over.wrap.or(self.wrap),
            commit_message_expand: over.commit_message_expand.or(self.commit_message_expand),
            line_blame: over.line_blame.or(self.line_blame),
            commit_message_lines: over.commit_message_lines.or(self.commit_message_lines),
            background: over.background.or(self.background),
        }
    }
}

/// The settings that one config file sets, before the files merge.
///
/// `None` (or an empty map / [`ViewDefaults::default`]) means the file does
/// not set that key, so the lower file or the built-in default applies.
#[derive(Debug, Clone, Default)]
struct ConfigFileSettings {
    hidden_paths: Option<Vec<String>>,
    ignored_paths: Option<Vec<String>>,
    max_depth: Option<u32>,
    default_branches: Option<BTreeMap<String, String>>,
    editor: Option<String>,
    diff_tool: Option<String>,
    view_defaults: ViewDefaults,
    runtime: RuntimeKeys,
}

impl ConfigFileSettings {
    /// Merge `over` on top of `self` (`self` is the user file, `over` the
    /// workspace file). A top-level key that `over` sets wins. `viewDefaults`
    /// and `defaultBranches` merge per sub-key. `hiddenPaths` and
    /// `ignoredPaths` are each replaced, not joined.
    fn overlay(self, over: ConfigFileSettings) -> ConfigFileSettings {
        let default_branches = match (self.default_branches, over.default_branches) {
            (Some(mut base), Some(top)) => {
                base.extend(top);
                Some(base)
            }
            (base, top) => top.or(base),
        };
        ConfigFileSettings {
            hidden_paths: over.hidden_paths.or(self.hidden_paths),
            ignored_paths: over.ignored_paths.or(self.ignored_paths),
            max_depth: over.max_depth.or(self.max_depth),
            default_branches,
            editor: over.editor.or(self.editor),
            diff_tool: over.diff_tool.or(self.diff_tool),
            view_defaults: self.view_defaults.overlay(over.view_defaults),
            runtime: self.runtime.overlay(over.runtime),
        }
    }

    /// Fill each key that no file sets with its built-in default.
    fn into_config(self) -> WorkspaceStatusConfig {
        WorkspaceStatusConfig {
            hidden_paths: self.hidden_paths.unwrap_or_default(),
            ignored_paths: self.ignored_paths.unwrap_or_default(),
            max_depth: self.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
            default_branches: self.default_branches.unwrap_or_default(),
            editor: self.editor,
            diff_tool: self.diff_tool,
            view_defaults: self.view_defaults,
            runtime: self.runtime,
        }
    }
}

/// Parse an optional command-string key (`editor`, `diffTool`). Blank is unset.
fn parse_command_key(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<Option<String>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    let Some(s) = v.as_str() else {
        return Err(format!("{file} {key} must be a string"));
    };
    let trimmed = s.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}

/// Parse an optional string key whose value must be one of `choices`
/// (trimmed, case-sensitive). `null` is omitted.
fn parse_choice_key<T: Copy>(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
    choices: &[(&str, T)],
) -> Result<Option<T>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    let picked = v.as_str().map(str::trim).and_then(|s| {
        choices
            .iter()
            .find(|(name, _)| *name == s)
            .map(|(_, out)| *out)
    });
    picked.map(Some).ok_or_else(|| {
        let names: Vec<String> = choices.iter().map(|(n, _)| format!("\"{n}\"")).collect();
        match names.as_slice() {
            [a, b] => format!("{file} {key} must be {a} or {b}"),
            _ => format!("{file} {key} must be one of {}", names.join(", ")),
        }
    })
}

/// Parse `theme`: one of the built-in theme ids.
fn parse_theme_key(
    file: &str,
    value: Option<serde_json::Value>,
) -> Result<Option<ThemeId>, String> {
    let choices: Vec<(&str, ThemeId)> = THEME_IDS.iter().map(|id| (id.as_str(), *id)).collect();
    parse_choice_key(file, "theme", value, &choices)
}

/// Parse an optional JSON integer key that must be at least `min`.
fn parse_int_key(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
    min: u64,
) -> Result<Option<u64>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    match v.as_u64() {
        Some(n) if n >= min => Ok(Some(n)),
        _ => Err(format!("{file} {key} must be an integer >= {min}")),
    }
}

/// Parse an optional JSON boolean key.
fn parse_bool_key(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<Option<bool>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    v.as_bool()
        .map(Some)
        .ok_or_else(|| format!("{file} {key} must be true or false"))
}

/// Absolute path, or `~/...` joined to `home`. `Ok(None)` for any other
/// (relative) path. `Err` when the value needs `HOME` and `home` is unset.
fn expand_config_path(
    file: &str,
    key: &str,
    raw: &str,
    home: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    if let Some(rest) = raw.strip_prefix("~/") {
        let Some(home) = home.map(str::trim).filter(|h| !h.is_empty()) else {
            return Err(format!("{file} {key} starts with ~/ but HOME is not set"));
        };
        return Ok(Some(PathBuf::from(home).join(rest)));
    }
    let path = PathBuf::from(raw);
    Ok(path.is_absolute().then_some(path))
}

/// Parse a store path key (`updateCheckStore`, `commentStore`,
/// `viewedStore`, `uiStore`): absolute, or `~/...` expanded with `home`.
fn parse_path_key(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
    home: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    let bad = || format!("{file} {key} must be an absolute path or a path that starts with ~/");
    let raw = v
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(bad)?;
    expand_config_path(file, key, raw, home)?
        .map(Some)
        .ok_or_else(bad)
}

/// Parse a program key (`git`, `prTitleCommand`): an absolute path, `~/...`
/// expanded with `home`, or a bare command name (no `/` or `\\`) that is
/// looked up on `PATH`.
fn parse_program_key(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
    home: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let Some(v) = value else {
        return Ok(None);
    };
    let bad = || {
        format!(
            "{file} {key} must be an absolute path, a path that starts with ~/, or a command name without /"
        )
    };
    let raw = v
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(bad)?;
    if !raw.contains(['/', '\\']) && !raw.starts_with('~') {
        return Ok(Some(PathBuf::from(raw)));
    }
    expand_config_path(file, key, raw, home)?
        .map(Some)
        .ok_or_else(bad)
}

/// Parse the [`RuntimeKeys`] of one config file.
fn parse_runtime_keys(
    file: &str,
    parsed: &mut RawConfig,
    home: Option<&str>,
) -> Result<RuntimeKeys, String> {
    Ok(RuntimeKeys {
        theme: parse_theme_key(file, parsed.theme.take())?,
        theme_mode: parse_choice_key(
            file,
            "themeMode",
            parsed.theme_mode.take(),
            &[
                ("auto", ThemeMode::Auto),
                ("dark", ThemeMode::Dark),
                ("light", ThemeMode::Light),
            ],
        )?,
        glyphs: parse_choice_key(
            file,
            "glyphs",
            parsed.glyphs.take(),
            &[("nerd", GlyphSet::Nerd), ("ascii", GlyphSet::Ascii)],
        )?,
        watch_ms: parse_int_key(file, "watchMs", parsed.watch_ms.take(), 0)?,
        fetch_ms: parse_int_key(file, "fetchMs", parsed.fetch_ms.take(), 0)?,
        fetch_concurrency: parse_int_key(
            file,
            "fetchConcurrency",
            parsed.fetch_concurrency.take(),
            1,
        )?
        .map(|n| usize::try_from(n).unwrap_or(usize::MAX)),
        update_check: parse_bool_key(file, "updateCheck", parsed.update_check.take())?,
        update_check_store: parse_path_key(
            file,
            "updateCheckStore",
            parsed.update_check_store.take(),
            home,
        )?,
        comment_store: parse_path_key(file, "commentStore", parsed.comment_store.take(), home)?,
        viewed_store: parse_path_key(file, "viewedStore", parsed.viewed_store.take(), home)?,
        ui_store: parse_path_key(file, "uiStore", parsed.ui_store.take(), home)?,
        git: parse_program_key(file, "git", parsed.git.take(), home)?,
        term_progress: parse_bool_key(file, "termProgress", parsed.term_progress.take())?,
        smooth_scroll: parse_bool_key(file, "smoothScroll", parsed.smooth_scroll.take())?,
        pr_polling: parse_bool_key(file, "prPolling", parsed.pr_polling.take())?,
        pr_title_command: parse_program_key(
            file,
            "prTitleCommand",
            parsed.pr_title_command.take(),
            home,
        )?,
    })
}

/// Which config file is parsed. The workspace file cannot set the user-only keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFileKind {
    User,
    Workspace,
}

/// Fail when a file still sets the removed `ignoredRepos` key (`null` too).
fn reject_ignored_repos(file: &str, value: Option<&serde_json::Value>) -> Result<(), String> {
    match value {
        None => Ok(()),
        Some(_) => Err(format!(
            "{file} ignoredRepos is no longer supported: use hiddenPaths (hidden by default) or ignoredPaths (never shown)"
        )),
    }
}

/// Parse one workspace-relative path list (`hiddenPaths`, `ignoredPaths`).
///
/// An omitted or `null` key is `None` (the file sets nothing). Each entry
/// must be a string relative to the workspace root (see
/// [`path_entry_is_invalid`]). The result is normalized, sorted, and
/// deduped.
fn parse_path_list(
    file: &str,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<Option<Vec<String>>, String> {
    let entries = match value {
        Some(serde_json::Value::Array(entries)) => entries,
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(_) => return Err(format!("{file} {key} must be a string array")),
    };
    let mut out = Vec::new();
    for entry in entries {
        let Some(s) = entry.as_str() else {
            return Err(format!("{file} {key} must contain only strings"));
        };
        if path_entry_is_invalid(s.trim()) {
            return Err(format!(
                "{file} {key} entries must be paths relative to the workspace root, without `..` (got {s:?})"
            ));
        }
        out.push(s.to_string());
    }
    Ok(Some(normalize_path_list(&out)))
}

/// Inputs that every config file parse shares.
struct ParseContext<'a> {
    /// `$HOME` for `~/` expansion in path keys.
    home: Option<&'a str>,
    /// User config file path for errors that point there.
    user_file: &'a str,
}

/// Keys that pick a program to run or a file to write, with their env var
/// (`None` when the key has no env var of its own).
///
/// A workspace file can be committed to a repo, so only the user file (or
/// the env var) may set these.
const USER_ONLY_KEYS: [(&str, Option<&str>); 8] = [
    ("git", Some("WORKSPACE_STATUS_GIT")),
    ("updateCheckStore", Some("WS_STATUS_UPDATE_CHECK_STORE")),
    ("commentStore", Some("WS_STATUS_COMMENT_STORE")),
    ("viewedStore", Some("WS_STATUS_VIEWED_STORE")),
    ("uiStore", Some("WS_STATUS_UI_STORE")),
    ("prTitleCommand", Some("WS_STATUS_PR_TITLE_CMD")),
    ("editor", None),
    ("diffTool", None),
];

/// Fail when the workspace file sets a [`USER_ONLY_KEYS`] key. `null`
/// counts as omitted.
fn reject_user_only_keys(
    file: &str,
    kind: ConfigFileKind,
    parsed: &RawConfig,
    user_file: &str,
) -> Result<(), String> {
    if kind != ConfigFileKind::Workspace {
        return Ok(());
    }
    let set = [
        parsed.git.is_some(),
        parsed.update_check_store.is_some(),
        parsed.comment_store.is_some(),
        parsed.viewed_store.is_some(),
        parsed.ui_store.is_some(),
        parsed.pr_title_command.is_some(),
        parsed.editor.is_some(),
        parsed.diff_tool.is_some(),
    ];
    match USER_ONLY_KEYS.iter().zip(set).find(|(_, is_set)| *is_set) {
        Some(((key, Some(env_var)), _)) => Err(format!(
            "{file} {key} is only allowed in the user config file ({user_file}) or {env_var}"
        )),
        Some(((key, None), _)) => Err(format!(
            "{file} {key} is only allowed in the user config file ({user_file})"
        )),
        None => Ok(()),
    }
}

/// Parse the JSON text of one config file. `file` labels the file in errors.
fn parse_config_file(
    file: &str,
    kind: ConfigFileKind,
    text: &str,
    ctx: &ParseContext,
) -> Result<ConfigFileSettings, String> {
    let mut parsed: RawConfig = serde_json::from_str(text)
        .map_err(|e| format!("{file} is not a valid JSON object: {e}"))?;
    reject_ignored_repos(file, parsed.ignored_repos.as_ref())?;
    let hidden_paths = parse_path_list(file, "hiddenPaths", parsed.hidden_paths.take())?;
    let ignored_paths = parse_path_list(file, "ignoredPaths", parsed.ignored_paths.take())?;
    reject_user_only_keys(file, kind, &parsed, ctx.user_file)?;
    let runtime = parse_runtime_keys(file, &mut parsed, ctx.home)?;

    let max_depth = match parsed.max_depth {
        None => None,
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => Some(n as u32),
            _ => return Err(format!("{file} maxDepth must be a positive integer")),
        },
    };

    let default_branches = match parsed.default_branches {
        None => None,
        Some(v) => {
            let Some(obj) = v.as_object() else {
                return Err(format!("{file} defaultBranches must be an object"));
            };
            let mut map = BTreeMap::new();
            for (repo, branch) in obj {
                let Some(branch) = branch.as_str() else {
                    return Err(format!(
                        "{file} defaultBranches values must be strings (key: {repo})"
                    ));
                };
                map.insert(repo.clone(), branch.to_string());
            }
            Some(normalize_default_branches(&map))
        }
    };

    Ok(ConfigFileSettings {
        hidden_paths,
        ignored_paths,
        max_depth,
        default_branches,
        editor: parse_command_key(file, "editor", parsed.editor)?,
        diff_tool: parse_command_key(file, "diffTool", parsed.diff_tool)?,
        view_defaults: parse_view_defaults(file, parsed.view_defaults)?,
        runtime,
    })
}

/// Read and parse one config file. A missing file is `Ok(None)`. Any other
/// read error names the file: no permission, a directory, or a symlink
/// whose target is missing (that read is also `NotFound`, so the link
/// itself is checked).
fn read_config_file(
    path: &Path,
    file: &str,
    kind: ConfigFileKind,
    ctx: &ParseContext,
) -> Result<Option<ConfigFileSettings>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(format!("{file}: symlink target is missing: {e}"));
            }
            return Ok(None);
        }
        Err(e) => return Err(format!("{file}: {e}")),
    };
    parse_config_file(file, kind, &text, ctx).map(Some)
}

/// Path of the user config file from the real environment.
///
/// See [`user_config_path_from_env`].
pub fn user_config_path() -> Option<PathBuf> {
    user_config_path_from_env(|key| env::var(key).ok())
}

/// Resolve the user config file path from an env lookup.
///
/// `$XDG_CONFIG_HOME/my-workspace-status/config.json`, else
/// `$HOME/.config/my-workspace-status/config.json`. A blank or
/// whitespace-only `XDG_CONFIG_HOME` counts as unset. `None` when neither
/// variable gives a directory (then there is no user config).
pub fn user_config_path_from_env<F>(mut get: F) -> Option<PathBuf>
where
    F: FnMut(&str) -> Option<String>,
{
    let config_home = get("XDG_CONFIG_HOME")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            get("HOME")
                .filter(|s| !s.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(config_home.join(USER_CONFIG_DIR).join(USER_CONFIG_FILENAME))
}

/// Load the user config file and the workspace config file, then merge them.
///
/// `user_file` is the user config path ([`user_config_path`]); `None` or a
/// missing file means no user config. The workspace file is
/// [`CONFIG_FILENAME`] under `workspace_root`. The workspace file wins per
/// top-level key; `viewDefaults` and `defaultBranches` merge per sub-key
/// (workspace wins); `hiddenPaths` and `ignoredPaths` are each replaced,
/// not joined. Keys that no file sets keep the built-in defaults.
/// `hiddenPaths` and `ignoredPaths` are optional in both files; an omitted or
/// `null` key sets nothing. The removed `ignoredRepos` key is an error in
/// either file. An invalid file is an error that names that
/// file (the full path for the user file, [`CONFIG_FILENAME`] for the
/// workspace file).
///
/// Path keys that start with `~/` expand with `$HOME`. The workspace file
/// must not set `git`, `updateCheckStore`, `commentStore`, `viewedStore`,
/// `uiStore`, or `prTitleCommand` (it can be committed to a repo); those
/// keys are user-file only.
pub fn load_config_files(
    user_file: Option<&Path>,
    workspace_root: &Path,
) -> Result<WorkspaceStatusConfig, String> {
    load_config_files_with_home(user_file, workspace_root, env::var("HOME").ok().as_deref())
}

/// [`load_config_files`] with an injected `HOME` for `~/` expansion.
fn load_config_files_with_home(
    user_file: Option<&Path>,
    workspace_root: &Path,
    home: Option<&str>,
) -> Result<WorkspaceStatusConfig, String> {
    let user_label = user_file.map_or_else(
        || format!("~/.config/{USER_CONFIG_DIR}/{USER_CONFIG_FILENAME}"),
        |path| path.display().to_string(),
    );
    let ctx = ParseContext {
        home,
        user_file: &user_label,
    };
    let user = match user_file {
        Some(path) => read_config_file(path, &user_label, ConfigFileKind::User, &ctx)?,
        None => None,
    };
    let workspace = read_config_file(
        &workspace_root.join(CONFIG_FILENAME),
        CONFIG_FILENAME,
        ConfigFileKind::Workspace,
        &ctx,
    )?;
    let merged = user
        .unwrap_or_default()
        .overlay(workspace.unwrap_or_default());
    Ok(merged.into_config())
}

/// Load the merged workspace-status config for `cwd` (the workspace root).
///
/// Reads the user config file at [`user_config_path`] and
/// [`CONFIG_FILENAME`] under `cwd`; see [`load_config_files`] for the merge
/// rules. With neither file: nothing is hidden or ignored and maxDepth is 3.
pub fn load_workspace_status_config(cwd: &Path) -> Result<WorkspaceStatusConfig, String> {
    load_config_files(user_config_path().as_deref(), cwd)
}

#[cfg(test)]
mod tests;
