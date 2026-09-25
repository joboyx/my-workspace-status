//! Guard: every production git spawn must set `GIT_OPTIONAL_LOCKS=0`.
//!
//! `git::git_process` is the one place that builds a bare `Command::new`
//! for the git binary and sets the env var (see `docs/git-operations.md`).
//! A shipped call that constructs `Command::new(git_binary())` /
//! `Command::new(crate::git::git_binary())` directly would skip it and can
//! collide with a concurrent write on `.git/index.lock`. This scans
//! production source only — `#[cfg(test)]` fixtures (inline `mod tests`
//! blocks and the whole-file `testutil.rs` fixture module) are exempt,
//! since none of that code ships.

use std::fs;
use std::path::{Path, PathBuf};

const BANNED: [&str; 2] = [
    "Command::new(git_binary())",
    "Command::new(crate::git::git_binary())",
];

/// Strip the trailing `#[cfg(test)] mod tests { ... }` block, if any, so
/// only shipped code is scanned.
fn production_source(src: &str) -> &str {
    match src.find("#[cfg(test)]\nmod tests") {
        Some(idx) => &src[..idx],
        None => src,
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn git_spawns_go_through_git_process() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut roots = vec![manifest_dir.join("src")];
    let sibling_crate_src = manifest_dir.join("../workspace-status-graph/src");
    if sibling_crate_src.is_dir() {
        roots.push(sibling_crate_src);
    }

    let mut files = Vec::new();
    for root in &roots {
        collect_rs_files(root, &mut files);
    }
    assert!(
        !files.is_empty(),
        "expected to find source files under {roots:?}"
    );

    let mut violations = Vec::new();
    for path in &files {
        if path.file_name().and_then(|n| n.to_str()) == Some("testutil.rs") {
            // Whole file is `#[cfg(test)]`-gated at its `mod` declaration
            // in lib.rs; it never ships.
            continue;
        }
        let src =
            fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let prod = production_source(&src);
        for (i, line) in prod.lines().enumerate() {
            if BANNED.iter().any(|needle| line.contains(needle)) {
                violations.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "production git spawns must build via git::git_process() so GIT_OPTIONAL_LOCKS=0 \
         is always set; found direct Command::new(git_binary()) construction:\n{}",
        violations.join("\n")
    );
}
