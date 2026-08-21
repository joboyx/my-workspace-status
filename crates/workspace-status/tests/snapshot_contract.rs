//! Fixture e2e: temp workspace, Rust --json shape and --plain omit hidden ignored.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_workspace-status"))
}

fn ws_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ws"))
}

fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_AUTHOR_NAME", "workspace-status e2e"),
        ("GIT_AUTHOR_EMAIL", "workspace-status-e2e@example.invalid"),
        ("GIT_COMMITTER_NAME", "workspace-status e2e"),
        ("GIT_COMMITTER_EMAIL", "workspace-status-e2e@example.invalid"),
    ]
}

fn git(cwd: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

fn seed_repo(workspace: &Path, name: &str, branch: &str, dirty: bool) {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", branch])
        .current_dir(&repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(&repo, &["init", "-q"]);
        git(&repo, &["checkout", "-q", "-b", branch]);
    }
    fs::write(repo.join("README.md"), format!("# {name}\n")).unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", &format!("seed {name}")]);
    if dirty {
        fs::write(repo.join("README.md"), format!("# {name}\ndirty\n")).unwrap();
    }
}

fn fixture() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "workspace-status-rs-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    seed_repo(&workspace, "lib", "main", false);
    seed_repo(&workspace, "notes", "main", true);
    fs::write(
        workspace.join(".workspace-status-config.json"),
        "{\n  \"ignoredRepos\": [\"notes\"]\n}\n",
    )
    .unwrap();
    (root, workspace)
}

fn run_json(workspace: &Path, args: &[&str]) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.args(args).current_dir(workspace).env("TERM", "dumb");
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run binary");
    assert!(
        out.status.success(),
        "command failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    serde_json::from_slice(&out.stdout).expect("json stdout")
}

fn run_plain(workspace: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new(bin());
    cmd.args(args).current_dir(workspace).env("TERM", "dumb");
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run binary");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn json_and_plain_match_snapshot_fixture() {
    let (root, workspace) = fixture();
    let snapshot = run_json(&workspace, &["--json"]);
    let plain = run_plain(&workspace, &["--plain"]);

    assert_eq!(snapshot["version"], 1);
    assert_eq!(snapshot["showIgnored"], false);
    assert_eq!(snapshot["filterRepos"], serde_json::json!([]));
    assert_eq!(snapshot["ignoredRepos"], serde_json::json!(["notes"]));
    let repos: Vec<&str> = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["repo"].as_str().unwrap())
        .collect();
    assert_eq!(repos, vec!["app", "lib"]);

    let app = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "app")
        .unwrap();
    assert_eq!(app["ignored"], false);
    assert_eq!(app["branch"], "main");
    assert_eq!(app["syncStatus"], "no-upstream");
    assert_eq!(app["checkoutKind"], "primary");
    assert_eq!(app["hasUnstaged"], true);
    assert_eq!(
        app["changes"],
        serde_json::json!([{ "path": "README.md", "unstagedStatus": "M" }])
    );

    let lib = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "lib")
        .unwrap();
    assert_eq!(lib["hasUnstaged"], false);
    assert_eq!(lib["changes"], serde_json::json!([]));

    assert!(!repos.contains(&"notes"));
    assert!(plain.contains("File changes"));
    assert!(plain.contains("app"));
    assert!(plain.contains("README.md"));
    assert!(!plain.contains("notes"));
    assert!(!String::from_utf8_lossy(
        &Command::new(bin())
            .args(["--json"])
            .current_dir(&workspace)
            .output()
            .unwrap()
            .stdout
    )
    .contains("🔄"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn json_all_and_plain_all_include_ignored() {
    let (root, workspace) = fixture();
    let snapshot = run_json(&workspace, &["--json", "--all"]);
    let notes = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "notes")
        .expect("notes present");
    assert_eq!(notes["ignored"], true);
    assert_eq!(snapshot["showIgnored"], true);
    assert_eq!(notes["hasUnstaged"], true);

    let plain = run_plain(&workspace, &["--plain", "--all"]);
    assert!(plain.contains("notes"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn json_fetch_stdout_stays_parseable() {
    let (root, workspace) = fixture();
    let mut cmd = Command::new(bin());
    cmd.args(["--json", "--fetch"])
        .current_dir(&workspace)
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let snapshot: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(snapshot["version"], 1);
    let repos: Vec<&str> = snapshot["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["repo"].as_str().unwrap())
        .collect();
    assert_eq!(repos, vec!["app", "lib"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn json_wins_over_plain() {
    let (root, workspace) = fixture();
    let snapshot = run_json(&workspace, &["--json", "--plain"]);
    assert_eq!(snapshot["version"], 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn named_filter_includes_ignored_and_unknown_exits() {
    let (root, workspace) = fixture();
    let snapshot = run_json(&workspace, &["--json", "notes"]);
    assert_eq!(snapshot["filterRepos"], serde_json::json!(["notes"]));
    assert_eq!(snapshot["repos"][0]["repo"], "notes");
    assert_eq!(snapshot["repos"][0]["ignored"], true);

    let mut cmd = Command::new(bin());
    cmd.args(["--json", "missing-repo"]).current_dir(&workspace);
    let out = cmd.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unknown repo: missing-repo"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ws_alias_runs_same_binary() {
    let (root, workspace) = fixture();
    let mut cmd = Command::new(ws_bin());
    cmd.args(["--json"]).current_dir(&workspace).env("TERM", "dumb");
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let snapshot: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(snapshot["version"], 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn default_without_flags_is_plain() {
    let (root, workspace) = fixture();
    let plain = run_plain(&workspace, &[]);
    assert!(plain.contains("File changes"));
    assert!(plain.contains("app"));
    let _ = fs::remove_dir_all(root);
}
