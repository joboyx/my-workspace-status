//! CI watch for the release gate.
//!
//! `.github/workflows/test.yml` runs the checks on pull requests and on main.
//! Its `release` job calls `.github/workflows/tag-release.yml` only after
//! every check passed on a main push, and only while that commit is still the
//! tip of main. The main ruleset requires the checks by job name and lets
//! only the release deploy key bypass them. These tests fail when a check job
//! is missing from the gate or from the documented ruleset list, when
//! tag-release gets a trigger of its own, or when the deploy key push path
//! drifts. Settings and release check: [docs/architecture.md](../../../docs/architecture.md)
//! (**Release gate**).

const TEST_YML: &str = include_str!("../../../.github/workflows/test.yml");
const TAG_RELEASE_YML: &str = include_str!("../../../.github/workflows/tag-release.yml");
const ARCHITECTURE_MD: &str = include_str!("../../../docs/architecture.md");

/// Jobs of `test.yml` that are not checks: the gate and the release call.
const RELEASE_JOBS: [&str; 2] = ["release-gate", "release"];

/// Top-level `jobs:` entries of a workflow, as `(name, body)` in file order.
///
/// A job starts at a two-space `<name>:` line under `jobs:` and runs to the
/// next one. Comment lines at job indent stay in the body of the job above.
fn jobs(workflow: &str) -> Vec<(&str, String)> {
    let mut out: Vec<(&str, String)> = Vec::new();
    let mut in_jobs = false;
    for line in workflow.lines() {
        if !line.starts_with(' ') && !line.is_empty() && !line.starts_with('#') {
            in_jobs = line == "jobs:";
            continue;
        }
        if !in_jobs {
            continue;
        }
        let job_name = line
            .strip_prefix("  ")
            .filter(|rest| !rest.starts_with(' ') && !rest.starts_with('#'))
            .and_then(|rest| rest.strip_suffix(':'));
        match job_name {
            Some(name) => out.push((name, String::new())),
            None => {
                if let Some((_, body)) = out.last_mut() {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    out
}

fn job<'a>(jobs: &'a [(&str, String)], name: &str) -> &'a str {
    jobs.iter()
        .find(|(n, _)| *n == name)
        .map(|(_, body)| body.as_str())
        .unwrap_or_else(|| panic!("test.yml must keep the `{name}` job"))
}

/// Job names in a `needs: [a, b]` line of a job body.
fn needs(body: &str) -> Vec<&str> {
    let line = body
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("needs:"))
        .expect("job must declare needs");
    line.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect()
}

fn check_jobs() -> Vec<&'static str> {
    jobs(TEST_YML)
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !RELEASE_JOBS.contains(name))
        .collect()
}

#[test]
fn jobs_parser_reads_top_level_jobs_only() {
    let src = "on:\n  push:\njobs:\n  a:\n    steps:\n      - run: x\n  # note\n  b-c:\n    needs: [a]\nenv:\n  K: v\n";
    let parsed = jobs(src);
    let names: Vec<&str> = parsed.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["a", "b-c"]);
    assert_eq!(needs(&parsed[1].1), ["a"]);
}

#[test]
fn release_gate_needs_every_check_job() {
    let checks = check_jobs();
    for expected in [
        "lint",
        "cargo",
        "cargo-macos",
        "cargo-windows",
        "tui-tty-desktop",
    ] {
        assert!(
            checks.contains(&expected),
            "test.yml must keep the `{expected}` check (the main ruleset requires checks by name)"
        );
    }
    let all = jobs(TEST_YML);
    let mut gate = needs(job(&all, "release-gate"));
    let mut want = checks.clone();
    gate.sort_unstable();
    want.sort_unstable();
    assert_eq!(
        gate, want,
        "release-gate must need every check job of test.yml, so no release runs before a check passes"
    );
}

#[test]
fn default_suite_runs_on_every_released_os_family() {
    // dist-workspace.toml ships Linux, macOS and Windows builds; each one
    // gets the default suite before a release.
    let all = jobs(TEST_YML);
    for (name, runner, commands) in [
        (
            "cargo",
            "runs-on: ubuntu-latest",
            &["run: cargo test --workspace --locked"][..],
        ),
        (
            "cargo-macos",
            "runs-on: macos-15",
            &["run: cargo test --workspace --locked"][..],
        ),
        (
            "cargo-windows",
            "runs-on: windows-latest",
            &[
                "run: cargo build --workspace --all-targets --locked",
                "run: cargo test --workspace --locked",
            ][..],
        ),
    ] {
        let body = job(&all, name);
        assert!(
            body.contains(runner),
            "test.yml `{name}` must keep `{runner}`"
        );
        for command in commands {
            assert!(
                body.contains(command),
                "test.yml `{name}` must keep `{command}`"
            );
        }
    }
    assert!(
        job(&all, "cargo-windows").contains("core.autocrlf false"),
        "cargo-windows must check out with LF line endings; tests read checked-in text"
    );
}

/// Entries of a top-level `<key>:` block of a workflow, trimmed, in file order.
fn top_level_block<'a>(workflow: &'a str, key: &str) -> Vec<&'a str> {
    let header = format!("{key}:");
    workflow
        .lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| line.starts_with(' '))
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

#[test]
fn test_yml_token_is_read_only_and_every_job_has_a_timeout() {
    assert_eq!(
        top_level_block(TEST_YML, "permissions"),
        ["contents: read"],
        "test.yml must default to a read-only token; a job that needs more sets its own permissions"
    );
    for (name, body) in jobs(TEST_YML) {
        if body.contains("uses: ./.github/workflows/") {
            // A reusable workflow call cannot set timeout-minutes: each job
            // of the called workflow sets it.
            for (called, called_body) in jobs(TAG_RELEASE_YML) {
                assert!(
                    called_body.contains("timeout-minutes:"),
                    "tag-release.yml `{called}` (called by test.yml `{name}`) must set timeout-minutes"
                );
            }
            continue;
        }
        assert!(
            body.contains("timeout-minutes:"),
            "test.yml `{name}` must set timeout-minutes, so a hung run cannot hold a runner for 6 hours"
        );
    }
}

#[test]
fn release_gate_runs_only_for_a_main_push_that_is_still_the_tip() {
    let all = jobs(TEST_YML);
    let gate = job(&all, "release-gate");
    assert!(
        gate.contains("if: github.event_name == 'push' && github.ref == 'refs/heads/main'"),
        "release-gate must run only for a push to main, never for a pull request"
    );
    assert!(
        gate.contains("git/ref/heads/main") && gate.contains("\"$tip\" == \"$GITHUB_SHA\""),
        "release-gate must compare the tip of main with the tested commit"
    );
    assert!(
        gate.contains("contents: read") && !gate.contains("contents: write"),
        "release-gate only reads the tip of main"
    );

    let release = job(&all, "release");
    assert_eq!(needs(release), ["release-gate"]);
    for pin in [
        "if: needs.release-gate.outputs.current == 'true'",
        "uses: ./.github/workflows/tag-release.yml",
        "secrets: inherit",
        "contents: write",
        "actions: write",
    ] {
        assert!(
            release.contains(pin),
            "test.yml `release` job must keep `{pin}`"
        );
    }
}

#[test]
fn tag_release_runs_only_when_test_yml_calls_it() {
    let on = TAG_RELEASE_YML
        .split("\non:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("tag-release.yml must have an `on:` block");
    assert_eq!(
        on.trim(),
        "workflow_call:",
        "tag-release.yml must have no trigger of its own; a push trigger would tag a commit before its checks pass"
    );
}

#[test]
fn tag_release_pushes_with_the_release_deploy_key_when_set() {
    for pin in [
        "environment: release",
        "ssh-key: ${{ secrets.RELEASE_DEPLOY_KEY }}",
        "DEPLOY_KEY_PUSH: ${{ secrets.RELEASE_DEPLOY_KEY != '' }}",
    ] {
        assert!(
            TAG_RELEASE_YML.contains(pin),
            "tag-release.yml must keep `{pin}`"
        );
    }
    // A deploy key tag push starts release.yml by itself. Dispatching too would
    // start a second Release run for the same tag. The guarded block must be
    // `if <deploy key>; then <no dispatch>; else <dispatch>; fi`.
    let guard = TAG_RELEASE_YML
        .find("if [[ \"$DEPLOY_KEY_PUSH\" == \"true\" ]]; then")
        .expect("tag-release.yml must skip the dispatch after a deploy key push");
    let block: Vec<&str> = TAG_RELEASE_YML[guard..]
        .lines()
        .map(str::trim)
        .take_while(|line| *line != "fi")
        .collect();
    let else_at = block
        .iter()
        .position(|line| *line == "else")
        .expect("the deploy key guard must have an else branch");
    let dispatch_at = block
        .iter()
        .position(|line| line.starts_with("gh workflow run release.yml"))
        .expect("the release.yml dispatch must sit inside the deploy key guard");
    assert!(
        else_at < dispatch_at,
        "the release.yml dispatch must sit in the else branch of the deploy key guard"
    );
}

#[test]
fn architecture_doc_lists_every_required_check() {
    let line = ARCHITECTURE_MD
        .lines()
        .find(|l| l.contains("Required checks (job names):"))
        .expect("docs/architecture.md must list the ruleset's required checks");
    for check in check_jobs() {
        assert!(
            line.contains(&format!("`{check}`")),
            "docs/architecture.md required checks must name `{check}`; the ruleset must match test.yml"
        );
    }
}
