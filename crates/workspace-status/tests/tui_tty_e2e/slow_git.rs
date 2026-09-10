//! `WORKSPACE_STATUS_GIT` wrapper that sleeps on `fetch` and `pull` only.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::harness::PtySession;

/// Sleep on each wrapped `fetch` / `pull` so a second key can land in-flight.
///
/// Sized so two overlapping remotes still finish inside [`crate::support::GIT_WAIT`].
const SLOW_SLEEP_SECS: &str = "2.5";

/// Wrapper around the real git binary. Other git verbs pass through.
pub struct SlowFetchPullGit {
    /// Path to pass as `WORKSPACE_STATUS_GIT`.
    pub shim: PathBuf,
    started: PathBuf,
}

impl SlowFetchPullGit {
    /// Install a 0755 shim under `workspace/.e2e-git-shim/`.
    pub fn install(workspace: &Path) -> Self {
        let shim_dir = workspace.join(".e2e-git-shim");
        fs::create_dir_all(&shim_dir).unwrap();
        let shim = shim_dir.join("git");
        let started = shim_dir.join("started");
        let real_git = std::env::var("WS_E2E_REAL_GIT").unwrap_or_else(|_| {
            if Path::new("/usr/bin/git").is_file() {
                "/usr/bin/git".into()
            } else {
                "git".into()
            }
        });
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\n\
                 real=\"{real_git}\"\n\
                 started=\"{started}\"\n\
                 is_slow=0\n\
                 for a in \"$@\"; do\n\
                   case \"$a\" in\n\
                     fetch|pull) is_slow=1; break ;;\n\
                   esac\n\
                 done\n\
                 if [ \"$is_slow\" = 1 ]; then\n\
                   : > \"$started\"\n\
                   sleep {sleep}\n\
                 fi\n\
                 exec \"$real\" \"$@\"\n",
                real_git = real_git,
                started = started.display(),
                sleep = SLOW_SLEEP_SECS,
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&shim).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim, perms).unwrap();
        Self { shim, started }
    }

    /// True after the wrapper has begun a slow `fetch` or `pull`.
    pub fn started(&self) -> bool {
        self.started.exists()
    }
}

/// Spawn the TUI with the slow git wrapper. FetchTick stays off (harness default).
pub fn open_with_slow_git(workspace: &Path, slow: &SlowFetchPullGit) -> PtySession {
    PtySession::open_with_env(
        workspace,
        &[(
            "WORKSPACE_STATUS_GIT",
            slow.shim.to_str().expect("utf-8 shim path"),
        )],
    )
}

/// Wait until the wrapper has started a slow fetch/pull (occupy, before finish).
pub fn wait_slow_git_started(
    slow: &SlowFetchPullGit,
    screen: impl Fn() -> String,
    timeout: Duration,
) {
    let start = Instant::now();
    while !slow.started() {
        if start.elapsed() >= timeout {
            panic!(
                "timeout waiting for slow git fetch/pull to start; screen:\n{}",
                screen()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}
