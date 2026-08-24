//! Smoke check for `scripts/seed-demo-workspace.sh`.
//!
//! Seeds a temp dir, then asserts git state and `--plain` / `--json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_workspace-status"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_AUTHOR_NAME", "workspace-status e2e"),
        ("GIT_AUTHOR_EMAIL", "workspace-status-e2e@example.invalid"),
        ("GIT_COMMITTER_NAME", "workspace-status e2e"),
        (
            "GIT_COMMITTER_EMAIL",
            "workspace-status-e2e@example.invalid",
        ),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ]
}

fn git(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(["-C"]).arg(repo).args(args);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn run_status(workspace: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(bin());
    cmd.args(args)
        .current_dir(workspace)
        .env("TERM", "dumb")
        .stdin(Stdio::null());
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.output().expect("workspace-status runs")
}

fn seed_dest() -> PathBuf {
    std::env::temp_dir().join(format!(
        "ws-demo-seed-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn seed_workspace(dest: &Path) {
    let script = repo_root().join("scripts/seed-demo-workspace.sh");
    let status = Command::new("bash")
        .arg(&script)
        .arg(dest)
        .status()
        .expect("seed script runs");
    assert!(status.success(), "seed-demo-workspace.sh failed");
}

#[test]
fn seed_demo_workspace_state_and_snapshot() {
    let dest = seed_dest();
    seed_workspace(&dest);

    let app = dest.join("app");
    assert_eq!(
        git(&app, &["branch", "--show-current"]),
        "feature/auth-refresh"
    );
    assert!(git(&app, &["status", "--short", "--branch"]).contains("ahead 1"));
    let app_porcelain = git(&app, &["status", "--porcelain"]);
    assert!(app_porcelain.contains(" M src/auth.ts"));
    assert!(app_porcelain.contains("M  src/session.ts"));
    assert!(app_porcelain.contains("?? src/login.ts"));
    assert!(git(&app, &["stash", "list"]).contains("WIP: in-memory token cache"));
    let worktrees = git(&app, &["worktree", "list"]);
    assert!(worktrees.contains("feat-login"));
    assert!(worktrees.contains("[feature/login-page]"));

    let first = git(&app, &["log", "--reverse", "--format=%at %an <%ae> %s"])
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert_eq!(
        first,
        "1786928400 Demo User <demo@example.invalid> seed app client"
    );
    let auth = fs::read_to_string(app.join("src/auth.ts")).unwrap();
    assert!(auth.contains("withRefreshedExpiry"));

    let api = dest.join("services/api");
    assert_eq!(
        git(&api, &["branch", "--show-current"]),
        "feature/rate-limit"
    );
    assert!(git(&api, &["status", "--short", "--branch"]).contains("ahead 1, behind 1"));
    assert!(git(&api, &["status", "--porcelain"]).contains("src/server.ts"));

    let lib = dest.join("lib");
    assert_eq!(git(&lib, &["branch", "--show-current"]), "main");
    assert_eq!(git(&lib, &["status", "--porcelain"]), "");

    let notes = dest.join("notes");
    assert_eq!(git(&notes, &["branch", "--show-current"]), "main");
    assert!(git(&notes, &["status", "--porcelain"]).contains("inbox.md"));

    let merger = dest.join("merger");
    assert_eq!(
        git(&merger, &["branch", "--show-current"]),
        "feature/reconciliation"
    );
    let parents = git(&merger, &["rev-list", "--parents", "-n", "1", "HEAD^"]);
    assert!(
        parents.split(' ').count() >= 3,
        "expected a merge commit on merger"
    );
    assert!(git(&merger, &["log", "--oneline", "--decorate"]).contains("merge billing into main"));
    assert!(git(&merger, &["stash", "list"]).contains("WIP: reconcile totals"));

    let config: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dest.join(".workspace-status-config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["ignoredRepos"], serde_json::json!(["notes"]));
    assert_eq!(config["editor"], "vim");
    assert!(dest.join(".remotes/app.git").exists());
    assert!(dest.join(".scratch").is_dir());

    let out = run_status(&dest, &["--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["repo"].as_str().unwrap())
        .collect();
    assert!(!names.iter().any(|n| *n == "notes"));
    assert!(names.contains(&"app"));
    assert!(names.contains(&"app/.worktrees/feat-login"));
    assert!(names.contains(&"services/api"));
    assert!(names.contains(&"lib"));
    assert!(names.contains(&"merger"));

    let lib = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "lib")
        .unwrap();
    assert_eq!(lib["hasUnstaged"], false);
    assert_eq!(lib["hasStaged"], false);
    assert_eq!(lib["hasUntracked"], false);
    assert_eq!(lib["branch"], "main");
    assert_eq!(lib["syncStatus"], "up-to-date");

    let app = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "app")
        .unwrap();
    assert_eq!(app["branch"], "feature/auth-refresh");
    assert_eq!(app["hasUnstaged"], true);
    assert_eq!(app["hasStaged"], true);
    assert_eq!(app["hasUntracked"], true);
    assert_eq!(app["syncStatus"], "ahead");
    let app_files: Vec<&str> = app["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(app_files.contains(&"src/auth.ts"));

    let api = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "services/api")
        .unwrap();
    assert_eq!(api["branch"], "feature/rate-limit");
    assert_eq!(api["hasUnstaged"], true);
    assert_eq!(api["syncStatus"], "diverged");

    let merger = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "merger")
        .unwrap();
    assert_eq!(merger["branch"], "feature/reconciliation");

    let plain = run_status(&dest, &["--plain"]);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    let plain_text = String::from_utf8_lossy(&plain.stdout);
    assert!(plain_text.contains("app"));
    assert!(plain_text.contains("auth.ts"));
    assert!(plain_text.contains("feat-login"));
    assert!(plain_text.contains("services/api"));
    assert!(plain_text.contains("merger"));
    assert!(plain_text.contains("feature/auth-refresh"));
    assert!(plain_text.contains("feature/reconciliation"));
    assert!(!plain_text.split_whitespace().any(|w| w == "notes"));

    let all = run_status(&dest, &["--json", "--all"]);
    assert!(
        all.status.success(),
        "{}",
        String::from_utf8_lossy(&all.stderr)
    );
    let snapshot_all: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    let notes = snapshot_all["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "notes")
        .expect("notes present with --all");
    assert_eq!(notes["ignored"], true);
    assert_eq!(notes["hasUnstaged"], true);
    let plain_all = run_status(&dest, &["--plain", "--all"]);
    let plain_all_text = String::from_utf8_lossy(&plain_all.stdout);
    assert!(plain_all_text.contains("notes"));
    assert!(plain_all_text.contains("inbox.md"));

    let _ = fs::remove_dir_all(&dest);
}
