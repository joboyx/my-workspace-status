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
}

impl WorkspaceStatusConfig {
    pub fn with_defaults() -> Self {
        Self {
            ignored_repos: Vec::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            default_branches: BTreeMap::new(),
            editor: None,
            diff_tool: None,
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

    Ok(WorkspaceStatusConfig {
        ignored_repos: normalize_ignored(&ignored),
        max_depth,
        default_branches,
        editor,
        diff_tool,
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
        let dir = std::env::temp_dir().join(format!(
            "{dir_prefix}-{}",
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
}
