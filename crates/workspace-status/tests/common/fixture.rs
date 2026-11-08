//! Std-only fixture helpers that every integration test binary can share.
//!
//! `tests/common/mod.rs` and `tag_release.rs` mod this file as part of
//! `common`. A test binary that does not mod `common` takes it alone with
//! `#[path = "common/fixture.rs"] mod fixture;`.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Author / committer plus isolated git config for fixture repos.
///
/// `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_NOSYSTEM` keep the host git config
/// (hooks from `init.templateDir`, signing, default branch) out of the
/// fixtures. Pass these to every `git` the tests run, `git init` included.
pub fn git_env() -> Vec<(&'static str, &'static str)> {
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

/// Process-wide counter, so two calls in one process never share a name.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// New empty temp directory `{prefix}-{pid}-{n}-{nanos}`, already created.
///
/// The process id and the counter keep parallel tests and parallel test
/// binaries apart, also when the clock does not tick between two calls.
/// `create_dir` fails on a name that exists, so a collision takes the next
/// name instead of sharing a directory.
pub fn unique_root(prefix: &str) -> PathBuf {
    let pid = std::process::id();
    loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{pid}-{n}-{nanos}"));
        match fs::create_dir(&path) {
            Ok(()) => return path,
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => panic!("create temp dir {}: {err}", path.display()),
        }
    }
}
