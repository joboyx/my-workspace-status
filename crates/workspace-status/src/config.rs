//! Workspace config from `.workspace-status-config.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::helpers::normalize_filter_repo;
use serde::Deserialize;

pub const CONFIG_FILENAME: &str = ".workspace-status-config.json";
pub const DEFAULT_MAX_DEPTH: u32 = 3;

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
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    ignored_repos: Vec<serde_json::Value>,
    max_depth: Option<serde_json::Value>,
    default_branches: Option<serde_json::Value>,
    editor: Option<serde_json::Value>,
    diff_tool: Option<serde_json::Value>,
    view_defaults: Option<serde_json::Value>,
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
fn parse_view_choice(
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
            "{CONFIG_FILENAME} viewDefaults.{key} must be \"{on}\" or \"{off}\""
        )),
    }
}

fn parse_view_defaults(value: Option<serde_json::Value>) -> Result<ViewDefaults, String> {
    let Some(v) = value else {
        return Ok(ViewDefaults::default());
    };
    let Some(obj) = v.as_object() else {
        return Err(format!("{CONFIG_FILENAME} viewDefaults must be an object"));
    };
    const KEYS: [&str; 6] = [
        "tree",
        "commitTree",
        "diff",
        "wrap",
        "commitMessage",
        "lineBlame",
    ];
    if let Some(key) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!(
            "{CONFIG_FILENAME} viewDefaults has unknown key \"{key}\""
        ));
    }
    Ok(ViewDefaults {
        tree: parse_view_choice(obj, "tree", "tree", "flat")?,
        commit_tree: parse_view_choice(obj, "commitTree", "tree", "flat")?,
        diff_split: parse_view_choice(obj, "diff", "split", "inline")?,
        wrap: parse_view_choice(obj, "wrap", "wrap", "unwrap")?,
        commit_message_expand: parse_view_choice(obj, "commitMessage", "expand", "collapse")?,
        line_blame: parse_view_choice(obj, "lineBlame", "show", "hide")?,
    })
}

/// Load workspace-status config. Missing file means empty ignore and maxDepth 3.
pub fn load_workspace_status_config(cwd: &Path) -> Result<WorkspaceStatusConfig, String> {
    let path = cwd.join(CONFIG_FILENAME);
    if !path.exists() {
        return Ok(WorkspaceStatusConfig::with_defaults());
    }
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let parsed: RawConfig = serde_json::from_str(&text)
        .map_err(|_| format!("{CONFIG_FILENAME} must contain an ignoredRepos string array"))?;

    let mut ignored = Vec::new();
    for repo in parsed.ignored_repos {
        let Some(s) = repo.as_str() else {
            return Err(format!(
                "{CONFIG_FILENAME} ignoredRepos must contain only strings"
            ));
        };
        ignored.push(s.to_string());
    }

    let max_depth = match parsed.max_depth {
        None => DEFAULT_MAX_DEPTH,
        Some(v) => {
            let Some(n) = v.as_u64() else {
                return Err(format!(
                    "{CONFIG_FILENAME} maxDepth must be a positive integer"
                ));
            };
            if n < 1 {
                return Err(format!(
                    "{CONFIG_FILENAME} maxDepth must be a positive integer"
                ));
            }
            n as u32
        }
    };

    let default_branches = match parsed.default_branches {
        None => BTreeMap::new(),
        Some(v) => {
            let Some(obj) = v.as_object() else {
                return Err(format!(
                    "{CONFIG_FILENAME} defaultBranches must be an object"
                ));
            };
            let mut map = BTreeMap::new();
            for (repo, branch) in obj {
                let Some(branch) = branch.as_str() else {
                    return Err(format!(
                        "{CONFIG_FILENAME} defaultBranches values must be strings (key: {repo})"
                    ));
                };
                map.insert(repo.clone(), branch.to_string());
            }
            normalize_default_branches(&map)
        }
    };

    let editor = match parsed.editor {
        None => None,
        Some(v) => {
            let Some(s) = v.as_str() else {
                return Err(format!("{CONFIG_FILENAME} editor must be a string"));
            };
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    };

    let diff_tool = match parsed.diff_tool {
        None => None,
        Some(v) => {
            let Some(s) = v.as_str() else {
                return Err(format!("{CONFIG_FILENAME} diffTool must be a string"));
            };
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    };

    let view_defaults = parse_view_defaults(parsed.view_defaults)?;

    Ok(WorkspaceStatusConfig {
        ignored_repos: normalize_ignored(&ignored),
        max_depth,
        default_branches,
        editor,
        diff_tool,
        view_defaults,
    })
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
        let cfg = load_workspace_status_config(&dir).unwrap();
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
        let cfg = load_workspace_status_config(&dir).unwrap();
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
        let cfg = load_workspace_status_config(&dir).unwrap();
        assert_eq!(cfg.diff_tool, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_tool_vimdiff_is_kept() {
        let dir = write_config(
            "ws-config-diff-vim",
            r#"{"ignoredRepos":[],"diffTool":"vimdiff"}"#,
        );
        let cfg = load_workspace_status_config(&dir).unwrap();
        assert_eq!(cfg.diff_tool.as_deref(), Some("vimdiff"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blank_diff_tool_is_none() {
        let dir = write_config(
            "ws-config-diff-blank",
            r#"{"ignoredRepos":[],"diffTool":"  "}"#,
        );
        let cfg = load_workspace_status_config(&dir).unwrap();
        assert_eq!(cfg.diff_tool, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_string_diff_tool_is_error() {
        let dir = write_config("ws-config-diff-bad", r#"{"ignoredRepos":[],"diffTool":1}"#);
        let err = load_workspace_status_config(&dir).unwrap_err();
        assert!(
            err.contains("diffTool must be a string"),
            "unexpected error: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn load_view_defaults(json: &str) -> Result<ViewDefaults, String> {
        let dir = write_config("ws-config-view", json);
        let out = load_workspace_status_config(&dir).map(|cfg| cfg.view_defaults);
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
}
