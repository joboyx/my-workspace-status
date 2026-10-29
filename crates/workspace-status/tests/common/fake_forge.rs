//! Fake `gh` / `glab` for PTY e2e: no network, no login.
//!
//! [`FakeForge`] owns a scenario directory and the child env that puts the
//! scripts of `scripts/fake-forge/` ahead of any real CLI. Tests write what
//! the CLI should print, spawn the TUI with [`FakeForge::env`], and read back
//! what the app called ([`FakeForge::calls`]) and what it opened
//! ([`FakeForge::opened`]). Scenario format: `scripts/fake-forge/README.md`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::seed::{git, git_stdout};

/// GitHub remote of the seeded `acme/app` repository.
pub const GITHUB_ORIGIN: &str = "https://github.com/acme/app.git";

/// GitLab remote of the seeded `acme/services/api` project.
pub const GITLAB_ORIGIN: &str = "https://gitlab.com/acme/services/api.git";

/// What a fake CLI call does.
pub enum Reply<'a> {
    /// Print this text on stdout and exit 0.
    Json(&'a str),
    /// Print `stderr` on stderr and exit `code`, with nothing on stdout.
    Fail {
        /// Exit code of the fake CLI.
        code: i32,
        /// Text for stderr.
        stderr: &'a str,
    },
}

/// A scenario directory plus the env that makes the app use it.
pub struct FakeForge {
    dir: PathBuf,
    browser_log: PathBuf,
    dir_env: String,
    path_env: String,
    browser_env: String,
}

impl FakeForge {
    /// Create the scenario directory and the `$BROWSER` recorder under `root`.
    ///
    /// The recorder appends the URL it gets to a file, one line per open.
    /// `PATH` lists `scripts/fake-forge/` first, then the parent `PATH`.
    pub fn new(root: &Path) -> Self {
        let bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/fake-forge")
            .canonicalize()
            .expect("scripts/fake-forge exists");
        let dir = root.join("fake-forge");
        fs::create_dir_all(&dir).unwrap();

        let browser_dir = root.join("fake-forge-browser");
        fs::create_dir_all(&browser_dir).unwrap();
        let browser_log = browser_dir.join("opened");
        let recorder = browser_dir.join("rec");
        fs::write(
            &recorder,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\n",
                browser_log.display()
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&recorder).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&recorder, perms).unwrap();

        Self {
            dir_env: dir.display().to_string(),
            path_env: format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
            browser_env: format!("{} %s", recorder.display()),
            dir,
            browser_log,
        }
    }

    /// Child env for `PtySession::open_with_env`: `PATH` with the fake
    /// `gh` / `glab` first, `WS_FAKE_FORGE_DIR`, and `BROWSER`.
    pub fn env(&self) -> [(&str, &str); 3] {
        [
            ("PATH", &self.path_env),
            ("WS_FAKE_FORGE_DIR", &self.dir_env),
            ("BROWSER", &self.browser_env),
        ]
    }

    /// Answer for `gh pr list --head <branch>` on GitHub repository
    /// `repo` (`owner/repo`).
    pub fn github_pr_list(&self, repo: &str, branch: &str, reply: Reply<'_>) {
        self.put(
            &format!("gh/{}/pr-list/{}.json", slug(repo), slug(branch)),
            reply,
        );
    }

    /// Answer for `gh pr view <number>` on GitHub repository `repo`.
    pub fn github_pr_view(&self, repo: &str, number: u64, reply: Reply<'_>) {
        self.put(&format!("gh/{}/pr-view/{number}.json", slug(repo)), reply);
    }

    /// Answer for the GitLab merge request list of `branch` in `project`
    /// (`group/sub/project`).
    pub fn gitlab_mr_list(&self, project: &str, branch: &str, reply: Reply<'_>) {
        self.put(
            &format!("glab/{}/mr-list/{}.json", slug(project), slug(branch)),
            reply,
        );
    }

    /// Answer for the GitLab merge request `iid` of `project`.
    pub fn gitlab_mr(&self, project: &str, iid: u64, reply: Reply<'_>) {
        self.put(&format!("glab/{}/mr/{iid}.json", slug(project)), reply);
    }

    /// Answer for the approvals of GitLab merge request `iid` of `project`.
    pub fn gitlab_approvals(&self, project: &str, iid: u64, reply: Reply<'_>) {
        self.put(
            &format!("glab/{}/mr/{iid}-approvals.json", slug(project)),
            reply,
        );
    }

    /// Answer for the jobs of GitLab pipeline `pipeline` of `project`.
    pub fn gitlab_pipeline_jobs(&self, project: &str, pipeline: u64, reply: Reply<'_>) {
        self.put(
            &format!("glab/{}/pipeline-jobs/{pipeline}.json", slug(project)),
            reply,
        );
    }

    /// Argv line of every call so far, program first, oldest first.
    pub fn calls(&self) -> Vec<String> {
        lines(&self.dir.join("calls.log"))
    }

    /// URL of every `$BROWSER` open so far, oldest first.
    pub fn opened(&self) -> Vec<String> {
        lines(&self.browser_log)
    }

    /// Write or replace one scenario. A running app sees the old answer or
    /// the new one, never half a file: each file is written under a temporary
    /// name and renamed.
    fn put(&self, rel: &str, reply: Reply<'_>) {
        let file = self.dir.join(rel);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let exit = sibling(&file, "exit");
        let stderr = sibling(&file, "stderr");
        match reply {
            Reply::Json(body) => {
                write_atomic(&file, body);
                let _ = fs::remove_file(&exit);
                let _ = fs::remove_file(&stderr);
            }
            Reply::Fail { code, stderr: text } => {
                write_atomic(&stderr, text);
                write_atomic(&exit, &code.to_string());
                let _ = fs::remove_file(&file);
            }
        }
    }
}

/// Point `origin` of `repo` at `url`, adding the remote when it has none.
///
/// Only `git remote` runs, so nothing touches the network.
pub fn set_origin(repo: &Path, url: &str) {
    let has_origin = git_stdout(repo, &["remote"])
        .lines()
        .any(|name| name == "origin");
    let verb = if has_origin { "set-url" } else { "add" };
    git(repo, &["remote", verb, "origin", url]);
}

/// File-name form of a branch or project path: `/` becomes `__`.
fn slug(text: &str) -> String {
    text.replace('/', "__")
}

fn sibling(file: &Path, extension: &str) -> PathBuf {
    let mut name = file.file_name().unwrap().to_os_string();
    name.push(".");
    name.push(extension);
    file.with_file_name(name)
}

fn write_atomic(file: &Path, body: &str) {
    let mut name = std::ffi::OsString::from(".");
    name.push(file.file_name().unwrap());
    name.push(".tmp");
    let tmp = file.with_file_name(name);
    fs::write(&tmp, body).unwrap();
    fs::rename(&tmp, file).unwrap();
}

fn lines(file: &Path) -> Vec<String> {
    fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::super::seed::unique_root;
    use super::*;

    /// Run `program` with the fake forge env. Returns exit code, stdout, stderr.
    fn run(forge: &FakeForge, program: &str, args: &[&str]) -> (i32, String, String) {
        let mut cmd = Command::new(program);
        cmd.args(args);
        for (key, value) in forge.env() {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("fake forge script runs");
        (
            out.status.code().expect("exit code"),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    #[test]
    fn fake_forge_answers_by_flags_and_endpoint() {
        let root = unique_root("ws-fake-forge");
        let forge = FakeForge::new(&root);
        let project = "acme/services/api";
        forge.github_pr_list("acme/app", "feature/x", Reply::Json("[gh-list]"));
        forge.github_pr_view("acme/app", 7, Reply::Json("{gh-view}"));
        forge.gitlab_mr_list(project, "feature/x", Reply::Json("[glab-list]"));
        forge.gitlab_mr(project, 3, Reply::Json("{glab-mr}"));
        forge.gitlab_approvals(project, 3, Reply::Json("{glab-approvals}"));
        forge.gitlab_pipeline_jobs(project, 99, Reply::Json("[glab-jobs]"));

        // GitHub: the host in `-R` is optional, and the `--json` fields,
        // `--state` and `--limit` do not matter.
        let list = [
            "pr",
            "list",
            "-R",
            "github.com/acme/app",
            "--head",
            "feature/x",
        ];
        let (code, out, _) = run(&forge, "gh", &[&list[..], &["--json", "a,b,c"]].concat());
        assert_eq!((code, out.as_str()), (0, "[gh-list]"), "gh pr list");
        let short = ["pr", "list", "-R", "acme/app", "--head", "feature/x"];
        let (code, out, _) = run(&forge, "gh", &[&short[..], &["--state", "all"]].concat());
        assert_eq!((code, out.as_str()), (0, "[gh-list]"), "-R owner/repo");
        let view = [
            "pr",
            "view",
            "7",
            "-R",
            "github.com/acme/app",
            "--json",
            "x",
        ];
        let (code, out, _) = run(&forge, "gh", &view);
        assert_eq!((code, out.as_str()), (0, "{gh-view}"), "gh pr view");

        // GitLab: the endpoint arrives URL-encoded and is decoded.
        let base = "projects/acme%2Fservices%2Fapi";
        let endpoints = [
            (
                format!("{base}/merge_requests?source_branch=feature%2Fx&state=all&per_page=30"),
                "[glab-list]",
            ),
            (format!("{base}/merge_requests/3"), "{glab-mr}"),
            (
                format!("{base}/merge_requests/3/approvals"),
                "{glab-approvals}",
            ),
            (
                format!("{base}/pipelines/99/jobs?scope[]=failed&scope[]=running"),
                "[glab-jobs]",
            ),
        ];
        for (endpoint, want) in &endpoints {
            let (code, out, _) = run(
                &forge,
                "glab",
                &["api", "--hostname", "gitlab.com", endpoint],
            );
            assert_eq!((code, out.as_str()), (0, *want), "glab api {endpoint}");
        }

        // No scenario: exit 1, and the call is still logged.
        let (code, out, err) = run(&forge, "gh", &["pr", "view", "8", "-R", "acme/app"]);
        assert_eq!((code, out.as_str()), (1, ""));
        assert_eq!(
            err,
            "fake forge: no scenario for gh pr view 8 -R acme/app\n"
        );

        // A failing answer, then the same call answered again.
        forge.github_pr_view(
            "acme/app",
            7,
            Reply::Fail {
                code: 4,
                stderr: "not logged in\n",
            },
        );
        let (code, out, err) = run(&forge, "gh", &view);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (4, "", "not logged in\n")
        );
        forge.github_pr_view("acme/app", 7, Reply::Json("{gh-view-2}"));
        let (code, out, err) = run(&forge, "gh", &view);
        assert_eq!((code, out.as_str(), err.as_str()), (0, "{gh-view-2}", ""));

        let calls = forge.calls();
        assert_eq!(calls.len(), 10, "one log line per call: {calls:?}");
        assert_eq!(
            calls[0],
            "gh pr list -R github.com/acme/app --head feature/x --json a,b,c"
        );
        assert_eq!(
            calls[4],
            format!("glab api --hostname gitlab.com {base}/merge_requests/3")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn set_origin_adds_then_replaces() {
        let root = unique_root("ws-fake-forge-origin");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        set_origin(&root, GITHUB_ORIGIN);
        assert_eq!(
            git_stdout(&root, &["remote", "get-url", "origin"]),
            GITHUB_ORIGIN
        );
        set_origin(&root, GITLAB_ORIGIN);
        assert_eq!(
            git_stdout(&root, &["remote", "get-url", "origin"]),
            GITLAB_ORIGIN
        );
        let _ = fs::remove_dir_all(&root);
    }
}
