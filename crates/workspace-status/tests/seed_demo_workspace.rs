//! Smoke check for `scripts/seed-demo-workspace.sh`.
//!
//! Seeds a temp dir, then asserts git state and `--plain` / `--json`.
//! The wipe guard tests run a copy of the script inside a fake repository
//! under a temp root, with `HOME` pointed at a temp dir, so a guard bug can
//! only delete temp files. The one exception is the root test (`/`, `//`):
//! there a bug in the root rule still meets two more refusals, because `/`
//! holds the temp `HOME` and is not empty without the seed marker.
//!
//! Unix only: the script needs bash.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[path = "common/fixture.rs"]
mod fixture;

use fixture::{git_env, unique_root};

/// Marker file the seed writes into DEST.
const SEED_MARKER: &str = ".ws-demo-seed";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_workspace-status"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
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
        .env_remove("WS_STATUS_WORKSPACE")
        // Missing dir under the seed: an operator user config cannot change the result.
        .env("XDG_CONFIG_HOME", workspace.join(".xdg-config"))
        .stdin(Stdio::null());
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.output().expect("workspace-status runs")
}

fn seed_dest() -> PathBuf {
    unique_root("ws-demo-seed")
}

/// A temp layout for the wipe guard tests: `home/` (the `HOME` of the run)
/// and `repo/scripts/seed-demo-workspace.sh` (a copy of the real script, so
/// `repo/` is the repository root it sees). Each directory holds a
/// `canary` file that a wipe would delete.
struct GuardSandbox {
    root: PathBuf,
    home: PathBuf,
    repo: PathBuf,
}

impl GuardSandbox {
    fn new() -> Self {
        let root = unique_root("ws-demo-seed-guard");
        let home = root.join("parent/home");
        let repo = root.join("repo");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(repo.join("scripts")).unwrap();
        fs::create_dir_all(repo.join("docs")).unwrap();
        fs::copy(
            repo_root().join("scripts/seed-demo-workspace.sh"),
            repo.join("scripts/seed-demo-workspace.sh"),
        )
        .unwrap();
        for dir in [&root, &home, &repo, &repo.join("docs")] {
            fs::write(dir.join("canary"), "keep\n").unwrap();
        }
        Self { root, home, repo }
    }

    /// Run the copied script with `dest` and `HOME` = `self.home`.
    fn seed(&self, dest: &str) -> std::process::Output {
        Command::new("bash")
            .arg(self.repo.join("scripts/seed-demo-workspace.sh"))
            .arg(dest)
            .env("HOME", &self.home)
            .output()
            .expect("seed script runs")
    }

    /// The script refused `dest` and every canary is still there.
    fn assert_refused(&self, dest: &str) {
        self.assert_refused_because(dest, "");
    }

    /// As `assert_refused`, and the refusal names `reason`.
    fn assert_refused_because(&self, dest: &str, reason: &str) {
        let out = self.seed(dest);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "dest {dest}: {stderr}");
        assert!(stderr.contains("refusing to wipe"), "dest {dest}: {stderr}");
        assert!(stderr.contains(reason), "dest {dest}: {stderr}");
        for dir in [&self.root, &self.home, &self.repo, &self.repo.join("docs")] {
            assert!(
                dir.join("canary").is_file(),
                "dest {dest} deleted {}",
                dir.display()
            );
        }
        assert!(self.repo.join("scripts/seed-demo-workspace.sh").is_file());
    }
}

impl Drop for GuardSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("utf-8 temp path").to_string()
}

#[test]
fn seed_refuses_home_with_trailing_slash() {
    let sandbox = GuardSandbox::new();
    sandbox.assert_refused(&format!("{}/", path_arg(&sandbox.home)));
    sandbox.assert_refused(&format!("{}/sub/..", path_arg(&sandbox.home)));
}

#[test]
fn seed_refuses_an_ancestor_of_home() {
    let sandbox = GuardSandbox::new();
    sandbox.assert_refused(&path_arg(&sandbox.root.join("parent")));
    sandbox.assert_refused(&format!("{}/", path_arg(&sandbox.root)));
}

#[test]
fn seed_refuses_the_filesystem_root() {
    let sandbox = GuardSandbox::new();
    // `pwd -P` in bash keeps a leading `//`: it must still hit the root rule.
    for dest in ["/", "//", "//."] {
        sandbox.assert_refused_because(dest, "it is the filesystem root");
    }
}

#[cfg(unix)]
#[test]
fn seed_refuses_an_unreadable_dir() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = GuardSandbox::new();
    // 0o300: cd works, ls fails. 0o000: cd fails too.
    for (mode, reason) in [
        (0o300, "cannot list it"),
        (0o000, "cannot resolve its physical path"),
    ] {
        let locked = sandbox.root.join(format!("locked-{mode:o}"));
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("keep.txt"), "keep\n").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(mode)).unwrap();
        // As root the mode does not lock the dir: then the non-empty rule refuses.
        let reason = if fs::read_dir(&locked).is_ok() {
            "it is not empty"
        } else {
            reason
        };
        sandbox.assert_refused_because(&path_arg(&locked), reason);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(locked.join("keep.txt").is_file());
    }
}

#[test]
fn seed_refuses_repo_root_with_trailing_slash() {
    let sandbox = GuardSandbox::new();
    sandbox.assert_refused(&format!("{}/", path_arg(&sandbox.repo)));
    sandbox.assert_refused(&path_arg(&sandbox.repo.join("scripts/..")));
}

#[test]
fn seed_refuses_a_repo_path_outside_tmp() {
    let sandbox = GuardSandbox::new();
    sandbox.assert_refused(&path_arg(&sandbox.repo.join("docs")));
    sandbox.assert_refused(&path_arg(&sandbox.repo.join("tmp")));
}

#[test]
fn seed_refuses_a_non_empty_dir_without_marker() {
    let sandbox = GuardSandbox::new();
    let other = sandbox.root.join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("keep.txt"), "keep\n").unwrap();
    sandbox.assert_refused(&path_arg(&other));
    assert!(other.join("keep.txt").is_file());
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
    assert!(
        dest.join("merger/.worktrees/recon").is_dir(),
        "seed must include a linked worktree on the current branch"
    );
    let merger_worktrees = git(&merger, &["worktree", "list"]);
    assert!(
        merger_worktrees.contains(".worktrees/recon"),
        "linked extra on feature/reconciliation: {merger_worktrees}"
    );

    let config: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dest.join(".workspace-status-config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["hiddenPaths"], serde_json::json!(["notes"]));
    assert!(
        config.get("editor").is_none(),
        "editor is user-file only, so the workspace file must not set it"
    );
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
    assert!(!names.contains(&"notes"));
    assert!(names.contains(&"app"));
    assert!(names.contains(&"app/.worktrees/feat-login"));
    assert!(names.contains(&"services/api"));
    assert!(names.contains(&"lib"));
    assert!(names.contains(&"merger"));
    assert!(names.contains(&"merger/.worktrees/recon"));

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
    assert!(plain_text.contains(".worktrees/recon"));
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
    assert_eq!(notes["hidden"], true);
    assert_eq!(notes["hasUnstaged"], true);
    let plain_all = run_status(&dest, &["--plain", "--all"]);
    let plain_all_text = String::from_utf8_lossy(&plain_all.stdout);
    assert!(plain_all_text.contains("notes"));
    assert!(plain_all_text.contains("inbox.md"));

    // The seed marks DEST, so a second run may wipe and reseed it.
    assert!(dest.join(SEED_MARKER).is_file());
    fs::write(dest.join("stale.txt"), "from the first run\n").unwrap();
    seed_workspace(&dest);
    assert!(!dest.join("stale.txt").exists());
    assert!(dest.join(SEED_MARKER).is_file());
    assert_eq!(
        git(&dest.join("app"), &["branch", "--show-current"]),
        "feature/auth-refresh"
    );

    let _ = fs::remove_dir_all(&dest);
}
