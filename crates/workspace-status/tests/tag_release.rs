//! Behaviour tests for `scripts/tag-release.sh` (the `tag-release.yml` step).
//!
//! Each test runs the real script with real `git` in a clone shaped like this
//! repository (`crates/workspace-status/Cargo.toml`,
//! `crates/workspace-status-graph/Cargo.toml`, and a `Cargo.lock` with both
//! packages). Its `origin` is a temp bare repository, never the real remote.
//! The asserts read the refs on that bare `origin`:
//!
//! - A taken `v<version>` tag, also one on a commit that is on no branch, is
//!   skipped and left as it is.
//! - A free version is tagged on HEAD with no bump commit.
//! - When `origin/main` moved past the checked-out commit, the push of `main`
//!   and the tag fails as one unit and `origin` does not change.
//! - The two skip rules exit 0, print no version, and push nothing.
//!
//! Unix only: the script needs bash.
#![cfg(unix)]

/// The temp-repo git helpers of the PTY suite (`tests/common/seed.rs`).
/// Only part of `seed` is used here.
#[allow(dead_code)]
mod common {
    pub mod hscroll;
    pub mod seed;
}

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use common::seed::{git, git_env, git_stdout, unique_root};

/// Version in both Cargo.toml files when a fixture starts.
const START_VERSION: &str = "0.1.290";

/// `crates/workspace-status/Cargo.toml` at `version`.
fn status_manifest(version: &str) -> String {
    format!(
        "[package]\nname = \"workspace-status\"\nversion = \"{version}\"\nedition = \"2021\"\n\n\
         [dependencies]\nworkspace-status-graph = {{ path = \"../workspace-status-graph\" }}\n"
    )
}

/// `crates/workspace-status-graph/Cargo.toml` at `version`.
fn graph_manifest(version: &str) -> String {
    format!("[package]\nname = \"workspace-status-graph\"\nversion = \"{version}\"\nedition = \"2021\"\n")
}

/// `Cargo.lock` with the two workspace packages at `version`. The third-party
/// package has the start version too, so a bump that rewrites every matching
/// line (not only the workspace packages) shows in a full-file compare.
fn cargo_lock(version: &str) -> String {
    format!(
        "version = 4\n\n\
         [[package]]\nname = \"other-dep\"\nversion = \"{START_VERSION}\"\n\n\
         [[package]]\nname = \"workspace-status\"\nversion = \"{version}\"\n\
         dependencies = [\n \"other-dep\",\n \"workspace-status-graph\",\n]\n\n\
         [[package]]\nname = \"workspace-status-graph\"\nversion = \"{version}\"\n"
    )
}

/// Bare `origin`, a `seed` checkout that plays the other pushers (earlier
/// runs, newer merges), and the `clone` the script runs in (the CI checkout).
struct Fixture {
    root: PathBuf,
    origin: PathBuf,
    seed: PathBuf,
    clone: PathBuf,
}

impl Fixture {
    /// One commit on `main` at [`START_VERSION`], pushed to `origin` and
    /// cloned. No tags yet.
    fn new(prefix: &str) -> Self {
        let root = unique_root(prefix);
        let seed = root.join("seed");
        let origin = root.join("origin.git");
        let clone = root.join("clone");
        fs::create_dir_all(&seed).unwrap();
        git(&root, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        git(&seed, &["init", "-q", "-b", "main"]);
        let fixture = Self {
            root,
            origin,
            seed,
            clone,
        };
        fixture.write_manifests(START_VERSION);
        fixture.commit("feat: first change");
        git(
            &fixture.seed,
            &["remote", "add", "origin", fixture.origin.to_str().unwrap()],
        );
        fixture.push(&["main"]);
        git(
            &fixture.root,
            &[
                "clone",
                "-q",
                "-b",
                "main",
                fixture.origin.to_str().unwrap(),
                "clone",
            ],
        );
        fixture
    }

    fn write_manifests(&self, version: &str) {
        let graph_dir = self.seed.join("crates/workspace-status-graph");
        let status_dir = self.seed.join("crates/workspace-status");
        fs::create_dir_all(&graph_dir).unwrap();
        fs::create_dir_all(&status_dir).unwrap();
        fs::write(status_dir.join("Cargo.toml"), status_manifest(version)).unwrap();
        fs::write(graph_dir.join("Cargo.toml"), graph_manifest(version)).unwrap();
        fs::write(self.seed.join("Cargo.lock"), cargo_lock(version)).unwrap();
    }

    /// Commit a change in `seed` with `subject`; returns the commit id.
    fn commit(&self, subject: &str) -> String {
        fs::write(self.seed.join("CHANGES.txt"), subject).unwrap();
        git(&self.seed, &["add", "-A"]);
        git(&self.seed, &["commit", "-q", "-m", subject]);
        git_stdout(&self.seed, &["rev-parse", "HEAD"])
    }

    /// Commit a lockstep version change in `seed`; returns the commit id.
    fn commit_version(&self, subject: &str, version: &str) -> String {
        self.write_manifests(version);
        self.commit(subject)
    }

    /// Annotated tag on the `seed` HEAD.
    fn tag(&self, name: &str) {
        git(&self.seed, &["tag", "-a", name, "-m", name]);
    }

    fn push(&self, refs: &[&str]) {
        let mut args = vec!["push", "-q", "origin"];
        args.extend_from_slice(refs);
        git(&self.seed, &args);
    }

    /// Move the `clone` to `origin/main` the way a CI checkout of the pushed
    /// commit does, without fetching tags, so the script must fetch them.
    fn checkout_origin_main(&self) {
        git(&self.clone, &["fetch", "-q", "--no-tags", "origin"]);
        git(&self.clone, &["reset", "-q", "--hard", "origin/main"]);
    }

    /// Run `scripts/tag-release.sh` in `clone`.
    fn run_script(&self) -> Output {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/tag-release.sh");
        let mut cmd = Command::new("bash");
        cmd.arg(script)
            .current_dir(&self.clone)
            .env_remove("GITHUB_OUTPUT")
            .stdin(Stdio::null());
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        cmd.output().expect("bash runs scripts/tag-release.sh")
    }

    /// Every ref on `origin` with the object it names.
    fn origin_refs(&self) -> BTreeMap<String, String> {
        git_stdout(
            &self.origin,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(name, id)| (name.to_string(), id.to_string()))
        .collect()
    }

    /// Commit id that `rev` names on `origin`.
    fn origin_commit(&self, rev: &str) -> String {
        git_stdout(&self.origin, &["rev-parse", &format!("{rev}^{{commit}}")])
    }

    /// File at `path` in commit `rev` on `origin`, trailing newline trimmed.
    fn origin_file(&self, rev: &str, path: &str) -> String {
        git_stdout(&self.origin, &["show", &format!("{rev}:{path}")])
    }

    /// Branches on `origin` that contain `commit`.
    fn origin_branches_with(&self, commit: &str) -> String {
        git_stdout(&self.origin, &["branch", "--contains", commit])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn describe(out: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn skips_taken_tags_including_an_orphan_and_bumps_in_lockstep() {
    let f = Fixture::new("tag-release-orphan");
    f.tag("v0.1.290");
    f.push(&["v0.1.290"]);
    // An earlier run pushed its tag but not its commit to main.
    git(&f.seed, &["checkout", "-q", "--detach"]);
    let orphan = f.commit_version("chore(release): v0.1.291", "0.1.291");
    f.tag("v0.1.291");
    f.push(&["v0.1.291"]);
    git(&f.seed, &["checkout", "-q", "main"]);
    let merge = f.commit("feat: land a pull request");
    f.push(&["main"]);
    f.checkout_origin_main();
    let orphan_tag_before = f.origin_refs()["refs/tags/v0.1.291"].clone();
    assert_eq!(f.origin_branches_with(&orphan), "");

    let out = f.run_script();

    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(stdout(&out), "0.1.292", "{}", describe(&out));
    let release = f.origin_commit("refs/heads/main");
    assert_ne!(release, merge);
    assert_eq!(f.origin_commit("refs/heads/main^"), merge);
    assert_eq!(
        git_stdout(&f.origin, &["log", "-1", "--format=%s", &release]),
        "chore(release): v0.1.292"
    );
    assert_eq!(f.origin_commit("refs/tags/v0.1.292"), release);
    assert_eq!(
        f.origin_file(&release, "crates/workspace-status/Cargo.toml"),
        status_manifest("0.1.292").trim_end()
    );
    assert_eq!(
        f.origin_file(&release, "crates/workspace-status-graph/Cargo.toml"),
        graph_manifest("0.1.292").trim_end()
    );
    assert_eq!(
        f.origin_file(&release, "Cargo.lock"),
        cargo_lock("0.1.292").trim_end()
    );
    let after = f.origin_refs();
    assert_eq!(after["refs/tags/v0.1.291"], orphan_tag_before);
    assert_eq!(f.origin_commit("refs/tags/v0.1.291"), orphan);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        [
            "refs/heads/main",
            "refs/tags/v0.1.290",
            "refs/tags/v0.1.291",
            "refs/tags/v0.1.292"
        ]
    );
}

#[test]
fn tags_a_free_version_on_head_without_a_bump_commit() {
    let f = Fixture::new("tag-release-free");
    f.tag("v0.1.290");
    f.push(&["v0.1.290"]);
    let bump = f.commit_version("chore: start the 0.1.291 line", "0.1.291");
    f.push(&["main"]);
    f.checkout_origin_main();

    let out = f.run_script();

    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(stdout(&out), "0.1.291", "{}", describe(&out));
    assert_eq!(f.origin_commit("refs/heads/main"), bump);
    assert_eq!(f.origin_commit("refs/tags/v0.1.291"), bump);
}

#[test]
fn refused_main_push_leaves_no_tag_on_origin() {
    let f = Fixture::new("tag-release-race");
    f.tag("v0.1.290");
    f.push(&["v0.1.290"]);
    f.commit("feat: older merge");
    f.push(&["main"]);
    f.checkout_origin_main();
    // A newer merge lands while the run for the older one is working.
    let newer = f.commit("feat: newer merge");
    f.push(&["main"]);
    let before = f.origin_refs();

    let out = f.run_script();

    assert!(!out.status.success(), "{}", describe(&out));
    assert_eq!(f.origin_refs(), before, "{}", describe(&out));
    assert_eq!(f.origin_commit("refs/heads/main"), newer);
    assert_eq!(stdout(&out), "", "{}", describe(&out));
}

#[test]
fn skips_an_incoming_release_commit_and_pushes_nothing() {
    let f = Fixture::new("tag-release-skip-subject");
    f.commit_version("chore(release): v0.1.291", "0.1.291");
    f.push(&["main"]);
    f.checkout_origin_main();
    let before = f.origin_refs();

    let out = f.run_script();

    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(stdout(&out), "", "{}", describe(&out));
    assert_eq!(f.origin_refs(), before);
}

#[test]
fn skips_a_head_that_already_has_a_release_tag_and_pushes_nothing() {
    let f = Fixture::new("tag-release-skip-tagged");
    f.tag("v0.1.290");
    f.push(&["v0.1.290"]);
    f.checkout_origin_main();
    let before = f.origin_refs();

    let out = f.run_script();

    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(stdout(&out), "", "{}", describe(&out));
    assert_eq!(f.origin_refs(), before);
}
