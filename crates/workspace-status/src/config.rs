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
use crate::tui::theme::{ThemeId, THEME_IDS};
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
    pub ignored_repos: Vec<String>,
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

/// Config keys that an env var can also set.
///
/// Each field is `None` when no file sets the key. [`crate::settings::Settings`]
/// resolves each one: a valid env var wins, then this value, then the
/// built-in default. Path values are already expanded (`~/` with `HOME`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeKeys {
    /// `theme` (`WS_STATUS_THEME`): a built-in theme id.
    pub theme: Option<ThemeId>,
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
    /// `git` (`WORKSPACE_STATUS_GIT`): git binary path or command name.
    pub git: Option<PathBuf>,
}

impl RuntimeKeys {
    /// Per-key merge: each key that `over` sets wins; the others keep `self`.
    fn overlay(self, over: RuntimeKeys) -> RuntimeKeys {
        RuntimeKeys {
            theme: over.theme.or(self.theme),
            glyphs: over.glyphs.or(self.glyphs),
            watch_ms: over.watch_ms.or(self.watch_ms),
            fetch_ms: over.fetch_ms.or(self.fetch_ms),
            fetch_concurrency: over.fetch_concurrency.or(self.fetch_concurrency),
            update_check: over.update_check.or(self.update_check),
            update_check_store: over.update_check_store.or(self.update_check_store),
            comment_store: over.comment_store.or(self.comment_store),
            viewed_store: over.viewed_store.or(self.viewed_store),
            git: over.git.or(self.git),
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
}

impl WorkspaceStatusConfig {
    pub fn with_defaults() -> Self {
        Self {
            ignored_repos: Vec::new(),
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
    ignored_repos: Option<serde_json::Value>,
    max_depth: Option<serde_json::Value>,
    default_branches: Option<serde_json::Value>,
    editor: Option<serde_json::Value>,
    diff_tool: Option<serde_json::Value>,
    view_defaults: Option<serde_json::Value>,
    theme: Option<serde_json::Value>,
    glyphs: Option<serde_json::Value>,
    watch_ms: Option<serde_json::Value>,
    fetch_ms: Option<serde_json::Value>,
    fetch_concurrency: Option<serde_json::Value>,
    update_check: Option<serde_json::Value>,
    update_check_store: Option<serde_json::Value>,
    comment_store: Option<serde_json::Value>,
    viewed_store: Option<serde_json::Value>,
    git: Option<serde_json::Value>,
}

fn normalize_ignored(repos: &[String]) -> Vec<String> {
    let mut out: Vec<String> = repos
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
    const KEYS: [&str; 7] = [
        "tree",
        "commitTree",
        "diff",
        "wrap",
        "commitMessage",
        "commitMessageLines",
        "lineBlame",
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
        }
    }
}

/// The settings that one config file sets, before the files merge.
///
/// `None` (or an empty map / [`ViewDefaults::default`]) means the file does
/// not set that key, so the lower file or the built-in default applies.
#[derive(Debug, Clone, Default)]
struct ConfigFileSettings {
    ignored_repos: Option<Vec<String>>,
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
    /// and `defaultBranches` merge per sub-key. `ignoredRepos` is replaced.
    fn overlay(self, over: ConfigFileSettings) -> ConfigFileSettings {
        let default_branches = match (self.default_branches, over.default_branches) {
            (Some(mut base), Some(top)) => {
                base.extend(top);
                Some(base)
            }
            (base, top) => top.or(base),
        };
        ConfigFileSettings {
            ignored_repos: over.ignored_repos.or(self.ignored_repos),
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
            ignored_repos: self.ignored_repos.unwrap_or_default(),
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
/// `viewedStore`): absolute, or `~/...` expanded with `home`.
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

/// Parse `git`: an absolute path, `~/...` expanded with `home`, or a bare
/// command name (no `/` or `\\`) that is looked up on `PATH`.
fn parse_git_key(
    file: &str,
    value: Option<serde_json::Value>,
    home: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    const KEY: &str = "git";
    let Some(v) = value else {
        return Ok(None);
    };
    let bad = || {
        format!(
            "{file} {KEY} must be an absolute path, a path that starts with ~/, or a command name without /"
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
    expand_config_path(file, KEY, raw, home)?
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
        git: parse_git_key(file, parsed.git.take(), home)?,
    })
}

/// Which config file is parsed. Only the workspace file must set `ignoredRepos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFileKind {
    User,
    Workspace,
}

/// Parse `ignoredRepos`. The workspace file requires it (`null` counts as
/// missing); in the user file an omitted or `null` key is `None`.
fn parse_ignored_repos(
    file: &str,
    kind: ConfigFileKind,
    value: Option<serde_json::Value>,
) -> Result<Option<Vec<String>>, String> {
    let missing = || format!("{file} must contain an ignoredRepos string array");
    let repos = match value {
        Some(serde_json::Value::Array(repos)) => repos,
        None | Some(serde_json::Value::Null) if kind == ConfigFileKind::User => return Ok(None),
        _ => return Err(missing()),
    };
    let mut ignored = Vec::new();
    for repo in repos {
        let Some(s) = repo.as_str() else {
            return Err(format!("{file} ignoredRepos must contain only strings"));
        };
        ignored.push(s.to_string());
    }
    Ok(Some(normalize_ignored(&ignored)))
}

/// Inputs that every config file parse shares.
struct ParseContext<'a> {
    /// `$HOME` for `~/` expansion in path keys.
    home: Option<&'a str>,
    /// User config file path for errors that point there.
    user_file: &'a str,
}

/// Keys that pick a program to run or a file to write, with their env var.
///
/// A workspace file can be committed to a repo, so only the user file (or
/// the env var) may set these.
const USER_ONLY_KEYS: [(&str, &str); 4] = [
    ("git", "WORKSPACE_STATUS_GIT"),
    ("updateCheckStore", "WS_STATUS_UPDATE_CHECK_STORE"),
    ("commentStore", "WS_STATUS_COMMENT_STORE"),
    ("viewedStore", "WS_STATUS_VIEWED_STORE"),
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
    ];
    match USER_ONLY_KEYS.iter().zip(set).find(|(_, is_set)| *is_set) {
        Some(((key, env_var), _)) => Err(format!(
            "{file} {key} is only allowed in the user config file ({user_file}) or {env_var}"
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
    let mut parsed: RawConfig = serde_json::from_str(text).map_err(|e| match kind {
        ConfigFileKind::User => format!("{file} is not a valid JSON object: {e}"),
        ConfigFileKind::Workspace => format!("{file} must contain an ignoredRepos string array"),
    })?;
    let ignored_repos = parse_ignored_repos(file, kind, parsed.ignored_repos.take())?;
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
        ignored_repos,
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
/// (workspace wins); `ignoredRepos` is replaced, not joined. Keys that no
/// file sets keep the built-in defaults. `ignoredRepos` is required in the
/// workspace file and optional in the user file. An invalid file is an error that
/// names that file (the full path for the user file, [`CONFIG_FILENAME`]
/// for the workspace file).
///
/// Path keys that start with `~/` expand with `$HOME`. The workspace file
/// must not set `git`, `updateCheckStore`, `commentStore`, or `viewedStore`
/// (it can be committed to a repo); those keys are user-file only.
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
/// rules. With neither file: nothing is ignored and maxDepth is 3.
pub fn load_workspace_status_config(cwd: &Path) -> Result<WorkspaceStatusConfig, String> {
    load_config_files(user_config_path().as_deref(), cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn missing_config_uses_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "ws-config-missing-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let cfg = load_config_files(None, &dir).unwrap();
        assert!(cfg.ignored_repos.is_empty());
        assert_eq!(cfg.max_depth, 3);
        assert_eq!(cfg.diff_tool, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_ignored_repos_sorted() {
        let dir = std::env::temp_dir().join(format!(
            "ws-config-ok-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(CONFIG_FILENAME),
            r#"{"ignoredRepos":["notes","./vendor/"]}"#,
        )
        .unwrap();
        let cfg = load_config_files(None, &dir).unwrap();
        assert_eq!(cfg.ignored_repos, vec!["notes", "vendor"]);
        assert_eq!(cfg.diff_tool, None);
        let _ = fs::remove_dir_all(&dir);
    }

    fn write_config(dir_prefix: &str, json: &str) -> std::path::PathBuf {
        // Tests share a prefix and run in parallel: a counter keeps each dir unique.
        static NEXT_DIR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "{dir_prefix}-{}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(CONFIG_FILENAME), json).unwrap();
        dir
    }

    #[test]
    fn omit_diff_tool_is_none() {
        let dir = write_config("ws-config-omit-diff", r#"{"ignoredRepos":[]}"#);
        let cfg = load_config_files(None, &dir).unwrap();
        assert_eq!(cfg.diff_tool, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_tool_vimdiff_is_kept() {
        let dir = write_config(
            "ws-config-diff-vim",
            r#"{"ignoredRepos":[],"diffTool":"vimdiff"}"#,
        );
        let cfg = load_config_files(None, &dir).unwrap();
        assert_eq!(cfg.diff_tool.as_deref(), Some("vimdiff"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blank_diff_tool_is_none() {
        let dir = write_config(
            "ws-config-diff-blank",
            r#"{"ignoredRepos":[],"diffTool":"  "}"#,
        );
        let cfg = load_config_files(None, &dir).unwrap();
        assert_eq!(cfg.diff_tool, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_string_diff_tool_is_error() {
        let dir = write_config("ws-config-diff-bad", r#"{"ignoredRepos":[],"diffTool":1}"#);
        let err = load_config_files(None, &dir).unwrap_err();
        assert!(
            err.contains("diffTool must be a string"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn load_view_defaults(json: &str) -> Result<ViewDefaults, String> {
        let dir = write_config("ws-config-view", json);
        let out = load_config_files(None, &dir).map(|cfg| cfg.view_defaults);
        let _ = fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn omitted_view_defaults_are_all_none() {
        assert_eq!(
            load_view_defaults(r#"{"ignoredRepos":[]}"#).unwrap(),
            ViewDefaults::default()
        );
        assert_eq!(
            WorkspaceStatusConfig::with_defaults().view_defaults,
            ViewDefaults::default()
        );
    }

    #[test]
    fn empty_view_defaults_object_is_all_none() {
        assert_eq!(
            load_view_defaults(r#"{"ignoredRepos":[],"viewDefaults":{}}"#).unwrap(),
            ViewDefaults::default()
        );
    }

    #[test]
    fn view_defaults_first_values_are_true() {
        let got = load_view_defaults(
            r#"{"ignoredRepos":[],"viewDefaults":{"tree":"tree","commitTree":"tree","diff":"split","wrap":"wrap","commitMessage":"expand","lineBlame":"show"}}"#,
        )
        .unwrap();
        assert_eq!(
            got,
            ViewDefaults {
                tree: Some(true),
                commit_tree: Some(true),
                diff_split: Some(true),
                wrap: Some(true),
                commit_message_expand: Some(true),
                line_blame: Some(true),
                commit_message_lines: None,
            }
        );
    }

    #[test]
    fn view_defaults_second_values_are_false() {
        let got = load_view_defaults(
            r#"{"ignoredRepos":[],"viewDefaults":{"tree":"flat","commitTree":"flat","diff":"inline","wrap":"unwrap","commitMessage":"collapse","lineBlame":"hide"}}"#,
        )
        .unwrap();
        assert_eq!(
            got,
            ViewDefaults {
                tree: Some(false),
                commit_tree: Some(false),
                diff_split: Some(false),
                wrap: Some(false),
                commit_message_expand: Some(false),
                line_blame: Some(false),
                commit_message_lines: None,
            }
        );
    }

    #[test]
    fn view_defaults_value_is_trimmed() {
        let got = load_view_defaults(r#"{"ignoredRepos":[],"viewDefaults":{"wrap":"  unwrap "}}"#)
            .unwrap();
        assert_eq!(got.wrap, Some(false));
        assert_eq!(got.tree, None);
    }

    #[test]
    fn view_defaults_not_an_object_is_error() {
        for raw in [r#"[]"#, r#""wrap""#, "1"] {
            let err = load_view_defaults(&format!(r#"{{"ignoredRepos":[],"viewDefaults":{raw}}}"#))
                .unwrap_err();
            assert_eq!(
                err, ".workspace-status-config.json viewDefaults must be an object",
                "viewDefaults: {raw}"
            );
        }
    }

    #[test]
    fn view_defaults_bad_value_names_the_key_and_choices() {
        let cases = [
            ("tree", "1", r#""tree" or "flat""#),
            ("tree", r#""Tree""#, r#""tree" or "flat""#),
            ("commitTree", r#""list""#, r#""tree" or "flat""#),
            ("diff", r#""sbs""#, r#""split" or "inline""#),
            ("diff", "true", r#""split" or "inline""#),
            ("wrap", r#""""#, r#""wrap" or "unwrap""#),
            ("wrap", r#""   ""#, r#""wrap" or "unwrap""#),
            ("wrap", "false", r#""wrap" or "unwrap""#),
            ("commitMessage", r#""open""#, r#""expand" or "collapse""#),
            ("commitMessage", "null", r#""expand" or "collapse""#),
            ("lineBlame", r#""on""#, r#""show" or "hide""#),
            ("lineBlame", "true", r#""show" or "hide""#),
        ];
        for (key, raw, choices) in cases {
            let err = load_view_defaults(&format!(
                r#"{{"ignoredRepos":[],"viewDefaults":{{"{key}":{raw}}}}}"#
            ))
            .unwrap_err();
            assert_eq!(
                err,
                format!(".workspace-status-config.json viewDefaults.{key} must be {choices}"),
                "{key}: {raw}"
            );
        }
    }

    #[test]
    fn view_defaults_commit_message_lines_accepts_min_max_and_default() {
        for n in [1usize, 4, 20] {
            let got = load_view_defaults(&format!(
                r#"{{"ignoredRepos":[],"viewDefaults":{{"commitMessageLines":{n}}}}}"#
            ))
            .unwrap();
            assert_eq!(got.commit_message_lines, Some(n));
            assert_eq!(got.commit_message_expand, None);
        }
        assert_eq!(
            load_view_defaults(r#"{"ignoredRepos":[],"viewDefaults":{"wrap":"wrap"}}"#)
                .unwrap()
                .commit_message_lines,
            None,
            "omitted key keeps the in-app default"
        );
    }

    #[test]
    fn view_defaults_commit_message_lines_rejects_bad_values() {
        for raw in [
            "0", "21", "-1", r#""8""#, "8.5", "8.0", "true", "null", "[8]",
        ] {
            let err = load_view_defaults(&format!(
                r#"{{"ignoredRepos":[],"viewDefaults":{{"commitMessageLines":{raw}}}}}"#
            ))
            .unwrap_err();
            assert_eq!(
                err,
                ".workspace-status-config.json viewDefaults.commitMessageLines must be an integer from 1 to 20",
                "commitMessageLines: {raw}"
            );
        }
    }

    #[test]
    fn view_defaults_unknown_key_is_error() {
        let err = load_view_defaults(
            r#"{"ignoredRepos":[],"viewDefaults":{"wrap":"wrap","theme":"dark"}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err,
            r#".workspace-status-config.json viewDefaults has unknown key "theme""#
        );
    }

    /// Temp root with an optional user file and an optional workspace file.
    /// Returns `(root, user_file, workspace_root)`; `user_file` is the path the
    /// loader gets, whether or not the file exists.
    fn layered(
        user_json: Option<&str>,
        workspace_json: Option<&str>,
    ) -> (PathBuf, PathBuf, PathBuf) {
        static NEXT_DIR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ws-config-layered-{}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let user_file = root
            .join("xdg-config")
            .join(USER_CONFIG_DIR)
            .join(USER_CONFIG_FILENAME);
        let workspace = root.join("workspace");
        fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        if let Some(json) = user_json {
            fs::write(&user_file, json).unwrap();
        }
        if let Some(json) = workspace_json {
            fs::write(workspace.join(CONFIG_FILENAME), json).unwrap();
        }
        (root, user_file, workspace)
    }

    fn load_layered(
        user_json: Option<&str>,
        workspace_json: Option<&str>,
    ) -> Result<WorkspaceStatusConfig, String> {
        let (root, user_file, workspace) = layered(user_json, workspace_json);
        let out = load_config_files(Some(&user_file), &workspace);
        let _ = fs::remove_dir_all(root);
        out
    }

    #[test]
    fn layered_no_files_uses_defaults() {
        let cfg = load_layered(None, None).unwrap();
        assert!(cfg.ignored_repos.is_empty());
        assert_eq!(cfg.max_depth, DEFAULT_MAX_DEPTH);
        assert!(cfg.default_branches.is_empty());
        assert_eq!(cfg.editor, None);
        assert_eq!(cfg.diff_tool, None);
        assert_eq!(cfg.view_defaults, ViewDefaults::default());

        let (root, _, workspace) = layered(None, None);
        let cfg = load_config_files(None, &workspace).unwrap();
        assert_eq!(cfg.max_depth, DEFAULT_MAX_DEPTH);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn layered_user_file_only() {
        let cfg = load_layered(
            Some(
                r#"{"ignoredRepos":["notes"],"maxDepth":2,"editor":"nvim","diffTool":"code --diff --wait","defaultBranches":{"app":"develop"},"viewDefaults":{"wrap":"unwrap"}}"#,
            ),
            None,
        )
        .unwrap();
        assert_eq!(cfg.ignored_repos, vec!["notes"]);
        assert_eq!(cfg.max_depth, 2);
        assert_eq!(cfg.editor.as_deref(), Some("nvim"));
        assert_eq!(cfg.diff_tool.as_deref(), Some("code --diff --wait"));
        assert_eq!(
            cfg.default_branches.get("app").map(String::as_str),
            Some("develop")
        );
        assert_eq!(cfg.view_defaults.wrap, Some(false));
    }

    #[test]
    fn layered_workspace_file_only() {
        let cfg = load_layered(None, Some(r#"{"ignoredRepos":["vendor"],"maxDepth":4}"#)).unwrap();
        assert_eq!(cfg.ignored_repos, vec!["vendor"]);
        assert_eq!(cfg.max_depth, 4);
        assert_eq!(cfg.editor, None);
    }

    #[test]
    fn layered_workspace_wins_per_top_level_key() {
        let cfg = load_layered(
            Some(r#"{"ignoredRepos":[],"maxDepth":2,"editor":"nvim","diffTool":"vimdiff"}"#),
            Some(r#"{"ignoredRepos":[],"maxDepth":5,"diffTool":"  "}"#),
        )
        .unwrap();
        assert_eq!(cfg.max_depth, 5, "workspace maxDepth wins");
        assert_eq!(
            cfg.editor.as_deref(),
            Some("nvim"),
            "key the workspace omits keeps the user value"
        );
        assert_eq!(
            cfg.diff_tool.as_deref(),
            Some("vimdiff"),
            "blank workspace diffTool is unset, so the user value applies"
        );
    }

    #[test]
    fn layered_view_defaults_merge_per_sub_key() {
        let cfg = load_layered(
            Some(
                r#"{"ignoredRepos":[],"viewDefaults":{"tree":"flat","wrap":"unwrap","commitMessageLines":12}}"#,
            ),
            Some(r#"{"ignoredRepos":[],"viewDefaults":{"wrap":"wrap","lineBlame":"hide"}}"#),
        )
        .unwrap();
        assert_eq!(
            cfg.view_defaults,
            ViewDefaults {
                tree: Some(false),
                wrap: Some(true),
                line_blame: Some(false),
                commit_message_lines: Some(12),
                ..ViewDefaults::default()
            }
        );
    }

    #[test]
    fn layered_default_branches_merge_per_sub_key() {
        let cfg = load_layered(
            Some(r#"{"ignoredRepos":[],"defaultBranches":{"app":"develop","lib":"main"}}"#),
            Some(r#"{"ignoredRepos":[],"defaultBranches":{"./app/":"trunk","api":"dev"}}"#),
        )
        .unwrap();
        let got: Vec<(&str, &str)> = cfg
            .default_branches
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(got, vec![("api", "dev"), ("app", "trunk"), ("lib", "main")]);
    }

    #[test]
    fn layered_ignored_repos_are_replaced_not_joined() {
        let cfg = load_layered(
            Some(r#"{"ignoredRepos":["notes","vendor"]}"#),
            Some(r#"{"ignoredRepos":["scratch"]}"#),
        )
        .unwrap();
        assert_eq!(cfg.ignored_repos, vec!["scratch"]);

        let cfg = load_layered(
            Some(r#"{"ignoredRepos":["notes"]}"#),
            Some(r#"{"ignoredRepos":[]}"#),
        )
        .unwrap();
        assert!(
            cfg.ignored_repos.is_empty(),
            "workspace [] clears the user list"
        );
    }

    #[test]
    fn layered_invalid_user_file_names_the_user_path() {
        let cases = [
            (
                r#"{"ignoredRepos":"notes"}"#,
                "must contain an ignoredRepos string array",
            ),
            (
                r#"{"ignoredRepos":{"notes":true}}"#,
                "must contain an ignoredRepos string array",
            ),
            (
                r#"{"ignoredRepos":["notes",1]}"#,
                "ignoredRepos must contain only strings",
            ),
            (
                r#"{"ignoredRepos":[],"maxDepth":0}"#,
                "maxDepth must be a positive integer",
            ),
            (
                r#"{"ignoredRepos":[],"viewDefaults":{"commitMessageLines":0}}"#,
                "viewDefaults.commitMessageLines must be an integer from 1 to 20",
            ),
            (
                r#"{"ignoredRepos":[],"viewDefaults":{"wrap":"on"}}"#,
                r#"viewDefaults.wrap must be "wrap" or "unwrap""#,
            ),
        ];
        for (json, tail) in cases {
            let (root, user_file, workspace) = layered(Some(json), Some(r#"{"ignoredRepos":[]}"#));
            let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
            assert_eq!(err, format!("{} {tail}", user_file.display()), "{json}");
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn layered_invalid_workspace_file_names_the_workspace_file() {
        let err = load_layered(
            Some(r#"{"ignoredRepos":[]}"#),
            Some(r#"{"ignoredRepos":[],"editor":1}"#),
        )
        .unwrap_err();
        assert_eq!(err, ".workspace-status-config.json editor must be a string");
    }

    #[test]
    fn layered_unreadable_user_file_names_the_user_path() {
        let (root, user_file, workspace) = layered(None, None);
        fs::create_dir_all(&user_file).unwrap();
        let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
        assert!(
            err.starts_with(&format!("{}: ", user_file.display())),
            "{err}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn user_config_path_prefers_xdg_config_home() {
        let env = |key: &str| match key {
            "XDG_CONFIG_HOME" => Some("/xdg/config".to_string()),
            "HOME" => Some("/home/demo".to_string()),
            _ => None,
        };
        assert_eq!(
            user_config_path_from_env(env),
            Some(PathBuf::from("/xdg/config/my-workspace-status/config.json"))
        );
    }

    #[test]
    fn user_config_path_blank_xdg_falls_back_to_home() {
        for xdg in [None, Some(""), Some("   ")] {
            let env = |key: &str| match key {
                "XDG_CONFIG_HOME" => xdg.map(str::to_string),
                "HOME" => Some("/home/demo".to_string()),
                _ => None,
            };
            assert_eq!(
                user_config_path_from_env(env),
                Some(PathBuf::from(
                    "/home/demo/.config/my-workspace-status/config.json"
                )),
                "XDG_CONFIG_HOME: {xdg:?}"
            );
        }
    }

    #[test]
    fn user_config_path_without_xdg_or_home_is_none() {
        assert_eq!(user_config_path_from_env(|_| None), None);
        let blank = |key: &str| match key {
            "XDG_CONFIG_HOME" | "HOME" => Some(" ".to_string()),
            _ => None,
        };
        assert_eq!(user_config_path_from_env(blank), None);
    }

    #[test]
    fn blank_xdg_config_home_reads_the_home_dot_config_file() {
        let (root, _, workspace) = layered(None, None);
        let home = root.join("home");
        let user_file = home
            .join(".config")
            .join(USER_CONFIG_DIR)
            .join(USER_CONFIG_FILENAME);
        fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        fs::write(&user_file, r#"{"ignoredRepos":[],"maxDepth":7}"#).unwrap();
        let home_str = home.to_string_lossy().to_string();
        let path = user_config_path_from_env(|key| match key {
            "XDG_CONFIG_HOME" => Some(String::new()),
            "HOME" => Some(home_str.clone()),
            _ => None,
        });
        assert_eq!(path.as_deref(), Some(user_file.as_path()));
        let cfg = load_config_files(path.as_deref(), &workspace).unwrap();
        assert_eq!(cfg.max_depth, 7);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn layered_user_file_without_ignored_repos_loads() {
        for json in [
            r#"{"viewDefaults":{"wrap":"unwrap"}}"#,
            r#"{"ignoredRepos":null,"viewDefaults":{"wrap":"unwrap"}}"#,
        ] {
            let cfg = load_layered(Some(json), None).unwrap();
            assert!(cfg.ignored_repos.is_empty(), "{json}");
            assert_eq!(cfg.view_defaults.wrap, Some(false), "{json}");

            let cfg = load_layered(Some(json), Some(r#"{"ignoredRepos":["vendor"]}"#)).unwrap();
            assert_eq!(cfg.ignored_repos, vec!["vendor"], "{json}");
            assert_eq!(cfg.view_defaults.wrap, Some(false), "{json}");
        }
        assert_eq!(
            load_layered(Some("{}"), None).unwrap().max_depth,
            DEFAULT_MAX_DEPTH
        );
    }

    #[test]
    fn layered_user_ignored_repos_apply_without_workspace_file() {
        let cfg = load_layered(Some(r#"{"ignoredRepos":["./notes/","vendor"]}"#), None).unwrap();
        assert_eq!(cfg.ignored_repos, vec!["notes", "vendor"]);
    }

    #[test]
    fn workspace_file_still_requires_ignored_repos() {
        for json in [r#"{}"#, r#"{"ignoredRepos":null}"#, r#"{"maxDepth":2}"#] {
            let err = load_layered(Some(r#"{"ignoredRepos":["notes"]}"#), Some(json)).unwrap_err();
            assert_eq!(
                err, ".workspace-status-config.json must contain an ignoredRepos string array",
                "{json}"
            );
        }
    }

    fn load_layered_home(
        user_json: Option<&str>,
        workspace_json: Option<&str>,
        home: Option<&str>,
    ) -> Result<WorkspaceStatusConfig, String> {
        let (root, user_file, workspace) = layered(user_json, workspace_json);
        let out = load_config_files_with_home(Some(&user_file), &workspace, home);
        let _ = fs::remove_dir_all(root);
        out
    }

    /// Load `{<key>: <raw>}` as the user file (every runtime key is allowed
    /// there). Errors start with `<user>` in place of the temp user path.
    fn load_runtime_key(key: &str, raw: &str) -> Result<RuntimeKeys, String> {
        let (root, user_file, workspace) = layered(Some(&format!(r#"{{"{key}":{raw}}}"#)), None);
        let out = load_config_files_with_home(Some(&user_file), &workspace, Some("/home/demo"))
            .map(|cfg| cfg.runtime)
            .map_err(|e| e.replace(&user_file.display().to_string(), "<user>"));
        let _ = fs::remove_dir_all(root);
        out
    }

    #[test]
    fn runtime_keys_omitted_or_null_are_none() {
        assert_eq!(
            load_runtime_key("theme", "null").unwrap(),
            RuntimeKeys::default()
        );
        let cfg = load_layered(None, Some(r#"{"ignoredRepos":[]}"#)).unwrap();
        assert_eq!(cfg.runtime, RuntimeKeys::default());
        assert_eq!(
            WorkspaceStatusConfig::with_defaults().runtime,
            RuntimeKeys::default()
        );
    }

    #[test]
    fn runtime_keys_parse_valid_values() {
        let cfg = load_layered_home(
            Some(
                r#"{"theme":" dracula ","glyphs":"ascii","watchMs":0,"fetchMs":60000,"fetchConcurrency":4,"updateCheck":false,"updateCheckStore":"~/state/update-check.json","commentStore":"/var/ws/comments.json","viewedStore":"~/viewed.json","git":"git-wrapper"}"#,
            ),
            None,
            Some("/home/demo"),
        )
        .unwrap();
        assert_eq!(
            cfg.runtime,
            RuntimeKeys {
                theme: Some(ThemeId::Dracula),
                glyphs: Some(GlyphSet::Ascii),
                watch_ms: Some(0),
                fetch_ms: Some(60000),
                fetch_concurrency: Some(4),
                update_check: Some(false),
                update_check_store: Some(PathBuf::from("/home/demo/state/update-check.json")),
                comment_store: Some(PathBuf::from("/var/ws/comments.json")),
                viewed_store: Some(PathBuf::from("/home/demo/viewed.json")),
                git: Some(PathBuf::from("git-wrapper")),
            }
        );
        assert_eq!(
            load_runtime_key("glyphs", r#""nerd""#).unwrap().glyphs,
            Some(GlyphSet::Nerd)
        );
        assert_eq!(
            load_runtime_key("updateCheck", "true")
                .unwrap()
                .update_check,
            Some(true)
        );
        for (raw, want) in [
            (r#""/usr/local/bin/git""#, "/usr/local/bin/git"),
            (r#""~/bin/git""#, "/home/demo/bin/git"),
            (r#""git""#, "git"),
        ] {
            assert_eq!(
                load_runtime_key("git", raw).unwrap().git,
                Some(PathBuf::from(want)),
                "git: {raw}"
            );
        }
    }

    #[test]
    fn runtime_keys_bad_values_name_the_file_key_and_choices() {
        let themes = r#"must be one of "tokyo-night", "monokai", "dracula", "gruvbox-dark", "catppuccin-mocha""#;
        let path_rule = "must be an absolute path or a path that starts with ~/";
        let git_rule =
            "must be an absolute path, a path that starts with ~/, or a command name without /";
        let cases: Vec<(&str, &str, String)> = vec![
            ("theme", r#""solarized""#, themes.to_string()),
            ("theme", r#""Dracula""#, themes.to_string()),
            ("theme", r#""""#, themes.to_string()),
            ("theme", "1", themes.to_string()),
            (
                "glyphs",
                r#""unicode""#,
                r#"must be "nerd" or "ascii""#.to_string(),
            ),
            (
                "glyphs",
                r#""  ""#,
                r#"must be "nerd" or "ascii""#.to_string(),
            ),
            ("glyphs", "true", r#"must be "nerd" or "ascii""#.to_string()),
            ("watchMs", "-1", "must be an integer >= 0".to_string()),
            (
                "watchMs",
                r#""3000""#,
                "must be an integer >= 0".to_string(),
            ),
            ("watchMs", "1.5", "must be an integer >= 0".to_string()),
            ("fetchMs", "-5", "must be an integer >= 0".to_string()),
            ("fetchMs", "false", "must be an integer >= 0".to_string()),
            (
                "fetchConcurrency",
                "0",
                "must be an integer >= 1".to_string(),
            ),
            (
                "fetchConcurrency",
                r#""8""#,
                "must be an integer >= 1".to_string(),
            ),
            ("updateCheck", "0", "must be true or false".to_string()),
            (
                "updateCheck",
                r#""false""#,
                "must be true or false".to_string(),
            ),
            (
                "updateCheckStore",
                r#""state/check.json""#,
                path_rule.to_string(),
            ),
            ("updateCheckStore", r#""""#, path_rule.to_string()),
            (
                "commentStore",
                r#""./comments.json""#,
                path_rule.to_string(),
            ),
            ("commentStore", "1", path_rule.to_string()),
            ("viewedStore", r#""~viewed.json""#, path_rule.to_string()),
            ("git", r#""bin/git""#, git_rule.to_string()),
            ("git", r#""./git""#, git_rule.to_string()),
            ("git", r#""~""#, git_rule.to_string()),
            ("git", r#""   ""#, git_rule.to_string()),
            ("git", "[]", git_rule.to_string()),
        ];
        for (key, raw, tail) in cases {
            let err = load_runtime_key(key, raw).unwrap_err();
            assert_eq!(err, format!("<user> {key} {tail}"), "{key}: {raw}");
        }
        let err =
            load_layered(None, Some(r#"{"ignoredRepos":[],"glyphs":"unicode"}"#)).unwrap_err();
        assert_eq!(
            err,
            r#".workspace-status-config.json glyphs must be "nerd" or "ascii""#
        );
        let (root, user_file, workspace) = layered(Some(r#"{"fetchConcurrency":0}"#), None);
        let err = load_config_files_with_home(Some(&user_file), &workspace, None).unwrap_err();
        assert_eq!(
            err,
            format!(
                "{} fetchConcurrency must be an integer >= 1",
                user_file.display()
            )
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_path_keys_need_home_for_tilde() {
        for home in [None, Some(""), Some("  ")] {
            let (root, user_file, workspace) =
                layered(Some(r#"{"commentStore":"~/comments.json"}"#), None);
            let err = load_config_files_with_home(Some(&user_file), &workspace, home).unwrap_err();
            assert_eq!(
                err,
                format!(
                    "{} commentStore starts with ~/ but HOME is not set",
                    user_file.display()
                ),
                "HOME: {home:?}"
            );
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn runtime_keys_merge_per_key_workspace_wins() {
        let cfg = load_layered_home(
            Some(r#"{"theme":"dracula","watchMs":4000,"git":"/opt/git"}"#),
            Some(r#"{"ignoredRepos":[],"theme":"monokai","fetchMs":0}"#),
            Some("/home/demo"),
        )
        .unwrap();
        assert_eq!(cfg.runtime.theme, Some(ThemeId::Monokai), "workspace wins");
        assert_eq!(cfg.runtime.watch_ms, Some(4000), "user key kept");
        assert_eq!(cfg.runtime.fetch_ms, Some(0));
        assert_eq!(cfg.runtime.git, Some(PathBuf::from("/opt/git")));
    }

    #[test]
    fn invalid_user_json_is_a_json_error_naming_the_path() {
        for json in ["{", "[]", r#""notes""#, "{\"theme\": }"] {
            let (root, user_file, workspace) = layered(Some(json), None);
            let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
            let prefix = format!("{} is not a valid JSON object: ", user_file.display());
            assert!(err.starts_with(&prefix), "{json}: {err}");
            assert!(err.len() > prefix.len(), "{json}: serde message missing");
            let _ = fs::remove_dir_all(root);
        }
        let err = load_layered(None, Some("{")).unwrap_err();
        assert_eq!(
            err, ".workspace-status-config.json must contain an ignoredRepos string array",
            "workspace file keeps its message"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dangling_user_file_symlink_is_an_error_not_a_missing_file() {
        let (root, user_file, workspace) = layered(None, None);
        std::os::unix::fs::symlink(root.join("no-such-target.json"), &user_file).unwrap();
        let err = load_config_files(Some(&user_file), &workspace).unwrap_err();
        assert!(
            err.starts_with(&format!(
                "{}: symlink target is missing: ",
                user_file.display()
            )),
            "{err}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_file_rejects_user_only_keys() {
        for (key, raw, env_var) in [
            ("git", r#""sh""#, "WORKSPACE_STATUS_GIT"),
            ("git", r#""/usr/bin/git""#, "WORKSPACE_STATUS_GIT"),
            ("git", "1", "WORKSPACE_STATUS_GIT"),
            (
                "updateCheckStore",
                r#""~/.bashrc""#,
                "WS_STATUS_UPDATE_CHECK_STORE",
            ),
            (
                "commentStore",
                r#""/tmp/c.json""#,
                "WS_STATUS_COMMENT_STORE",
            ),
            ("viewedStore", r#""/tmp/v.json""#, "WS_STATUS_VIEWED_STORE"),
        ] {
            let workspace_json = format!(r#"{{"ignoredRepos":[],"{key}":{raw}}}"#);
            let (root, user_file, workspace) = layered(None, Some(&workspace_json));
            let err = load_config_files_with_home(Some(&user_file), &workspace, Some("/home/demo"))
                .unwrap_err();
            assert_eq!(
                err,
                format!(
                    ".workspace-status-config.json {key} is only allowed in the user config file ({}) or {env_var}",
                    user_file.display()
                ),
                "{key}: {raw}"
            );
            let err =
                load_config_files_with_home(None, &workspace, Some("/home/demo")).unwrap_err();
            assert_eq!(
                err,
                format!(
                    ".workspace-status-config.json {key} is only allowed in the user config file (~/.config/my-workspace-status/config.json) or {env_var}"
                ),
                "no user path: {key}"
            );
            let _ = fs::remove_dir_all(root);
        }
        let cfg = load_layered(
            None,
            Some(r#"{"ignoredRepos":[],"git":null,"commentStore":null}"#),
        )
        .unwrap();
        assert_eq!(
            cfg.runtime,
            RuntimeKeys::default(),
            "null counts as omitted"
        );
    }

    #[test]
    fn user_file_sets_user_only_keys_under_a_workspace_file() {
        let cfg = load_layered_home(
            Some(
                r#"{"git":"git-wrapper","updateCheckStore":"~/u.json","commentStore":"/c.json","viewedStore":"/v.json"}"#,
            ),
            Some(r#"{"ignoredRepos":[],"theme":"monokai"}"#),
            Some("/home/demo"),
        )
        .unwrap();
        assert_eq!(cfg.runtime.git, Some(PathBuf::from("git-wrapper")));
        assert_eq!(
            cfg.runtime.update_check_store,
            Some(PathBuf::from("/home/demo/u.json"))
        );
        assert_eq!(cfg.runtime.comment_store, Some(PathBuf::from("/c.json")));
        assert_eq!(cfg.runtime.viewed_store, Some(PathBuf::from("/v.json")));
        assert_eq!(cfg.runtime.theme, Some(ThemeId::Monokai));
    }
}
