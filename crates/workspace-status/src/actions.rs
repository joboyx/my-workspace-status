//! Fetch / pull / default-branch ops. Progress strings go to the caller.

use std::path::Path;

use crate::git::{
    checkout_branch_detailed, exec_git, exec_git_checked, first_error_line, get_default_branch,
    pull_quiet_detailed, repo_has_local_changes, rev_parse_quiet,
};

pub fn pull_behind_repos(cwd: &Path, repos: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    for repo in repos {
        let result = pull_quiet_detailed(&cwd.join(repo));
        lines.push(format!("  Pulling {repo}..."));
        if result.ok {
            if result.stashed {
                lines.push("    ✅ Success (stashed local changes, reapplied)".to_string());
            } else {
                lines.push("    ✅ Success".to_string());
            }
        } else if result.stash_pop_failed {
            lines.push(
                "    ⚠️ Pulled but stash pop conflicted — resolve conflicts / check stash list"
                    .to_string(),
            );
        } else {
            lines.push(failure_line("Failed", result.error.as_deref()));
        }
    }
    lines
}

/// Outcome of [`switch_repo_to_default_branch`] for one repo.
pub struct DefaultBranchSwitch {
    /// True when HEAD moved to the default branch.
    pub switched: bool,
    /// First git error on the way (switch or the pull after it). `None` on success.
    pub error: Option<String>,
    /// Human progress lines for the plain report.
    pub lines: Vec<String>,
}

pub fn switch_repo_to_default_branch(
    repo_path: &str,
    current_branch: &str,
    cwd: &Path,
    override_name: Option<&str>,
) -> DefaultBranchSwitch {
    let repo_dir = cwd.join(repo_path);
    let mut lines = Vec::new();
    let Some(default_branch) = get_default_branch(&repo_dir, override_name) else {
        lines.push(format!(
            "  ⚠️ {repo_path}: No default branch found (develop/main/master)"
        ));
        return DefaultBranchSwitch {
            switched: false,
            error: Some("no default branch found (develop/main/master)".to_string()),
            lines,
        };
    };

    if current_branch == default_branch {
        lines.push(format!("  ✅ {repo_path}: Already on {default_branch}"));
        lines.push("    Pulling latest...".to_string());
        let result = pull_quiet_detailed(&repo_dir);
        if result.ok {
            if result.stashed {
                lines.push("    ✅ Pulled successfully (stashed local changes, reapplied)".to_string());
            } else {
                lines.push("    ✅ Pulled successfully".to_string());
            }
        } else if result.stash_pop_failed {
            lines.push(
                "    ⚠️ Pulled but stash pop conflicted — resolve conflicts / check stash list"
                    .to_string(),
            );
        } else {
            lines.push(failure_line("Pull failed", result.error.as_deref()));
        }
        return DefaultBranchSwitch {
            switched: false,
            error: result.error,
            lines,
        };
    }

    if repo_has_local_changes(&repo_dir) {
        lines.push(format!(
            "  ⚠️ {repo_path} ({current_branch}): Has uncommitted changes, skipping"
        ));
        return DefaultBranchSwitch {
            switched: false,
            error: Some("has uncommitted changes, skipped".to_string()),
            lines,
        };
    }

    lines.push(format!(
        "  🔄 {repo_path}: Switching from {current_branch} to {default_branch}"
    ));
    let _ = exec_git_checked(&["fetch", "--quiet", "origin", &default_branch], &repo_dir);
    if let Err(err) = checkout_branch_detailed(&default_branch, &repo_dir) {
        lines.push(failure_line("Failed to switch", Some(&err)));
        return DefaultBranchSwitch {
            switched: false,
            error: Some(err),
            lines,
        };
    }
    lines.push("    ✅ Switched successfully".to_string());
    lines.push("    Pulling latest...".to_string());
    let local = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    let remote = rev_parse_quiet(&format!("origin/{default_branch}"), &repo_dir);
    let mut error = None;
    // No `origin/<default>` means nothing to pull; that is not a failed switch.
    if remote.is_none() {
        lines.push("    ℹ️ No remote default branch, skipping pull".to_string());
    } else if remote.as_deref() != Some(local.as_str()) {
        let result = pull_quiet_detailed(&repo_dir);
        if result.ok {
            lines.push("    ✅ Pulled successfully".to_string());
        } else {
            lines.push(failure_line("Pull failed", result.error.as_deref()));
            error = result.error;
        }
    } else {
        lines.push("    ✅ Already up to date".to_string());
    }
    DefaultBranchSwitch {
        switched: true,
        error,
        lines,
    }
}

/// `    ⚠️ <label>: <first git error line>`, or just the label when git said nothing.
fn failure_line(label: &str, error: Option<&str>) -> String {
    match error.map(first_error_line).filter(|line| !line.is_empty()) {
        Some(line) => format!("    ⚠️ {label}: {line}"),
        None => format!("    ⚠️ {label}"),
    }
}
