//! Fetch / pull / default-branch ops. Progress strings go to the caller.

use std::path::Path;

use crate::git::{
    checkout_branch, exec_git, exec_git_checked, get_default_branch, pull_quiet, pull_quiet_detailed,
    repo_has_local_changes,
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
            lines.push("    ⚠️ Failed (may have conflicts)".to_string());
        }
    }
    lines
}

pub fn switch_repo_to_default_branch(
    repo_path: &str,
    current_branch: &str,
    cwd: &Path,
    override_name: Option<&str>,
) -> (bool, Vec<String>) {
    let repo_dir = cwd.join(repo_path);
    let mut lines = Vec::new();
    let Some(default_branch) = get_default_branch(&repo_dir, override_name) else {
        lines.push(format!(
            "  ⚠️ {repo_path}: No default branch found (develop/main/master)"
        ));
        return (false, lines);
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
            lines.push("    ⚠️ Pull failed or no updates".to_string());
        }
        return (false, lines);
    }

    if repo_has_local_changes(&repo_dir) {
        lines.push(format!(
            "  ⚠️ {repo_path} ({current_branch}): Has uncommitted changes, skipping"
        ));
        return (false, lines);
    }

    lines.push(format!(
        "  🔄 {repo_path}: Switching from {current_branch} to {default_branch}"
    ));
    let _ = exec_git_checked(&["fetch", "--quiet", "origin", &default_branch], &repo_dir);
    if !checkout_branch(&default_branch, &repo_dir) {
        lines.push("    ⚠️ Failed to switch (branch may not exist)".to_string());
        return (false, lines);
    }
    lines.push("    ✅ Switched successfully".to_string());
    lines.push("    Pulling latest...".to_string());
    let local = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    let remote = exec_git(&["rev-parse", &format!("origin/{default_branch}")], &repo_dir);
    if local != remote {
        if pull_quiet(&repo_dir) {
            lines.push("    ✅ Pulled successfully".to_string());
        } else {
            lines.push("    ⚠️ Pull failed".to_string());
        }
    } else {
        lines.push("    ✅ Already up to date".to_string());
    }
    (true, lines)
}
