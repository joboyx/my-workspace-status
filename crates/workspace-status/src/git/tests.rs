use super::*;
use crate::testutil::{
    git, init_repo, init_repo_empty, pull_fixture, unique_dir, upstream_rewrite_fixture,
    RewriteFixture, RewriteKind,
};
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn resolve_git_binary_env_beats_config_then_default() {
    let config = Path::new("/opt/git/bin/git");
    assert_eq!(
        resolve_git_binary(Some("/env/git"), Some(config)),
        PathBuf::from("/env/git")
    );
    assert_eq!(resolve_git_binary(None, Some(config)), config);
    assert_eq!(
        resolve_git_binary(Some(""), Some(config)),
        config,
        "empty env falls through to config"
    );
    assert_eq!(
        resolve_git_binary(None, Some(Path::new("git-wrapper"))),
        PathBuf::from("git-wrapper")
    );
    let default = resolve_git_binary(None, None);
    assert!(
        default == Path::new("/usr/bin/git") || default == Path::new("git"),
        "{default:?}"
    );
}

#[test]
fn git_binary_is_nonempty() {
    assert!(!git_binary().as_os_str().is_empty());
}

#[test]
fn git_command_nulls_stdin_and_disables_prompts() {
    let dir = unique_dir("ws-git-stdin");
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("probe");
    fs::write(
            &script,
            "#!/bin/sh\nif [ -t 0 ]; then echo TTY; exit 7; fi\nprintf 'prompt=%s\\n' \"${GIT_TERMINAL_PROMPT-}\"\n",
        )
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Write + chmod + exec can return ETXTBSY on a busy runner.
    let out = {
        let mut last_err = None;
        let mut result = None;
        for _ in 0..8 {
            match git_command(&script, &["status"], &dir).output() {
                Ok(out) => {
                    result = Some(out);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    last_err = Some(err);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(err) => panic!("probe runs: {err}"),
            }
        }
        result.unwrap_or_else(|| panic!("probe runs: {last_err:?}"))
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "probe failed: {stdout} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("prompt=0"),
        "expected GIT_TERMINAL_PROMPT=0, got {stdout:?}"
    );
    assert!(
        !stdout.contains("TTY"),
        "stdin must not be a TTY: {stdout:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_process_disables_optional_locks() {
    let cmd = git_process(git_binary());
    assert_eq!(
        cmd.get_envs()
            .find(|(k, _)| *k == "GIT_OPTIONAL_LOCKS")
            .and_then(|(_, v)| v),
        Some(std::ffi::OsStr::new("0")),
        "every ws git spawn must disable optional locks so a background poll cannot \
             collide with a concurrent write on .git/index.lock"
    );
}

#[test]
fn git_command_and_run_with_stdin_disable_optional_locks() {
    let dir = std::env::temp_dir();
    let has_no_locks_env = |cmd: &Command| {
        cmd.get_envs()
            .find(|(k, _)| *k == "GIT_OPTIONAL_LOCKS")
            .and_then(|(_, v)| v)
            == Some(std::ffi::OsStr::new("0"))
    };
    assert!(has_no_locks_env(&git_command(
        git_binary(),
        &["status"],
        &dir
    )));

    let out = run_with_stdin(&["hash-object", "--stdin"], &dir, b"probe\n")
        .expect("run_with_stdin spawns");
    assert!(out.status.success(), "hash-object failed: {out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim().len(),
        40,
        "stdin reached git through the bounded runner: {out:?}"
    );
}

/// Write an executable fake git `script` into a new temp dir.
fn fake_git(name: &str, script: &str) -> (PathBuf, PathBuf) {
    let dir = unique_dir(name);
    fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("fake-git");
    fs::write(&bin, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (dir, bin)
}

/// Run `cmd()` under `limit`, retrying while write + chmod + exec
/// returns ETXTBSY on a busy runner.
fn output_retrying_busy(
    cmd: impl Fn() -> Command,
    limit: std::time::Duration,
) -> io::Result<Output> {
    for _ in 0..8 {
        match git_output_within(cmd(), None, limit) {
            Err(err) if err.kind() == io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
    git_output_within(cmd(), None, limit)
}

#[test]
fn git_subcommand_skips_global_options_and_spots_remote_commands() {
    for remote in [
        &["fetch"][..],
        &["-c", "x=y", "fetch"],
        &["-C", "dir", "--no-pager", "pull", "--ff-only"],
        &["push", "origin", "main"],
        &["pull"],
        &["ls-remote", "origin"],
        &["clone", "url"],
    ] {
        assert!(is_remote_git(remote), "{remote:?} is remote");
        assert_eq!(git_time_limit(remote), REMOTE_GIT_TIME_LIMIT);
    }
    for local in [
        &["status"][..],
        &["config", "--get", "core.sshCommand"],
        &["remote", "get-url", "origin"],
        &["-c", "fetch.x=y", "log"],
        &[],
    ] {
        assert!(!is_remote_git(local), "{local:?} is local");
        assert_eq!(git_time_limit(local), LOCAL_GIT_TIME_LIMIT);
    }
    assert_eq!(git_subcommand(&["-c", "k=v", "fetch"]), Some("fetch"));
    assert_eq!(git_subcommand(&["--no-pager"]), None);
}

#[test]
fn default_ssh_command_adds_timeouts_only_when_no_custom_ssh_is_set() {
    let ssh = default_ssh_command(None, None, None).expect("default ssh command");
    assert_eq!(
        ssh,
        "ssh -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=2 \
             -o BatchMode=yes"
    );
    let empty = Some(OsStr::new(""));
    assert_eq!(
        default_ssh_command(empty, empty, Some(" ")),
        Some(ssh),
        "empty values do not count as set"
    );
    let custom = Some(OsStr::new("my-ssh"));
    assert_eq!(default_ssh_command(custom, None, None), None);
    assert_eq!(default_ssh_command(None, custom, None), None);
    assert_eq!(default_ssh_command(None, None, Some("my-ssh -i key")), None);
}

#[test]
fn remote_git_gets_ssh_connect_timeout_unless_a_custom_ssh_is_set() {
    // `config` answers the `core.sshCommand` lookup; `envs` go on the
    // command after the operator's ssh env is cleared from it (the
    // process env is not touched).
    let probe = |name: &str, config: &str, envs: &[(&str, &str)]| {
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = config ]; then {config}; fi\n\
                 printf 'ssh=%s\\n' \"${{GIT_SSH_COMMAND-}}\"\n"
        );
        let (dir, bin) = fake_git(name, &script);
        let out = output_retrying_busy(
            || {
                let mut cmd = git_command(&bin, &["fetch", "--quiet"], &dir);
                cmd.env_remove("GIT_SSH_COMMAND").env_remove("GIT_SSH");
                cmd.envs(envs.iter().copied());
                cmd
            },
            REMOTE_GIT_TIME_LIMIT,
        )
        .expect("fake git runs");
        let _ = fs::remove_dir_all(&dir);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(
        probe("ws-git-ssh-default", "exit 1", &[]),
        format!("ssh={}", default_ssh_command(None, None, None).unwrap()),
        "remote git gets the ssh deadline"
    );
    assert_eq!(
        probe("ws-git-ssh-config", "echo 'my-ssh -i key'; exit 0", &[]),
        "ssh=",
        "core.sshCommand must not be replaced"
    );
    assert_eq!(
        probe("ws-git-ssh-config-fails", "exit 2", &[]),
        "ssh=",
        "a failed core.sshCommand lookup leaves the env alone"
    );
    assert_eq!(
        probe(
            "ws-git-ssh-env-command",
            "exit 1",
            &[("GIT_SSH_COMMAND", "my-ssh")]
        ),
        "ssh=my-ssh",
        "GIT_SSH_COMMAND must not be replaced"
    );
    assert_eq!(
        probe("ws-git-ssh-env-ssh", "exit 1", &[("GIT_SSH", "my-ssh")]),
        "ssh=",
        "GIT_SSH must not get a GIT_SSH_COMMAND on top"
    );
}

/// True while `pid` exists and is not a zombie. A killed orphan stays a
/// zombie until init reaps it, and init may never reap in a container.
#[cfg(unix)]
fn process_runs(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    // Linux: the state letter follows the `(comm)` field.
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .is_none_or(|(_, rest)| !rest.trim_start().starts_with('Z')),
        Err(_) => true,
    }
}

/// The pid a fake git wrote to `pid_file` (atomically, by rename).
#[cfg(unix)]
fn recorded_pid(pid_file: &Path) -> Option<libc::pid_t> {
    fs::read_to_string(pid_file).ok()?.trim().parse().ok()
}

/// Fail unless `pid` is gone within a few seconds.
#[cfg(unix)]
fn assert_gone_soon(pid: libc::pid_t, what: &str) {
    let gone_by = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while process_runs(pid) {
        assert!(std::time::Instant::now() < gone_by, "{what} still runs");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Shell lines that start `sleep 30` in the background, in git's
/// process group, and record its pid in `pid_file` by rename.
#[cfg(unix)]
fn background_sleep(pid_file: &Path) -> String {
    let pid_file = pid_file.display();
    format!("sleep 30 &\necho $! > '{pid_file}.tmp' && mv '{pid_file}.tmp' '{pid_file}'\n")
}

#[cfg(unix)]
#[test]
fn stalled_git_is_killed_with_its_process_group_at_the_time_limit() {
    let run = |name: &str, args: &[&str]| {
        let (dir, bin) = fake_git(name, "");
        let pid_file = dir.join("sleep.pid");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = config ]; then exit 1; fi\n{}wait\n",
            background_sleep(&pid_file)
        );
        fs::write(&bin, script).unwrap();
        // A busy host may not start the script within the short bound;
        // retry with a longer one so the group-kill check always runs.
        for bound in [500, 3000].map(std::time::Duration::from_millis) {
            let started = std::time::Instant::now();
            let err = output_retrying_busy(|| git_command(&bin, args, &dir), bound)
                .expect_err("a stalled git times out");
            assert!(
                started.elapsed() < bound + std::time::Duration::from_secs(4),
                "returned after {:?}",
                started.elapsed()
            );
            let Some(pid) = recorded_pid(&pid_file) else {
                continue;
            };
            // The background `sleep` shares git's process group, so the
            // kill reaches it. It may need a moment to be reaped.
            assert_gone_soon(pid, "the child of a killed git");
            let _ = fs::remove_dir_all(&dir);
            return (err, bound.as_secs());
        }
        panic!("the fake git never started");
    };
    let (local, secs) = run("ws-git-stall-local", &["status"]);
    assert_eq!(local.kind(), io::ErrorKind::TimedOut);
    assert_eq!(
        local.to_string(),
        format!("git status timed out after {secs}s")
    );
    let (remote, secs) = run("ws-git-stall-remote", &["fetch", "--quiet"]);
    assert_eq!(remote.kind(), io::ErrorKind::TimedOut);
    assert_eq!(
        remote.to_string(),
        format!("git fetch timed out after {secs}s: the remote did not answer")
    );
    assert_eq!(
        timed_out(None, std::time::Duration::from_secs(5), false).to_string(),
        "git timed out after 5s",
        "no subcommand: no `git git`"
    );
}

#[cfg(unix)]
#[test]
fn git_that_exits_returns_its_status_while_a_child_holds_its_output() {
    let (dir, bin) = fake_git("ws-git-pipe-holder", "");
    let pid_file = dir.join("sleep.pid");
    // The background `sleep` inherits stdout and stderr and keeps them
    // open after git exits.
    let script = format!(
        "#!/bin/sh\n{}echo done\nexit 0\n",
        background_sleep(&pid_file)
    );
    fs::write(&bin, script).unwrap();
    let started = std::time::Instant::now();
    let out = output_retrying_busy(
        || git_command(&bin, &["status"], &dir),
        LOCAL_GIT_TIME_LIMIT,
    )
    .expect("an exited git is not an error");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "returned after {:?}",
        started.elapsed()
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "done");
    let pid = recorded_pid(&pid_file).expect("git wrote the pid before it exited");
    assert_gone_soon(pid, "the pipe holder in git's group");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn git_that_exits_while_an_escaped_process_holds_its_output_is_a_distinct_error() {
    if !Path::new("/usr/bin/setsid").exists() {
        eprintln!("skipped: /usr/bin/setsid is missing, so no process can leave git's group");
        return;
    }
    let (dir, bin) = fake_git("ws-git-pipe-escape", "");
    let pid_file = dir.join("sleep.pid");
    let pid = pid_file.display();
    // `setsid` moves the holder out of git's group, so the kill cannot
    // reach it. The holder records its pid only after it moved, and git
    // waits for that record (up to 5 s) before it exits, so the group
    // kill after git exits cannot catch the holder still in the group.
    let script = format!(
        "#!/bin/sh\n\
             /usr/bin/setsid sh -c 'echo $$ > \"{pid}.tmp\" && mv \"{pid}.tmp\" \"{pid}\"; \
             exec sleep 30' &\n\
             i=0\n\
             while [ ! -s '{pid}' ] && [ $i -lt 500 ]; do sleep 0.01; i=$((i+1)); done\n\
             exit 0\n"
    );
    fs::write(&bin, script).unwrap();
    let started = std::time::Instant::now();
    let err = output_retrying_busy(
        || git_command(&bin, &["status"], &dir),
        LOCAL_GIT_TIME_LIMIT,
    )
    .expect_err("an output held open is an error");
    if let Some(pid) = recorded_pid(&pid_file) {
        // SAFETY: a plain syscall; `pid` is the `sleep` this test started.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    let _ = fs::remove_dir_all(&dir);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "returned after {:?}",
        started.elapsed()
    );
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(
        err.to_string(),
        "git status exited but a process it started kept its output open"
    );
}

/// Wait up to 10 s for a fake git to record its pid in `pid_file`.
#[cfg(unix)]
fn wait_for_pid(pid_file: &Path) -> libc::pid_t {
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(pid) = recorded_pid(pid_file) {
            return pid;
        }
        assert!(
            std::time::Instant::now() < give_up,
            "the fake git never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Shell line that records the script's own pid (its process group,
/// since git leads its group) in `pid_file` by rename.
#[cfg(unix)]
fn record_own_pid(pid_file: &Path) -> String {
    let pid_file = pid_file.display();
    format!("echo $$ > '{pid_file}.tmp' && mv '{pid_file}.tmp' '{pid_file}'\n")
}

#[cfg(unix)]
#[test]
fn timed_out_git_gets_sigterm_first_and_frees_its_group_slot() {
    let (dir, bin) = fake_git("ws-git-term-first", "");
    let pid_file = dir.join("git.pid");
    let marker = dir.join("cleaned");
    // Like git, the script cleans up on SIGTERM. The pid is recorded
    // after the trap is set, so a recorded pid means the trap was live
    // when the stop came.
    let script = format!(
        "#!/bin/sh\ntrap 'echo cleaned > \"{}\"; exit 143' TERM\n{}sleep 30 &\nwait\n",
        marker.display(),
        record_own_pid(&pid_file)
    );
    fs::write(&bin, script).unwrap();
    for bound in [500, 3000].map(std::time::Duration::from_millis) {
        let started = std::time::Instant::now();
        let err = output_retrying_busy(|| git_command(&bin, &["checkout"], &dir), bound)
            .expect_err("a stalled git times out");
        let Some(pid) = recorded_pid(&pid_file) else {
            continue;
        };
        assert!(
            started.elapsed() < bound + GIT_TERM_GRACE + std::time::Duration::from_secs(2),
            "returned after {:?}",
            started.elapsed()
        );
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            fs::read_to_string(&marker).unwrap_or_default().trim(),
            "cleaned",
            "git got SIGTERM and ran its cleanup before the kill"
        );
        assert!(!is_tracked(pid), "the group slot is free after a timeout");
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    panic!("the fake git never started");
}

#[cfg(unix)]
#[test]
fn running_git_holds_a_group_slot_until_it_is_reaped() {
    let (dir, bin) = fake_git("ws-git-slot", "");
    let pid_file = dir.join("git.pid");
    let go = dir.join("go");
    let script = format!(
            "#!/bin/sh\n{}i=0\nwhile [ ! -e '{}' ] && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done\necho ok\n",
            record_own_pid(&pid_file),
            go.display()
        );
    fs::write(&bin, script).unwrap();
    let run = {
        let (dir, bin) = (dir.clone(), bin.clone());
        std::thread::spawn(move || {
            output_retrying_busy(
                || git_command(&bin, &["status"], &dir),
                LOCAL_GIT_TIME_LIMIT,
            )
        })
    };
    let pid = wait_for_pid(&pid_file);
    assert!(is_tracked(pid), "a running git holds a group slot");
    fs::write(&go, "").unwrap();
    let out = run.join().unwrap().expect("fake git runs");
    assert!(out.status.success(), "{out:?}");
    assert!(!is_tracked(pid), "the group slot is free after git exits");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn git_that_cannot_start_returns_the_spawn_error() {
    // The group registers only after a spawn succeeds, so a spawn error
    // holds no slot.
    let dir = std::env::temp_dir();
    let err = git_output_within(
        git_command(Path::new("/nonexistent/ws-no-git"), &["status"], &dir),
        None,
        LOCAL_GIT_TIME_LIMIT,
    )
    .expect_err("a missing binary does not start");
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn stop_groups_ends_a_running_git_promptly() {
    let (dir, bin) = fake_git("ws-git-stop", "");
    let pid_file = dir.join("git.pid");
    let script = format!("#!/bin/sh\n{}sleep 30 &\nwait\n", record_own_pid(&pid_file));
    fs::write(&bin, script).unwrap();
    // A registry of this test's own group only: the global one also
    // holds the groups of tests that run in parallel.
    let slots = std::sync::Arc::new([AtomicI32::new(0)]);
    let run = {
        let (dir, bin, slots) = (dir.clone(), bin.clone(), slots.clone());
        std::thread::spawn(move || {
            let result = output_retrying_busy(
                || git_command(&bin, &["status"], &dir),
                LOCAL_GIT_TIME_LIMIT,
            );
            slots[0].store(0, Ordering::SeqCst);
            result
        })
    };
    let pid = wait_for_pid(&pid_file);
    slots[0].store(pid, Ordering::SeqCst);
    let started = std::time::Instant::now();
    stop_groups(&slots[..], GIT_TERM_GRACE);
    assert!(
        started.elapsed() < GIT_TERM_GRACE,
        "git ended at SIGTERM, before the grace ran out: {:?}",
        started.elapsed()
    );
    match run.join().unwrap() {
        Ok(out) => assert!(!out.status.success(), "a stopped git fails: {out:?}"),
        Err(err) => assert_ne!(err.kind(), io::ErrorKind::TimedOut, "{err}"),
    }
    assert!(!is_tracked(pid), "the group slot is free after the stop");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_may_start_refuses_new_git_once_ws_is_exiting() {
    assert!(check_may_start(false, false, Some("fetch")).is_ok());
    let err = check_may_start(true, false, Some("fetch")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert_eq!(err.to_string(), "git fetch not started: ws is exiting");
    assert_eq!(
        check_may_start(true, false, None).unwrap_err().to_string(),
        "git not started: ws is exiting"
    );
    assert!(
        check_may_start(true, true, Some("stash")).is_ok(),
        "a run that restores the user's work still starts while ws exits"
    );
    assert!(check_may_start(false, true, Some("stash")).is_ok());
}

#[test]
fn quit_waits_while_a_pull_still_has_to_restore_the_auto_stash() {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let hold = Duration::from_millis(200);
    let restoring = std::thread::spawn(move || {
        let _restoring = RestoringUserWork::start();
        started_tx.send(()).unwrap();
        std::thread::sleep(hold);
    });
    started_rx.recv().unwrap();
    let start = Instant::now();
    assert!(
        wait_while_restoring_user_work(Duration::from_secs(10)),
        "the wait ends once the restore is done"
    );
    assert!(
        start.elapsed() >= hold - Duration::from_millis(50),
        "quit waited for the restore: {:?}",
        start.elapsed()
    );
    restoring.join().unwrap();

    let _stuck = RestoringUserWork::start();
    let start = Instant::now();
    assert!(
        !wait_while_restoring_user_work(Duration::from_millis(100)),
        "a restore that does not end stops the wait at its bound"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}

/// Set in the child process that
/// [`quit_during_pull_still_restores_the_auto_stash`] starts. The child
/// sets the process-wide shutdown flag, which would break every other
/// test in a shared test process.
const EXITING_CHILD_ENV: &str = "WS_STATUS_TEST_EXITING_CHILD";

#[cfg(unix)]
#[test]
fn quit_during_pull_still_restores_the_auto_stash() {
    if std::env::var_os(EXITING_CHILD_ENV).is_some() {
        quit_during_pull_in_this_process();
        return;
    }
    let name = "git::tests::quit_during_pull_still_restores_the_auto_stash";
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(EXITING_CHILD_ENV, "1")
        .output()
        .expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child test failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Body of [`quit_during_pull_still_restores_the_auto_stash`], run in a
/// child process: a quit (`stop_running_git`) while the pull of a dirty
/// repo hangs must still pop the auto-stash.
#[cfg(unix)]
fn quit_during_pull_in_this_process() {
    let root = unique_dir("ws-git-quit-pull");
    let up = root.join("up");
    let down = root.join("down");
    init_repo(&up);
    git(
        &root,
        &["clone", "-q", up.to_str().unwrap(), down.to_str().unwrap()],
    );
    git(&down, &["config", "user.name", "workspace-status test"]);
    git(
        &down,
        &[
            "config",
            "user.email",
            "workspace-status-test@example.invalid",
        ],
    );
    // The pull hangs: the remote's upload-pack never answers.
    let (_, hang) = fake_git("ws-git-quit-pull-upload-pack", "#!/bin/sh\nsleep 30\n");
    git(
        &down,
        &["config", "remote.origin.uploadpack", hang.to_str().unwrap()],
    );
    fs::write(down.join("README.md"), "# local edit\n").unwrap();

    let pull = {
        let down = down.clone();
        std::thread::spawn(move || pull_quiet_detailed(&down))
    };
    // Wait for the auto-stash, then for the pull to be under way.
    let stash_count = || {
        let out = Command::new(git_binary())
            .args(["stash", "list"])
            .current_dir(&down)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).lines().count()
    };
    // The push has saved its entry and reset the worktree. A stop that still
    // reaches the push gives the same result: the entry is popped back.
    let pushed =
        || stash_count() == 1 && fs::read_to_string(down.join("README.md")).unwrap() == "# seed\n";
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !pushed() {
        assert!(std::time::Instant::now() < give_up, "no auto-stash");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));

    let started = std::time::Instant::now();
    stop_running_git();
    let result = pull.join().unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "quit waited {:?}",
        started.elapsed()
    );
    assert!(result.stashed, "{result:?}");
    assert!(
        result.error.is_some(),
        "the stopped pull failed: {result:?}"
    );
    assert!(!result.stash_pop_failed, "the pop ran: {result:?}");
    assert_eq!(
        fs::read_to_string(down.join("README.md")).unwrap(),
        "# local edit\n",
        "the user's change is back in the worktree"
    );
    assert_eq!(stash_count(), 0, "no auto-stash entry is left");
    // Other git is still refused.
    let refused = exec_git_checked(&["status"], &down).unwrap_err();
    assert_eq!(refused, "git status not started: ws is exiting");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(hang.parent().unwrap());
}

#[test]
fn git_diff_args_inserts_full_context() {
    let path = literal_pathspec("README.md");
    let normal = git_diff_args(&["diff"], "README.md", None);
    assert_eq!(normal, vec!["diff".to_string(), "--".into(), path.clone()]);
    let full = git_diff_args(&["diff"], "README.md", Some(FULL_DIFF_CONTEXT_LINES));
    assert_eq!(
        full,
        vec![
            "diff".to_string(),
            format!("-U{FULL_DIFF_CONTEXT_LINES}"),
            "--".into(),
            path,
        ]
    );
}

#[test]
fn stage_unstage_revert_on_fixture() {
    let dir = unique_dir("ws-git-ops");
    init_repo(&dir);
    fs::write(dir.join("README.md"), "# dirty\n").unwrap();
    assert!(repo_has_local_changes(&dir));
    assert_ne!(exec_git_status(&["diff", "--quiet"], &dir), 0);
    stage_file(&dir, "README.md").unwrap();
    assert_ne!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
    unstage_file(&dir, "README.md").unwrap();
    assert_eq!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
    assert_ne!(exec_git_status(&["diff", "--quiet"], &dir), 0);
    revert_tracked_file(&dir, "README.md").unwrap();
    assert_eq!(exec_git_status(&["diff", "--quiet"], &dir), 0);
    assert!(!repo_has_local_changes(&dir));
    fs::write(dir.join("tmp-untracked.txt"), "x\n").unwrap();
    assert!(!repo_has_local_changes(&dir));
    remove_untracked_file(&dir, "tmp-untracked.txt").unwrap();
    assert!(!dir.join("tmp-untracked.txt").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// (target, sibling) pairs: as a glob pathspec, each target also matches
/// its sibling (`[b]` matches `b`, `*` and `?` match `y` / `x`).
const GLOB_NAME_PAIRS: [(&str, &str); 4] = [
    ("a[b].txt", "ab.txt"),
    ("u?.txt", "ux.txt"),
    ("x*.txt", "xy.txt"),
    ("app/[id]/page.tsx", "app/i/page.tsx"),
];

fn write_file(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn read_file(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// A repo with every [`GLOB_NAME_PAIRS`] file committed as `base\n`.
fn glob_names_fixture(prefix: &str) -> std::path::PathBuf {
    let dir = unique_dir(prefix);
    init_repo(&dir);
    for (target, sibling) in GLOB_NAME_PAIRS {
        write_file(&dir, target, "base\n");
        write_file(&dir, sibling, "base\n");
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "glob names"]);
    dir
}

#[test]
fn literal_pathspec_prefix_unless_git_reads_every_pathspec_literally() {
    assert_eq!(
        literal_pathspec_with("a[b].txt", false),
        ":(literal)a[b].txt"
    );
    assert_eq!(literal_pathspec_with("a[b].txt", true), "a[b].txt");
    for on in ["1", "true", "YES", "on", "2"] {
        assert!(git_env_bool(on), "{on}");
    }
    for off in ["", "0", "false", "No", "off", "maybe"] {
        assert!(!git_env_bool(off), "{off}");
    }
}

// Windows does not allow `*` or `?` in file names.
#[cfg(unix)]
#[test]
fn file_diff_reads_only_the_named_file() {
    let dir = glob_names_fixture("ws-git-literal-diff");
    for (target, sibling) in GLOB_NAME_PAIRS {
        write_file(&dir, target, "dirty\n");
        write_file(&dir, sibling, "dirty\n");
        let args = git_diff_args(&["diff"], target, None);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let diff = exec_git(&refs, &dir);
        assert_eq!(
            diff.matches("diff --git ").count(),
            1,
            "diff of {target} names one file:\n{diff}"
        );
        assert!(!diff.contains(sibling), "diff of {target}:\n{diff}");
        git(&dir, &["checkout", "-q", "--", "."]);
    }
    let _ = fs::remove_dir_all(&dir);
}

// Windows does not allow `*` or `?` in file names.
#[cfg(unix)]
#[test]
fn stage_unstage_revert_take_paths_literally() {
    let dir = glob_names_fixture("ws-git-literal-tracked");
    for (target, sibling) in GLOB_NAME_PAIRS {
        write_file(&dir, target, "dirty\n");
        write_file(&dir, sibling, "dirty\n");

        stage_file(&dir, target).unwrap();
        assert_eq!(staged_paths(&dir), [target], "stage {target}");

        git(&dir, &["add", "-A"]);
        unstage_file(&dir, target).unwrap();
        assert_eq!(staged_paths(&dir), [sibling], "unstage {target}");
        git(&dir, &["reset", "-q"]);

        revert_tracked_file(&dir, target).unwrap();
        assert_eq!(read_file(&dir, target), "base\n", "revert {target}");
        assert_eq!(read_file(&dir, sibling), "dirty\n", "revert {target}");
        git(&dir, &["checkout", "-q", "--", "."]);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn remove_untracked_file_takes_paths_literally() {
    let dir = unique_dir("ws-git-literal-clean");
    init_repo(&dir);
    for (target, sibling) in GLOB_NAME_PAIRS {
        write_file(&dir, target, "new\n");
        write_file(&dir, sibling, "new\n");
        remove_untracked_file(&dir, target).unwrap();
        assert!(!dir.join(target).exists(), "remove {target}");
        assert_eq!(read_file(&dir, sibling), "new\n", "remove {target}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn stash_push_of_a_path_takes_it_literally() {
    let dir = glob_names_fixture("ws-git-literal-stash");
    for (target, sibling) in GLOB_NAME_PAIRS {
        write_file(&dir, target, "dirty\n");
        write_file(&dir, sibling, "dirty\n");
        stash_push(&dir, &[target.to_string()]).unwrap();
        assert_eq!(read_file(&dir, target), "base\n", "stash {target}");
        assert_eq!(read_file(&dir, sibling), "dirty\n", "stash {target}");
        let stashed = exec_git(&["stash", "show", "--name-only", "-z"], &dir);
        assert_eq!(stashed.trim_end_matches('\0'), target, "stash {target}");
        git(&dir, &["stash", "drop", "-q"]);
        git(&dir, &["checkout", "-q", "--", "."]);
    }
    // An untracked target with an untracked sibling (`-u`).
    write_file(&dir, "new/[id].txt", "new\n");
    write_file(&dir, "new/i.txt", "new\n");
    stash_push(&dir, &["new/[id].txt".to_string()]).unwrap();
    assert!(!dir.join("new/[id].txt").exists());
    assert_eq!(read_file(&dir, "new/i.txt"), "new\n");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn revert_compare_file_takes_paths_literally() {
    let dir = glob_names_fixture("ws-git-literal-compare");
    let merge_base = exec_git(&["rev-parse", "HEAD"], &dir);
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    for (target, sibling) in GLOB_NAME_PAIRS {
        write_file(&dir, target, "head\n");
        write_file(&dir, sibling, "head\n");
    }
    git(&dir, &["commit", "-q", "-am", "feature"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);

    for (target, sibling) in GLOB_NAME_PAIRS {
        // A clean sibling: the restore must not reach it.
        revert_compare_file(&dir, &merge_base, &head, target, None).unwrap();
        assert_eq!(read_file(&dir, target), "base\n", "revert {target}");
        assert_eq!(read_file(&dir, sibling), "head\n", "revert {target}");
        git(&dir, &["checkout", "-q", "--", "."]);

        // A dirty sibling: the safety check must not refuse the target.
        write_file(&dir, sibling, "dirty\n");
        revert_compare_file(&dir, &merge_base, &head, target, None).unwrap();
        assert_eq!(read_file(&dir, target), "base\n", "dirty sibling {target}");
        assert_eq!(
            read_file(&dir, sibling),
            "dirty\n",
            "dirty sibling {target}"
        );
        git(&dir, &["checkout", "-q", "--", "."]);
    }
    assert!(index_is_head(&dir));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn apply_cached_patch_stages_and_unstages_one_hunk() {
    let dir = unique_dir("ws-git-apply");
    init_repo(&dir);
    let body = "\
keep-a
keep-b
keep-c
ALPHA-OLD
keep-d
keep-e
keep-f
pad-1
pad-2
pad-3
pad-4
pad-5
pad-6
OMEGA-OLD
keep-x
keep-y
keep-z
";
    fs::write(dir.join("regions.txt"), body).unwrap();
    git(&dir, &["add", "regions.txt"]);
    git(&dir, &["commit", "-q", "-m", "regions"]);
    fs::write(
        dir.join("regions.txt"),
        body.replace("ALPHA-OLD", "ALPHA-NEW")
            .replace("OMEGA-OLD", "OMEGA-NEW"),
    )
    .unwrap();
    let diff = exec_git(&["diff", "--", "regions.txt"], &dir);
    assert!(
        diff.contains("ALPHA-NEW") && diff.contains("OMEGA-NEW"),
        "{diff}"
    );
    let first_hunk = first_unified_hunk(&diff);
    apply_cached_patch(&dir, &first_hunk, false).unwrap();
    let cached = exec_git(&["diff", "--cached", "--", "regions.txt"], &dir);
    let unstaged = exec_git(&["diff", "--", "regions.txt"], &dir);
    assert!(
        cached.contains("ALPHA-NEW") && !cached.contains("OMEGA-NEW"),
        "{cached}"
    );
    assert!(
        unstaged.contains("OMEGA-NEW") && !unstaged.contains("ALPHA-NEW"),
        "{unstaged}"
    );
    apply_cached_patch(&dir, &first_hunk, true).unwrap();
    assert_eq!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
    let unstaged = exec_git(&["diff", "--", "regions.txt"], &dir);
    assert!(
        unstaged.contains("ALPHA-NEW") && unstaged.contains("OMEGA-NEW"),
        "{unstaged}"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn first_unified_hunk(diff: &str) -> String {
    let mut header = String::new();
    let mut hunk = String::new();
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("@@") {
            if in_hunk {
                break;
            }
            in_hunk = true;
            hunk.push_str(line);
            hunk.push('\n');
            continue;
        }
        if in_hunk {
            hunk.push_str(line);
            hunk.push('\n');
        } else {
            header.push_str(line);
            header.push('\n');
        }
    }
    format!("{header}{hunk}")
}

#[test]
fn stash_and_branch_on_fixture() {
    let dir = unique_dir("ws-git-stash");
    init_repo(&dir);
    fs::write(dir.join("README.md"), "# dirty\n").unwrap();
    stash_push(&dir, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        "# seed\n"
    );
    let latest = latest_stash_ref(&dir).expect("stash");
    stash_apply(&dir, &latest).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        "# dirty\n"
    );
    stash_drop(&dir, &latest).unwrap();
    assert!(latest_stash_ref(&dir).is_none());
    fs::write(dir.join("README.md"), "# dirty2\n").unwrap();
    stash_push(&dir, &["README.md".into()]).unwrap();
    let latest = latest_stash_ref(&dir).expect("stash2");
    stash_pop(&dir, &latest).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("README.md")).unwrap(),
        "# dirty2\n"
    );
    assert!(latest_stash_ref(&dir).is_none());

    create_branch_checkout(&dir, "feature/x").unwrap();
    let branches = list_local_branches(&dir);
    assert!(branches.iter().any(|b| b.name == "feature/x" && b.current));
    assert!(checkout_branch("main", &dir));
    assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(
        create_branch_at_args("feature/at", &head),
        ["branch", "--", "feature/at", head.as_str()]
    );
    create_branch_at(&dir, "feature/at", &head).unwrap();
    assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");
    assert_eq!(exec_git(&["rev-parse", "feature/at"], &dir), head);

    let remote = dir.join("remote.git");
    Command::new(git_binary())
        .args(["init", "-q", "--bare", remote.to_str().unwrap()])
        .status()
        .unwrap();
    git(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&dir, &["push", "-u", "origin", "main", "--quiet"]);
    git(&dir, &["checkout", "-q", "-b", "feature/behind"]);
    fs::write(dir.join("README.md"), "# behind-local\n").unwrap();
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "local"]);
    git(&dir, &["push", "-u", "origin", "feature/behind", "--quiet"]);
    // advance origin
    let other = dir.join("other");
    Command::new(git_binary())
        .args([
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    git(&other, &["checkout", "-q", "feature/behind"]);
    fs::write(other.join("README.md"), "# origin-ahead\n").unwrap();
    git(&other, &["add", "README.md"]);
    git(&other, &["commit", "-q", "-m", "remote"]);
    git(&other, &["push", "--quiet"]);
    git(&dir, &["fetch", "--quiet"]);
    assert_eq!(
        origin_out_of_sync(&dir, "feature/behind").as_deref(),
        Some("origin/feature/behind")
    );
    let remote_sha = exec_git(&["rev-parse", "origin/feature/behind"], &dir);
    assert_ne!(exec_git(&["rev-parse", "HEAD"], &dir), remote_sha);
    assert!(fast_forward_to_remote_ref("origin/feature/behind", &dir));
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), remote_sha);
    assert_eq!(
        exec_git(&["branch", "--show-current"], &dir),
        "feature/behind"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fast_forward_to_remote_ref_ahead_and_missing_leave_head() {
    let dir = unique_dir("ws-git-ff");
    init_repo(&dir);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    assert!(!fast_forward_to_remote_ref("origin/foo", &dir));
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), head);
    assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");

    let remote = dir.join("remote.git");
    Command::new(git_binary())
        .args(["init", "-q", "--bare", remote.to_str().unwrap()])
        .status()
        .unwrap();
    git(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&dir, &["push", "-u", "origin", "main", "--quiet"]);
    git(&dir, &["checkout", "-q", "-b", "foo"]);
    git(&dir, &["push", "-u", "origin", "foo", "--quiet"]);
    fs::write(dir.join("ahead.txt"), "ahead\n").unwrap();
    git(&dir, &["add", "ahead.txt"]);
    git(&dir, &["commit", "-q", "-m", "ahead"]);
    let ahead = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(ahead_behind("foo", "origin/foo", &dir), Some((1, 0)));
    assert_eq!(ahead_behind("foo", "origin/missing", &dir), None);
    assert!(!fast_forward_to_remote_ref("origin/foo", &dir));
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), ahead);
    assert_eq!(exec_git(&["branch", "--show-current"], &dir), "foo");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn merge_into_head_ff_merge_commit_conflict_and_worktree() {
    let dir = unique_dir("ws-git-merge");
    init_repo(&dir);
    let seed = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(
        merge_into_head(&seed, &dir),
        MergeIntoHeadResult::AlreadyUpToDate
    );
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), seed);

    git(&dir, &["checkout", "-q", "-b", "topic"]);
    fs::write(dir.join("topic.txt"), "topic\n").unwrap();
    git(&dir, &["add", "topic.txt"]);
    git(&dir, &["commit", "-q", "-m", "topic"]);
    let topic = exec_git(&["rev-parse", "HEAD"], &dir);
    git(&dir, &["tag", "v1.0"]);
    git(&dir, &["checkout", "-q", "main"]);
    assert_eq!(
        merge_into_head(&topic, &dir),
        MergeIntoHeadResult::FastForward
    );
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), topic);
    assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");

    git(&dir, &["reset", "--hard", "--quiet", &seed]);
    fs::write(dir.join("main.txt"), "main\n").unwrap();
    git(&dir, &["add", "main.txt"]);
    git(&dir, &["commit", "-q", "-m", "main"]);
    let main_tip = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(
        merge_into_head(&topic, &dir),
        MergeIntoHeadResult::MergeCommit
    );
    assert_ne!(exec_git(&["rev-parse", "HEAD"], &dir), main_tip);
    assert_eq!(exec_git(&["rev-parse", "HEAD^1"], &dir), main_tip);
    assert_eq!(exec_git(&["rev-parse", "HEAD^2"], &dir), topic);
    assert!(rev_parse_quiet("MERGE_HEAD", &dir).is_none());

    git(&dir, &["reset", "--hard", "--quiet", &seed]);
    fs::write(dir.join("README.md"), "# main-side\n").unwrap();
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "main-side"]);
    git(&dir, &["checkout", "-q", "-B", "conflict-topic", &seed]);
    fs::write(dir.join("README.md"), "# topic-side\n").unwrap();
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "topic-side"]);
    let conflict_topic = exec_git(&["rev-parse", "HEAD"], &dir);
    git(&dir, &["checkout", "-q", "main"]);
    let main_before = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(
        merge_into_head(&conflict_topic, &dir),
        MergeIntoHeadResult::Conflict
    );
    assert!(rev_parse_quiet("MERGE_HEAD", &dir).is_some());
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), main_before);
    git(&dir, &["merge", "--abort"]);

    git(&dir, &["reset", "--hard", "--quiet", &seed]);
    let wt = dir.join(".worktrees").join("feat");
    fs::create_dir_all(dir.join(".worktrees")).unwrap();
    git(
        &dir,
        &[
            "worktree",
            "add",
            "-b",
            "wt-main",
            wt.to_str().unwrap(),
            &seed,
        ],
    );
    let primary_head = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(
        merge_into_head(&topic, &wt),
        MergeIntoHeadResult::FastForward
    );
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &wt), topic);
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), primary_head);

    match merge_into_head("this-ref-does-not-exist", &dir) {
        MergeIntoHeadResult::Failed(_) => {}
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remove_worktree_linked_fixture() {
    let dir = unique_dir("ws-git-wt");
    init_repo(&dir);
    let wt = dir.join(".worktrees").join("feat");
    fs::create_dir_all(dir.join(".worktrees")).unwrap();
    git(
        &dir,
        &["worktree", "add", "-b", "feature/x", wt.to_str().unwrap()],
    );
    assert!(wt.join(".git").exists() || wt.exists());
    remove_worktree(&dir, &wt, false).unwrap();
    assert!(!wt.exists());
    let _ = fs::remove_dir_all(&dir);
}

/// One expected [`NameStatus`] row with no line counts.
fn row(status: &str, path: &str, old_path: Option<&str>) -> NameStatus {
    NameStatus {
        status: status.into(),
        path: path.into(),
        old_path: old_path.map(str::to_string),
        stat: None,
    }
}

#[test]
fn parse_name_status_z_keeps_odd_paths_raw() {
    let stdout = "M\0my file.txt\0A\0café.md\0D\0a -> b.txt\0\
                  R087\0old name.txt\0new name.txt\0C100\0src.rs\0copy\tof src.rs\0\
                  M\0tab\there.txt\0";
    assert_eq!(
        parse_name_status_z(stdout),
        vec![
            row("M", "my file.txt", None),
            row("A", "café.md", None),
            row("D", "a -> b.txt", None),
            row("R", "new name.txt", Some("old name.txt")),
            row("C", "copy\tof src.rs", Some("src.rs")),
            row("M", "tab\there.txt", None),
        ]
    );
    // A rename record cut short is dropped, not misread.
    assert_eq!(
        parse_name_status_z("M\0a.rs\0R100\0old.rs\0"),
        vec![row("M", "a.rs", None)]
    );
    assert!(parse_name_status_z("").is_empty());
}

#[test]
fn parse_name_status_and_commit_files_fixture() {
    assert_eq!(
        parse_name_status_z("M\0src/a.rs\0R100\0old.rs\0new.rs\0"),
        vec![
            NameStatus {
                status: "M".into(),
                path: "src/a.rs".into(),
                old_path: None,
                stat: None,
            },
            NameStatus {
                status: "R".into(),
                path: "new.rs".into(),
                old_path: Some("old.rs".into()),
                stat: None,
            },
        ]
    );
    let dir = unique_dir("ws-git-commit-files");
    init_repo_empty(&dir);
    fs::write(dir.join("one.txt"), "one\n").unwrap();
    git(&dir, &["add", "one.txt"]);
    git(&dir, &["commit", "-q", "-m", "one"]);
    fs::write(dir.join("two.txt"), "two\n").unwrap();
    git(&dir, &["add", "two.txt"]);
    git(&dir, &["commit", "-q", "-m", "two"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let files = list_commit_name_status(&dir, &head);
    assert!(files.iter().any(|f| f.path == "two.txt"), "{files:?}");
    let diff = diff_commit_file(&dir, &head, "two.txt");
    assert!(diff.iter().any(|l| l.contains("two")), "{diff:?}");
    fs::write(dir.join("two.txt"), "dirty\n").unwrap();
    stash_push(&dir, &[]).unwrap();
    fs::write(dir.join("two.txt"), "older\n").unwrap();
    stash_push(&dir, &[]).unwrap();
    let refs = list_stash_refs(&dir);
    assert!(refs.len() >= 2, "{refs:?}");
    let older = refs
        .iter()
        .find(|r| r.ends_with("{1}"))
        .cloned()
        .unwrap_or_else(|| refs[1].clone());
    let stash_files = list_stash_name_status(&dir, &older);
    assert!(
        stash_files.iter().any(|f| f.path == "two.txt"),
        "{stash_files:?}"
    );
    let stash_diff = diff_stash_file(&dir, &older, "two.txt");
    assert!(!stash_diff.is_empty(), "{stash_diff:?}");
    fs::write(dir.join("untracked.txt"), "u\n").unwrap();
    let worktree = list_worktree_name_status(&dir);
    assert!(
        worktree.iter().any(|f| f.path == "untracked.txt"),
        "{worktree:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parse_numstat_z_plain_rename_binary_and_empty() {
    assert!(parse_numstat_z("").is_empty());
    assert_eq!(
        parse_numstat_z(concat!(
            "3\t1\tsrc/a.rs\0",
            "-\t-\timg.png\0",
            "0\t0\tdir/with\ttab.txt\0"
        )),
        vec![
            (
                "src/a.rs".to_string(),
                Some(LineStat {
                    added: 3,
                    deleted: 1
                })
            ),
            ("img.png".to_string(), None),
            (
                "dir/with\ttab.txt".to_string(),
                Some(LineStat {
                    added: 0,
                    deleted: 0
                })
            ),
        ]
    );
    assert_eq!(
        parse_numstat_z(concat!("2\t5\t\0old.rs\0new.rs\0", "1\t1\tz.rs\0")),
        vec![
            (
                "new.rs".to_string(),
                Some(LineStat {
                    added: 2,
                    deleted: 5
                })
            ),
            (
                "z.rs".to_string(),
                Some(LineStat {
                    added: 1,
                    deleted: 1
                })
            ),
        ]
    );
    // A rename row cut short drops the row instead of panicking.
    assert!(parse_numstat_z("2\t5\t\0old.rs\0").is_empty());
}

fn stat_of(files: &[NameStatus], path: &str) -> Option<LineStat> {
    files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} not listed in {files:?}"))
        .stat
}

fn counts(added: u32, deleted: u32) -> Option<LineStat> {
    Some(LineStat { added, deleted })
}

#[test]
fn list_commit_name_status_carries_line_counts() {
    let dir = unique_dir("ws-git-numstat-commit");
    init_repo_empty(&dir);
    // Root commit: exercises the `--root` fallback.
    fs::write(dir.join("a.txt"), "1\n2\n3\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["commit", "-q", "-m", "root"]);
    let root = exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(
        stat_of(&list_commit_name_status(&dir, &root), "a.txt"),
        counts(3, 0)
    );

    fs::write(dir.join("a.txt"), "1\nX\n3\n4\n").unwrap();
    fs::write(dir.join("bin.dat"), b"\x00\x01\x02").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "second"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let files = list_commit_name_status(&dir, &head);
    assert_eq!(stat_of(&files, "a.txt"), counts(2, 1));
    assert_eq!(stat_of(&files, "bin.dat"), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn list_stash_name_status_carries_line_counts() {
    let dir = unique_dir("ws-git-numstat-stash");
    init_repo(&dir);
    fs::write(dir.join("README.md"), "# seed\nmore\nlines\n").unwrap();
    stash_push(&dir, &[]).unwrap();
    let refs = list_stash_refs(&dir);
    let files = list_stash_name_status(&dir, &refs[0]);
    assert_eq!(stat_of(&files, "README.md"), counts(2, 0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn list_worktree_name_status_carries_line_counts_but_not_for_untracked() {
    let dir = unique_dir("ws-git-numstat-worktree");
    init_repo(&dir);
    fs::write(dir.join("README.md"), "changed\n").unwrap();
    fs::write(dir.join("untracked.txt"), "u\nv\n").unwrap();
    let files = list_worktree_name_status(&dir);
    assert_eq!(stat_of(&files, "README.md"), counts(1, 1));
    assert_eq!(stat_of(&files, "untracked.txt"), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn list_compare_name_status_carries_line_counts_incl_rename() {
    let dir = unique_dir("ws-git-numstat-compare");
    init_repo(&dir);
    let base = exec_git(&["rev-parse", "HEAD"], &dir);
    fs::write(dir.join("new.txt"), "a\nb\n").unwrap();
    git(&dir, &["mv", "README.md", "RENAMED.md"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "change"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let files = list_compare_name_status(&dir, &base, &head).unwrap();
    assert_eq!(stat_of(&files, "new.txt"), counts(2, 0));
    assert_eq!(stat_of(&files, "RENAMED.md"), counts(0, 0));
    let _ = fs::remove_dir_all(&dir);
}

/// Repo with `core.quotePath` on (git's default, pinned against a user
/// config), so newline output would C-quote `café.md` and porcelain status
/// would quote `my file.txt`.
fn quote_path_repo(prefix: &str) -> PathBuf {
    let dir = unique_dir(prefix);
    init_repo(&dir);
    git(&dir, &["config", "core.quotePath", "true"]);
    dir
}

/// Odd names with their line count, one per quoting case git has.
const ODD_NAMES: [(&str, &str); 5] = [
    ("my file.txt", "a\nb\n"),
    ("café.md", "c\n"),
    ("a -> b.txt", "d\n"),
    ("tab\there.txt", "t\n"),
    ("say \"hi\".txt", "q\n"),
];

#[test]
fn name_status_lists_give_raw_odd_paths_with_line_counts() {
    let dir = quote_path_repo("ws-git-odd-names");
    let base = exec_git(&["rev-parse", "HEAD"], &dir);
    for (name, body) in ODD_NAMES {
        fs::write(dir.join(name), body).unwrap();
    }
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "odd names"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let commit = list_commit_name_status(&dir, &head);
    let compare = list_compare_name_status(&dir, &base, &head).unwrap();
    let keys = list_compare_patch_keys(&dir, &base, &head);
    for (name, body) in ODD_NAMES {
        let lines = counts(body.lines().count() as u32, 0);
        assert_eq!(stat_of(&commit, name), lines, "commit {name}");
        assert_eq!(stat_of(&compare, name), lines, "compare {name}");
        assert!(keys.contains_key(name), "{name} in {keys:?}");
    }

    git(&dir, &["mv", "café.md", "new café.md"]);
    fs::write(dir.join("my file.txt"), "a\nb\nc\n").unwrap();
    fs::write(dir.join("un tracked é.txt"), "u\n").unwrap();
    let worktree = list_worktree_name_status(&dir);
    assert_eq!(stat_of(&worktree, "my file.txt"), counts(1, 0));
    assert_eq!(stat_of(&worktree, "un tracked é.txt"), None);
    let renamed = worktree
        .iter()
        .find(|f| f.path == "new café.md")
        .unwrap_or_else(|| panic!("rename listed in {worktree:?}"));
    assert_eq!(renamed.status, "R");
    assert_eq!(renamed.old_path.as_deref(), Some("café.md"));
    assert_eq!(renamed.stat, counts(0, 0));
    let vs_commit =
        list_worktree_vs_commit_name_status(&dir, &head, "new café.md", Some("café.md")).unwrap();
    assert_eq!(vs_commit.len(), 1, "{vs_commit:?}");
    assert_eq!(vs_commit[0].path, "new café.md");

    stash_push(&dir, &[]).unwrap();
    let stash = list_stash_name_status(&dir, &list_stash_refs(&dir)[0]);
    assert_eq!(stat_of(&stash, "my file.txt"), counts(1, 0));
    let _ = fs::remove_dir_all(&dir);
}

/// Paths in the index that differ from HEAD (`diff --cached -z`).
fn staged_paths(dir: &Path) -> Vec<String> {
    let out = Command::new(git_binary())
        .args(["diff", "--cached", "--name-only", "-z"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn stage_and_unstage_round_trip_a_listed_path_with_a_space() {
    let dir = quote_path_repo("ws-git-odd-stage");
    fs::write(dir.join("my file.txt"), "a\n").unwrap();
    git(&dir, &["add", "my file.txt"]);
    git(&dir, &["commit", "-q", "-m", "space"]);
    fs::write(dir.join("my file.txt"), "a\nb\n").unwrap();
    let meta = crate::discovery::RepoCheckoutMeta {
        checkout_kind: crate::snapshot::CheckoutKind::Primary,
        primary_repo: None,
    };
    let parent = dir.parent().unwrap();
    let repo = dir.file_name().unwrap().to_str().unwrap();
    let snap = crate::discovery::process_repo(repo, parent, false, None, &meta).unwrap();
    let paths: Vec<&str> = snap.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["my file.txt"]);

    stage_file(&dir, paths[0]).unwrap();
    assert_eq!(staged_paths(&dir), ["my file.txt"]);
    unstage_file(&dir, paths[0]).unwrap();
    assert!(staged_paths(&dir).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compare_patch_keys_follow_the_file_diff_not_the_commits() {
    let dir = unique_dir("ws-git-patch-keys");
    init_repo(&dir);
    let base = exec_git(&["rev-parse", "HEAD"], &dir);
    fs::write(dir.join("a.txt"), "a\n").unwrap();
    fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["mv", "README.md", "RENAMED.md"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "one"]);
    let head1 = exec_git(&["rev-parse", "HEAD"], &dir);
    let keys1 = list_compare_patch_keys(&dir, &base, &head1);
    assert!(keys1
        .get("RENAMED.md")
        .is_some_and(|k| k.contains("README.md")));

    fs::write(dir.join("b.txt"), "b2\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "two"]);
    let head2 = exec_git(&["rev-parse", "HEAD"], &dir);
    let keys2 = list_compare_patch_keys(&dir, &base, &head2);
    assert_eq!(keys1.get("a.txt"), keys2.get("a.txt"), "untouched file");
    assert_eq!(keys1.get("RENAMED.md"), keys2.get("RENAMED.md"));
    assert!(keys2.contains_key("b.txt"));
    assert_ne!(keys1.get("b.txt"), keys2.get("b.txt"), "edited file");

    fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "revert"]);
    let head3 = exec_git(&["rev-parse", "HEAD"], &dir);
    let keys3 = list_compare_patch_keys(&dir, &base, &head3);
    assert_eq!(keys1.get("b.txt"), keys3.get("b.txt"), "same diff again");

    assert!(list_compare_patch_keys(&dir, "nope", &head3).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compare_patch_keys_spell_paths_like_the_compare_list() {
    let dir = unique_dir("ws-git-patch-keys-quoted");
    init_repo(&dir);
    git(&dir, &["config", "core.quotePath", "true"]);
    let base = exec_git(&["rev-parse", "HEAD"], &dir);
    fs::write(dir.join("é.txt"), "e\n").unwrap();
    fs::write(dir.join("tab\there.txt"), "t\n").unwrap();
    fs::write(dir.join("plain.txt"), "p\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "odd names"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let files = list_compare_name_status(&dir, &base, &head).unwrap();
    let keys = list_compare_patch_keys(&dir, &base, &head);
    assert_eq!(files.len(), 3, "{files:?}");
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["plain.txt", "tab\there.txt", "é.txt"], "raw paths");
    for file in &files {
        assert!(keys.contains_key(&file.path), "{} in {keys:?}", file.path);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parse_raw_patch_keys_reads_plain_and_rename_rows() {
    let z = "1".repeat(40);
    let o = "0".repeat(40);
    let raw = format!(
        ":100644 100644 {o} {z} M\0src/a.rs\0:100644 100644 {o} {z} R087\0old.rs\0new.rs\0\
         :100644 100644 {o} {z} A\0:odd name.rs\0"
    );
    let keys = parse_raw_patch_keys(&raw);
    assert_eq!(keys.len(), 3);
    assert_eq!(
        keys[":odd name.rs"],
        format!("100644 100644 {o} {z} A\0:odd name.rs\0:odd name.rs")
    );
    assert_eq!(
        keys["src/a.rs"],
        format!("100644 100644 {o} {z} M\0src/a.rs\0src/a.rs")
    );
    assert_eq!(
        keys["new.rs"],
        format!("100644 100644 {o} {z} R\0old.rs\0new.rs")
    );
}

#[test]
fn blob_bytes_returns_head_contents_without_utf8_trim() {
    let dir = unique_dir("ws-git-blob");
    init_repo(&dir);
    let payload = b"hello \n\x00\xff";
    fs::write(dir.join("bin.dat"), payload).unwrap();
    git(&dir, &["add", "bin.dat"]);
    git(&dir, &["commit", "-q", "-m", "bin"]);
    assert_eq!(
        blob_bytes(&dir, "HEAD", "bin.dat").as_deref(),
        Some(payload.as_slice())
    );
    assert_eq!(
        blob_bytes(&dir, "HEAD", "README.md").as_deref(),
        Some(b"# seed\n".as_slice())
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn blob_bytes_missing_path_is_none() {
    let dir = unique_dir("ws-git-blob-miss");
    init_repo(&dir);
    assert_eq!(blob_bytes(&dir, "HEAD", "nope.txt"), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compare_wrappers_ahead_behind_diverged_rename_and_failure() {
    let dir = unique_dir("ws-git-compare");
    init_repo(&dir);
    let base = exec_git(&["rev-parse", "HEAD"], &dir);
    fs::write(dir.join("one.txt"), "one\n").unwrap();
    git(&dir, &["add", "one.txt"]);
    git(&dir, &["commit", "-q", "-m", "ahead"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let ahead = list_compare_name_status(&dir, &base, &head).unwrap();
    assert!(ahead.iter().any(|row| row.path == "one.txt"), "{ahead:?}");
    let behind = list_compare_name_status(&dir, &head, &base).unwrap();
    assert!(behind.is_empty(), "{behind:?}");
    git(&dir, &["checkout", "-q", "-b", "topic", &base]);
    fs::write(dir.join("topic.txt"), "topic\n").unwrap();
    git(&dir, &["add", "topic.txt"]);
    git(&dir, &["commit", "-q", "-m", "topic"]);
    git(&dir, &["checkout", "-q", "main"]);
    fs::write(dir.join("main-only.txt"), "main\n").unwrap();
    git(&dir, &["add", "main-only.txt"]);
    git(&dir, &["commit", "-q", "-m", "main-only"]);
    let main = exec_git(&["rev-parse", "HEAD"], &dir);
    let topic = exec_git(&["rev-parse", "topic"], &dir);
    let mb = merge_base(&dir, &main, &topic).unwrap();
    assert_eq!(mb.as_deref(), Some(base.as_str()));
    let diverged = list_compare_name_status(&dir, &topic, &main).unwrap();
    assert!(
        diverged.iter().any(|row| row.path == "main-only.txt"),
        "{diverged:?}"
    );
    git(&dir, &["mv", "one.txt", "renamed.txt"]);
    git(&dir, &["commit", "-q", "-m", "rename"]);
    let after_rename = exec_git(&["rev-parse", "HEAD"], &dir);
    // `main` still has `one.txt`. Three-dot from the initial seed commit
    // sees only an add (`renamed.txt` never existed on that base).
    let renamed = list_compare_name_status(&dir, &main, &after_rename).unwrap();
    assert!(
        renamed
            .iter()
            .any(|row| { row.path == "renamed.txt" && row.old_path.as_deref() == Some("one.txt") }),
        "{renamed:?}"
    );
    assert!(rev_parse_commit(&dir, "no-such-ref").unwrap().is_none());
    assert!(exec_git_stdout(&["this-is-not-a-git-command"], &dir).is_err());
    let _ = fs::remove_dir_all(&dir);
}

/// `main` has `m.txt`, `d.txt`, `old.txt`; `feature` (checked out)
/// modifies `m.txt`, deletes `d.txt`, renames `old.txt` → `new.txt`,
/// and adds `added.txt`. Returns (dir, merge base, head).
fn compare_revert_fixture() -> (std::path::PathBuf, String, String) {
    let dir = unique_dir("ws-git-compare-revert");
    init_repo(&dir);
    fs::write(dir.join("m.txt"), "m-base\n").unwrap();
    fs::write(dir.join("d.txt"), "d-base\n").unwrap();
    fs::write(dir.join("old.txt"), "r1\nr2\nr3\nr4\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    let merge_base = exec_git(&["rev-parse", "HEAD"], &dir);
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    fs::write(dir.join("m.txt"), "m-head\n").unwrap();
    fs::write(dir.join("added.txt"), "added\n").unwrap();
    git(&dir, &["rm", "-q", "d.txt"]);
    git(&dir, &["mv", "old.txt", "new.txt"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "feature"]);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    (dir, merge_base, head)
}

fn index_is_head(dir: &Path) -> bool {
    exec_git_status(&["diff", "--cached", "--quiet"], dir) == 0
}

#[test]
fn revert_compare_file_restores_each_status_from_merge_base() {
    let (dir, mb, head) = compare_revert_fixture();
    let listed = list_compare_name_status(&dir, &mb, &head).unwrap();
    let status = |path: &str| {
        listed
            .iter()
            .find(|row| row.path == path)
            .map(|row| row.status.clone())
    };
    assert_eq!(status("m.txt").as_deref(), Some("M"));
    assert_eq!(status("added.txt").as_deref(), Some("A"));
    assert_eq!(status("d.txt").as_deref(), Some("D"));
    assert_eq!(status("new.txt").as_deref(), Some("R"));

    // M: content back to the merge base.
    revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap();
    assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "m-base\n");
    // A: the file is not in the merge base, so it is deleted.
    revert_compare_file(&dir, &mb, &head, "added.txt", None).unwrap();
    assert!(!dir.join("added.txt").exists());
    // D: the file is written back.
    revert_compare_file(&dir, &mb, &head, "d.txt", None).unwrap();
    assert_eq!(fs::read_to_string(dir.join("d.txt")).unwrap(), "d-base\n");
    // R: old path written back, new path deleted.
    revert_compare_file(&dir, &mb, &head, "new.txt", Some("old.txt")).unwrap();
    assert!(!dir.join("new.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.join("old.txt")).unwrap(),
        "r1\nr2\nr3\nr4\n"
    );

    assert!(
        index_is_head(&dir),
        "compare revert must not touch the index"
    );
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), head);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn revert_compare_file_aborts_when_head_moved_or_path_dirty() {
    let (dir, mb, head) = compare_revert_fixture();

    // Unstaged edit.
    fs::write(dir.join("m.txt"), "local edit\n").unwrap();
    let err = revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap_err();
    assert!(err.contains("m.txt has uncommitted changes"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.join("m.txt")).unwrap(),
        "local edit\n"
    );
    revert_tracked_file(&dir, "m.txt").unwrap();

    // Staged-only edit.
    fs::write(dir.join("m.txt"), "staged edit\n").unwrap();
    stage_file(&dir, "m.txt").unwrap();
    fs::write(dir.join("m.txt"), "m-head\n").unwrap();
    let err = revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap_err();
    assert!(err.contains("uncommitted"), "{err}");
    unstage_file(&dir, "m.txt").unwrap();

    // Untracked file at a rename's old path would be overwritten.
    fs::write(dir.join("old.txt"), "mine\n").unwrap();
    let err = revert_compare_file(&dir, &mb, &head, "new.txt", Some("old.txt")).unwrap_err();
    assert!(err.contains("uncommitted"), "{err}");
    assert_eq!(fs::read_to_string(dir.join("old.txt")).unwrap(), "mine\n");
    assert!(dir.join("new.txt").exists());
    fs::remove_file(dir.join("old.txt")).unwrap();

    // HEAD moved since the compare loaded.
    fs::write(dir.join("other.txt"), "x\n").unwrap();
    git(&dir, &["add", "other.txt"]);
    git(&dir, &["commit", "-q", "-m", "moved"]);
    let err = revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap_err();
    assert!(err.contains("HEAD moved"), "{err}");
    assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "m-head\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn revert_compare_patch_checks_before_apply() {
    let (dir, mb, head) = compare_revert_fixture();
    let patch = diff_compare_file_ctx(&dir, &mb, &head, "m.txt", None, None)
        .unwrap()
        .join("\n")
        + "\n";
    fs::write(dir.join("m.txt"), "dirty\n").unwrap();
    let err = revert_compare_patch(&dir, &head, "m.txt", &patch).unwrap_err();
    assert!(err.contains("uncommitted"), "{err}");
    assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "dirty\n");
    revert_tracked_file(&dir, "m.txt").unwrap();
    revert_compare_patch(&dir, &head, "m.txt", &patch).unwrap();
    assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "m-base\n");
    assert!(index_is_head(&dir));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compare_picker_drops_origin_head_and_current_local() {
    let raw = "main\t1\t*\norigin/HEAD\t2\t\norigin/main\t3\t\nfeature\t4\t\n";
    let names: Vec<_> = parse_compare_picker_branches(raw)
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(names, vec!["origin/main", "feature"]);
}

#[test]
fn compare_picker_commits_list_ancestors_without_head() {
    let dir = unique_dir("ws-git-picker-commits");
    init_repo(&dir);
    let rev = |r: &str| rev_parse_commit(&dir, r).unwrap().unwrap();
    // Root HEAD has no ancestors.
    assert_eq!(list_compare_picker_commits(&dir).unwrap(), Vec::new());
    let root = rev("HEAD");

    git(&dir, &["checkout", "-q", "-b", "side"]);
    fs::write(dir.join("side.txt"), "side\n").unwrap();
    git(&dir, &["add", "side.txt"]);
    git(&dir, &["commit", "-q", "-m", "Side\twork"]);
    let side = rev("HEAD");
    git(&dir, &["checkout", "-q", "main"]);
    fs::write(dir.join("main.txt"), "main\n").unwrap();
    git(&dir, &["add", "main.txt"]);
    git(&dir, &["commit", "-q", "-m", "Main work"]);
    let main = rev("HEAD");
    git(
        &dir,
        &["merge", "-q", "--no-ff", "-m", "Merge side", "side"],
    );
    let head = rev("HEAD");

    let rows = list_compare_picker_commits(&dir).unwrap();
    let mut ids: Vec<_> = rows.iter().map(|c| c.id.as_str()).collect();
    assert!(!ids.contains(&head.as_str()), "HEAD is excluded: {ids:?}");
    ids.sort_unstable();
    let mut want = vec![root.as_str(), side.as_str(), main.as_str()];
    want.sort_unstable();
    assert_eq!(ids, want, "first- and merge-side ancestors");
    let side_row = rows.iter().find(|c| c.id == side).unwrap();
    assert_eq!(side_row.subject, "Side\twork", "a tab stays in the subject");

    // The cap keeps the newest `limit` ancestors and still drops HEAD.
    let capped = list_compare_picker_commits_capped(&dir, 1).unwrap();
    assert_eq!(capped.len(), 1);
    assert_eq!(capped[0].id, rows[0].id);
    assert_ne!(capped[0].id, head);
    assert_eq!(
        list_compare_picker_commits_capped(&dir, 0).unwrap(),
        Vec::new()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn checked_git_errors_carry_the_git_reason_line() {
    let dir = unique_dir("ws-git-reason");
    init_repo(&dir);
    let err = stage_file(&dir, "nope.txt").unwrap_err();
    assert_eq!(err, "pathspec 'nope.txt' did not match any files");
    // The literal pathspec prefix stays out of the reason line, glob characters included.
    let err = stage_file(&dir, "nope[1].txt").unwrap_err();
    assert_eq!(err, "pathspec 'nope[1].txt' did not match any files");
    let err = stash_pop(&dir, "stash@{0}").unwrap_err();
    assert!(err.contains("stash@{0} is not a valid reference"), "{err}");
    assert!(!err.starts_with("error:"), "tag is stripped: {err}");

    // `stash apply` prints its conflict on stdout, not stderr.
    fs::write(dir.join("README.md"), "# stashed\n").unwrap();
    git(&dir, &["stash", "-q"]);
    fs::write(dir.join("README.md"), "# committed\n").unwrap();
    git(&dir, &["commit", "-qam", "conflicting"]);
    let err = stash_apply(&dir, "stash@{0}").unwrap_err();
    assert!(err.starts_with("CONFLICT"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn failed_patch_apply_reports_one_reason_line() {
    let dir = unique_dir("ws-git-patch-reason");
    init_repo(&dir);
    // git prints two `error:` lines for a patch that does not apply.
    let patch = "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n\
                     @@ -1 +1 @@\n-# other\n+# new\n";
    let err = apply_cached_patch(&dir, patch, false).unwrap_err();
    assert_eq!(err, "patch failed: README.md:1");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pull_reports_an_auto_stash_conflict_and_keeps_the_stash() {
    let (root, up, down) = pull_fixture("ws-git-pull-conflict");
    fs::write(up.join("README.md"), "# upstream\n").unwrap();
    git(&up, &["commit", "-qam", "upstream"]);
    fs::write(down.join("README.md"), "# local edit\n").unwrap();

    let result = pull_quiet_detailed(&down);
    assert!(result.stashed && result.stash_pop_failed && !result.ok);
    assert_eq!(result.error, None, "the pull itself worked");
    assert_eq!(list_stash_refs(&down).len(), 1, "stash entry is kept");
    let _ = fs::remove_dir_all(&root);
}

/// `stash@{0}` as `<sha> <subject>` lines, newest first.
fn stash_entries(dir: &Path) -> Vec<String> {
    exec_git(&["stash", "list", "--format=%H %s"], dir)
        .lines()
        .map(str::to_string)
        .collect()
}

/// Make one user stash entry in `down` that edits `README.md`, and return
/// [`stash_entries`].
fn push_user_stash(down: &Path) -> Vec<String> {
    fs::write(down.join("README.md"), "# user work\n").unwrap();
    git(down, &["stash", "push", "-q", "-m", "user work"]);
    let entries = stash_entries(down);
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries
}

#[test]
fn pull_with_only_a_dirty_submodule_leaves_the_user_stash_alone() {
    let (root, up, down) = pull_fixture("ws-git-pull-submodule");
    let sub = root.join("sub");
    init_repo(&sub);
    git(
        &up,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            sub.to_str().unwrap(),
            "sub",
        ],
    );
    git(&up, &["commit", "-qm", "add submodule"]);
    git(&down, &["pull", "-q"]);
    git(
        &down,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "-q",
        ],
    );
    let user_stash = push_user_stash(&down);
    // A tracked edit inside the submodule: `diff --quiet` exits 1, but
    // `stash push` saves nothing for the parent repo.
    fs::write(down.join("sub/README.md"), "# dirty submodule\n").unwrap();
    fs::write(up.join("upstream.txt"), "upstream\n").unwrap();
    git(&up, &["add", "upstream.txt"]);
    git(&up, &["commit", "-qm", "upstream"]);

    let result = pull_quiet_detailed(&down);
    assert!(result.ok, "{result:?}");
    assert!(!result.stashed, "no auto-stash entry was made: {result:?}");
    assert!(down.join("upstream.txt").exists(), "the pull ran");
    assert_eq!(stash_entries(&down), user_stash, "user stash is untouched");
    assert_eq!(
        fs::read_to_string(down.join("README.md")).unwrap(),
        "# seed\n",
        "the user stash was not applied"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn pull_auto_stash_pops_its_own_entry_and_keeps_the_user_stash() {
    let (root, up, down) = pull_fixture("ws-git-pull-own-stash");
    let user_stash = push_user_stash(&down);
    fs::write(down.join("README.md"), "# local edit\n").unwrap();
    fs::write(up.join("upstream.txt"), "upstream\n").unwrap();
    git(&up, &["add", "upstream.txt"]);
    git(&up, &["commit", "-qm", "upstream"]);

    let result = pull_quiet_detailed(&down);
    assert!(result.ok && result.stashed, "{result:?}");
    assert!(!result.stash_pop_failed, "{result:?}");
    assert!(down.join("upstream.txt").exists(), "the pull ran");
    assert_eq!(
        fs::read_to_string(down.join("README.md")).unwrap(),
        "# local edit\n",
        "the auto-stash came back"
    );
    assert_eq!(stash_entries(&down), user_stash, "user stash is untouched");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn pull_failure_carries_the_git_reason() {
    let dir = unique_dir("ws-git-pull-fail");
    init_repo(&dir);
    let result = pull_quiet_detailed(&dir);
    assert!(!result.ok && !result.stash_pop_failed);
    let reason = result.error.expect("reason");
    assert!(
        !reason.is_empty() && !reason.contains("exited with code"),
        "{reason}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// First 12 hex chars of a `<sha> <subject>` line of [`stash_entries`].
fn entry_short(line: &str) -> &str {
    &line[..12]
}

#[cfg(unix)]
#[test]
fn pull_gives_back_an_auto_stash_whose_push_failed_after_saving() {
    use std::os::unix::fs::PermissionsExt;
    let (root, up, down) = pull_fixture("ws-git-pull-push-failed");
    fs::create_dir_all(up.join("d")).unwrap();
    fs::write(up.join("d/f"), "base\n").unwrap();
    git(&up, &["add", "d/f"]);
    git(&up, &["commit", "-qm", "add d/f"]);
    git(&down, &["pull", "-q"]);
    let user_stash = push_user_stash(&down);
    fs::write(up.join("upstream.txt"), "upstream\n").unwrap();
    git(&up, &["add", "upstream.txt"]);
    git(&up, &["commit", "-qm", "upstream"]);
    // `stash push` saves its entry, then cannot reset `d/f` in a read-only
    // folder and exits 1.
    fs::write(down.join("d/f"), "local\n").unwrap();
    let dir = down.join("d");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(dir.join("probe"), "").is_ok() {
        // Root ignores the mode: this host cannot make the push fail.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fs::remove_dir_all(&root);
        return;
    }

    let result = pull_quiet_detailed(&down);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let entries = stash_entries(&down);
    assert!(result.stashed && !result.ok, "{result:?}");
    let error = result.error.clone().unwrap_or_default();
    assert!(error.starts_with("auto-stash failed: "), "{result:?}");
    assert!(!down.join("upstream.txt").exists(), "no pull over the tree");
    // The pop ran and git refused it over the file the push did not reset:
    // the entry is kept and the note names it.
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(entries[1], user_stash[0], "user stash is untouched");
    assert!(result.stash_pop_failed, "{result:?}");
    let note = result.stash_note.clone().unwrap_or_default();
    assert!(
        note.starts_with("restoring the auto-stash failed: "),
        "{note}"
    );
    assert!(
        note.contains(&format!("`git stash list` as {}", entry_short(&entries[0]))),
        "{note}"
    );
    assert_eq!(fs::read_to_string(down.join("d/f")).unwrap(), "local\n");
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn pull_keeps_the_auto_stash_when_a_newer_stash_lands_during_the_pull() {
    use std::os::unix::fs::PermissionsExt;
    let (root, up, down) = pull_fixture("ws-git-pull-newer-stash");
    let user_stash = push_user_stash(&down);
    // A stash made in another terminal while the pull runs: the post-merge
    // hook stashes a new file right after the merge.
    let hooks = root.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("post-merge");
    fs::write(
        &hook,
        "#!/bin/sh\necho newer > newer.txt && git add newer.txt && git stash push -q -m newer\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &down,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    fs::write(down.join("README.md"), "# local edit\n").unwrap();
    fs::write(up.join("upstream.txt"), "upstream\n").unwrap();
    git(&up, &["add", "upstream.txt"]);
    git(&up, &["commit", "-qm", "upstream"]);

    let result = pull_quiet_detailed(&down);
    let entries = stash_entries(&down);
    assert!(down.join("upstream.txt").exists(), "the pull ran");
    assert_eq!(result.error, None, "{result:?}");
    assert!(
        result.stashed && result.stash_pop_failed && !result.ok,
        "{result:?}"
    );
    assert_eq!(entries.len(), 3, "{entries:?}");
    assert!(entries[0].ends_with(" newer"), "{entries:?}");
    assert!(entries[1].ends_with(AUTO_STASH_MESSAGE), "{entries:?}");
    assert_eq!(entries[2], user_stash[0], "user stash is untouched");
    let note = result.stash_note.unwrap_or_default();
    assert!(
        note.contains("a newer stash entry is on top of it"),
        "{note}"
    );
    assert!(
        note.contains(&format!("`git stash list` as {}", entry_short(&entries[1]))),
        "{note}"
    );
    assert_eq!(
        fs::read_to_string(down.join("README.md")).unwrap(),
        "# seed\n",
        "neither stash was applied"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn push_on_detached_head_says_why() {
    let dir = unique_dir("ws-git-push-detached");
    init_repo(&dir);
    git(&dir, &["checkout", "-q", "--detach"]);
    assert_eq!(push_quiet(&dir).unwrap_err(), "detached HEAD cannot push");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn push_plan_names_branch_remote_and_first_publish() {
    let root = unique_dir("ws-git-push-plan");
    let up = root.join("up");
    let down = root.join("down");
    init_repo(&up);
    git(
        &root,
        &["clone", "-q", up.to_str().unwrap(), down.to_str().unwrap()],
    );
    git(&down, &["config", "user.name", "workspace-status test"]);
    git(
        &down,
        &[
            "config",
            "user.email",
            "workspace-status-test@example.invalid",
        ],
    );
    fs::write(down.join("README.md"), "# ahead\n").unwrap();
    git(&down, &["commit", "-qam", "ahead"]);
    assert_eq!(
        push_plan(&down),
        Ok(PushPlan {
            branch: "main".into(),
            remote: "origin".into(),
            publish: false,
        })
    );

    git(&down, &["checkout", "-q", "-b", "feature/x"]);
    assert_eq!(
        push_plan(&down),
        Ok(PushPlan {
            branch: "feature/x".into(),
            remote: "origin".into(),
            publish: true,
        })
    );

    git(&down, &["checkout", "-q", "--detach"]);
    assert_eq!(push_plan(&down), Err("detached HEAD cannot push".into()));
    let _ = fs::remove_dir_all(&root);
}

fn commit_file(dir: &std::path::Path, path: &str, text: &str, msg: &str) -> String {
    fs::write(dir.join(path), text).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
    exec_git(&["rev-parse", "HEAD"], dir)
}

fn blame(dir: &std::path::Path, rev: BlameRev, path: &str, line: u32) -> LineBlame {
    blame_line(dir, &rev, path, line)
        .expect("git starts")
        .unwrap_or_else(|| panic!("blame {path}:{line}"))
}

#[test]
fn parse_blame_porcelain_reads_headers_previous_and_boundary() {
    let renamed = "\
63b08ad4823cc8096b108736af6268306d1c5328 2 2 1
author Ada Lovelace
author-mail <ada@example.invalid>
author-time 1791204822
author-tz +0000
summary rename and fix
previous 3ef72304f100a91bb7f85a6f16253820cd51d07d old name.txt
filename new.txt
\tB
";
    let blame = parse_blame_porcelain(renamed).expect("parses");
    assert_eq!(blame.sha, "63b08ad4823cc8096b108736af6268306d1c5328");
    assert_eq!(blame.author, "Ada Lovelace");
    assert_eq!(blame.author_time, 1_791_204_822);
    assert_eq!(blame.summary, "rename and fix");
    assert_eq!(blame.orig_line, 2);
    assert_eq!(blame.filename, "new.txt");
    assert_eq!(
        blame.previous,
        Some((
            "3ef72304f100a91bb7f85a6f16253820cd51d07d".into(),
            "old name.txt".into()
        ))
    );
    assert!(!blame.boundary && !blame.uncommitted);

    let root = "3ef72304f100a91bb7f85a6f16253820cd51d07d 1 1 1\nauthor T\nsummary first\nboundary\nfilename f\n\ta\n";
    let root = parse_blame_porcelain(root).expect("parses");
    assert!(root.boundary && root.previous.is_none());

    let zero = format!(
        "{} 4 4 1\nauthor Not Committed Yet\nfilename f\n\tz\n",
        "0".repeat(40)
    );
    assert!(parse_blame_porcelain(&zero).expect("parses").uncommitted);

    assert_eq!(parse_blame_porcelain(""), None);
    assert_eq!(parse_blame_porcelain("not-a-sha 1 1 1\nfilename f\n"), None);
    assert_eq!(
        parse_blame_porcelain("3ef72304f100a91bb7f85a6f16253820cd51d07d 1 1 1\n\ta\n"),
        None,
        "no filename"
    );
}

#[test]
fn blame_line_reads_commit_worktree_and_index_versions() {
    let dir = unique_dir("ws-git-blame");
    init_repo(&dir);
    let first = commit_file(&dir, "f.txt", "a\nb\nc\n", "first");

    let committed = blame(&dir, BlameRev::Worktree, "f.txt", 2);
    assert_eq!(committed.sha, first);
    assert_eq!(committed.summary, "first");
    assert_eq!(committed.author, "workspace-status test");
    assert_eq!(
        (committed.orig_line, committed.filename.as_str()),
        (2, "f.txt")
    );
    assert!(!committed.boundary && !committed.uncommitted);
    assert!(committed.author_time > 0);

    // A worktree edit is uncommitted; the index and HEAD still blame `first`.
    fs::write(dir.join("f.txt"), "a\nB\nc\n").unwrap();
    assert!(blame(&dir, BlameRev::Worktree, "f.txt", 2).uncommitted);
    assert_eq!(blame(&dir, BlameRev::Index, "f.txt", 2).sha, first);
    assert_eq!(
        blame(&dir, BlameRev::Commit("HEAD".into()), "f.txt", 2).sha,
        first
    );

    // Staged, the index copy differs from HEAD; line 1 still matches.
    git(&dir, &["add", "f.txt"]);
    fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
    assert!(blame(&dir, BlameRev::Index, "f.txt", 2).uncommitted);
    assert_eq!(blame(&dir, BlameRev::Index, "f.txt", 1).sha, first);
    assert!(!blame(&dir, BlameRev::Worktree, "f.txt", 2).uncommitted);

    // The seed commit is the root: porcelain marks it `boundary`.
    let root = blame(&dir, BlameRev::Commit(first.clone()), "README.md", 1);
    assert!(root.boundary && root.previous.is_none(), "{root:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn blame_line_is_none_when_git_has_no_answer() {
    let dir = unique_dir("ws-git-blame-none");
    init_repo(&dir);
    fs::write(dir.join("untracked.txt"), "u\n").unwrap();
    for (rev, path, line) in [
        (BlameRev::Worktree, "untracked.txt", 1),
        (BlameRev::Index, "untracked.txt", 1),
        (BlameRev::Worktree, "missing.txt", 1),
        (BlameRev::Worktree, "README.md", 9),
        (BlameRev::Worktree, "README.md", 0),
        (BlameRev::Commit("no-such-ref".into()), "README.md", 1),
    ] {
        assert_eq!(
            blame_line(&dir, &rev, path, line),
            Ok(None),
            "{rev:?} {path}:{line}"
        );
    }
    let unborn = unique_dir("ws-git-blame-unborn");
    init_repo_empty(&unborn);
    fs::write(unborn.join("f.txt"), "a\n").unwrap();
    git(&unborn, &["add", "f.txt"]);
    assert_eq!(
        blame_line(&unborn, &BlameRev::Worktree, "f.txt", 1),
        Ok(None)
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&unborn);
}

#[test]
fn blame_line_follows_a_rename_through_previous() {
    let dir = unique_dir("ws-git-blame-rename");
    init_repo(&dir);
    let first = commit_file(&dir, "old.txt", "a\nb\n", "first");
    git(&dir, &["mv", "old.txt", "new.txt"]);
    let second = commit_file(&dir, "new.txt", "a\nB\n", "rename and fix");
    let changed = blame(&dir, BlameRev::Worktree, "new.txt", 2);
    assert_eq!(changed.sha, second);
    assert_eq!(changed.filename, "new.txt");
    assert_eq!(changed.previous, Some((first.clone(), "old.txt".into())));
    let kept = blame(&dir, BlameRev::Worktree, "new.txt", 1);
    assert_eq!(
        (kept.sha.as_str(), kept.filename.as_str()),
        (first.as_str(), "old.txt")
    );
    // The previous change of the renamed line is `first`, under the old name.
    match previous_line_change(&dir, &changed).unwrap() {
        PreviousLineChange::Found(earlier) => {
            assert_eq!(earlier.sha, first);
            assert_eq!(
                (earlier.filename.as_str(), earlier.orig_line),
                ("old.txt", 2)
            );
        }
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn previous_line_change_finds_the_older_edit_or_the_adding_commit() {
    let dir = unique_dir("ws-git-blame-previous");
    init_repo(&dir);
    commit_file(&dir, "f.txt", "a\nb\nc\n", "first");
    let second = commit_file(&dir, "f.txt", "a\nB\nc\n", "second");
    // `third` inserts a line above, edits B again, and appends `d`.
    let third = commit_file(&dir, "f.txt", "top\na\nBB\nc\nd\n", "third");

    let line = blame(&dir, BlameRev::Worktree, "f.txt", 3);
    assert_eq!((line.sha.as_str(), line.orig_line), (third.as_str(), 3));
    match previous_line_change(&dir, &line).unwrap() {
        PreviousLineChange::Found(earlier) => {
            assert_eq!(
                (earlier.sha.as_str(), earlier.orig_line),
                (second.as_str(), 2)
            );
            assert!(!earlier.boundary);
        }
        other => panic!("{other:?}"),
    }

    let appended = blame(&dir, BlameRev::Worktree, "f.txt", 5);
    assert_eq!(appended.sha, third);
    assert_eq!(
        previous_line_change(&dir, &appended),
        Ok(PreviousLineChange::AddedIn)
    );

    let new_file_sha = commit_file(&dir, "g.txt", "g\n", "add g");
    let new_file = blame(&dir, BlameRev::Worktree, "g.txt", 1);
    assert_eq!(
        (new_file.sha.as_str(), new_file.previous.as_ref()),
        (new_file_sha.as_str(), None)
    );
    assert_eq!(
        previous_line_change(&dir, &new_file),
        Ok(PreviousLineChange::AddedIn)
    );

    // Editing the seed line: the earlier change is the root commit.
    commit_file(&dir, "README.md", "# edited\n", "edit readme");
    let readme = blame(&dir, BlameRev::Worktree, "README.md", 1);
    match previous_line_change(&dir, &readme).unwrap() {
        PreviousLineChange::Found(earlier) => assert!(earlier.boundary, "{earlier:?}"),
        other => panic!("{other:?}"),
    }

    fs::write(dir.join("f.txt"), "top\na\nzz\nc\nd\n").unwrap();
    let dirty = blame(&dir, BlameRev::Worktree, "f.txt", 3);
    assert_eq!(
        previous_line_change(&dir, &dirty),
        Err("line is not committed yet".into())
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn map_line_to_parent_follows_u0_hunks() {
    // Line 2 modified in place.
    let modified = "@@ -2 +2 @@\n-b\n+B\n";
    assert_eq!(map_line_to_parent(modified, 2), Some(2));
    assert_eq!(map_line_to_parent(modified, 1), Some(1));
    assert_eq!(map_line_to_parent(modified, 3), Some(3));
    // Lines 2..3 became 2..5: only the first two map back.
    let grown = "@@ -2,2 +2,4 @@\n-b\n-c\n+B\n+C\n+D\n+E\n";
    assert_eq!(map_line_to_parent(grown, 3), Some(3));
    assert_eq!(map_line_to_parent(grown, 4), None);
    assert_eq!(map_line_to_parent(grown, 6), Some(4), "shifted by -2");
    // Pure insertion after line 1, then a pure deletion after new line 5.
    let mixed = "@@ -1,0 +2,2 @@\n+x\n+y\n@@ -4,3 +5,0 @@\n-p\n-q\n-r\n";
    assert_eq!(map_line_to_parent(mixed, 1), Some(1));
    assert_eq!(map_line_to_parent(mixed, 2), None);
    assert_eq!(map_line_to_parent(mixed, 3), None);
    assert_eq!(map_line_to_parent(mixed, 4), Some(2));
    assert_eq!(map_line_to_parent(mixed, 5), Some(3));
    assert_eq!(map_line_to_parent(mixed, 6), Some(7));
    assert_eq!(map_line_to_parent("", 4), Some(4));
}

#[test]
fn git_worktree_compare_diff_args_adds_context_and_rename_path() {
    assert_eq!(
        git_worktree_compare_diff_args("abc", "new.txt", Some("old.txt"), Some(3)),
        ["diff", "-U3", "-M", "abc", "--", "old.txt", "new.txt"]
    );
    assert_eq!(
        git_worktree_compare_diff_args("abc", "f.txt", Some("f.txt"), None),
        ["diff", "-M", "abc", "--", "f.txt"]
    );
}

#[test]
fn worktree_vs_commit_lists_and_diffs_one_path() {
    let dir = unique_dir("ws-git-worktree-compare");
    init_repo(&dir);
    let base = commit_file(&dir, "old.txt", "one\ntwo\nthree\n", "base");
    commit_file(&dir, "f.txt", "x\n", "later");

    let clean = list_worktree_vs_commit_name_status(&dir, &base, "old.txt", None).unwrap();
    assert!(clean.is_empty(), "{clean:?}");
    assert_eq!(
        diff_worktree_vs_commit_file_ctx(&dir, &base, "old.txt", None, None).unwrap(),
        ["(no diff)"]
    );

    // Rename in the index, edit in the worktree only: both count.
    git(&dir, &["mv", "old.txt", "new.txt"]);
    fs::write(dir.join("new.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    let rows =
        list_worktree_vs_commit_name_status(&dir, &base, "new.txt", Some("old.txt")).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].status, "R");
    assert_eq!(rows[0].path, "new.txt");
    assert_eq!(rows[0].old_path.as_deref(), Some("old.txt"));
    assert_eq!(rows[0].stat, counts(1, 0));
    let diff =
        diff_worktree_vs_commit_file_ctx(&dir, &base, "new.txt", Some("old.txt"), Some(0)).unwrap();
    assert!(diff.iter().any(|l| l == "rename from old.txt"), "{diff:?}");
    assert!(diff.iter().any(|l| l == "+four"), "{diff:?}");

    let added = list_worktree_vs_commit_name_status(&dir, &base, "f.txt", None).unwrap();
    assert_eq!(added[0].status, "A", "f.txt is newer than base: {added:?}");
    assert!(diff_worktree_vs_commit_file_ctx(&dir, "no-such-ref", "f.txt", None, None).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unquote_c_path_decodes_git_quoting() {
    assert_eq!(unquote_c_path(r#""na\303\257ve.txt""#), "naïve.txt");
    assert_eq!(unquote_c_path(r#""a \"q\" b\\c\td""#), "a \"q\" b\\c\td");
    assert_eq!(unquote_c_path("plain name.txt"), "plain name.txt");
    assert_eq!(unquote_c_path("naïve.txt"), "naïve.txt");
    assert_eq!(unquote_c_path_bytes(r#""my notes.txt""#), b"my notes.txt");
    assert_eq!(unquote_c_path_bytes(r#""\303\251.rs""#), "é.rs".as_bytes());
    // A lone invalid byte stays raw (no lossy replacement).
    assert_eq!(unquote_c_path_bytes(r#""a\377""#), b"a\xff");
    assert_eq!(unquote_c_path_bytes("plain \"x.rs"), b"plain \"x.rs");
    let porcelain = "63b08ad4823cc8096b108736af6268306d1c5328 1 1 1\n\
            previous 3ef72304f100a91bb7f85a6f16253820cd51d07d \"old\\303\\251.txt\"\n\
            filename \"na\\303\\257ve.txt\"\n\tx\n";
    let blame = parse_blame_porcelain(porcelain).expect("parses");
    assert_eq!(blame.filename, "naïve.txt");
    assert_eq!(
        blame.previous.map(|(_, path)| path).as_deref(),
        Some("oldé.txt")
    );
}

/// Non-ASCII, a space, and a `"` (git quotes that one even with
/// `core.quotePath=false`) survive blame and the previous-change walk.
#[cfg(unix)]
#[test]
fn blame_line_unquotes_non_ascii_and_quoted_paths() {
    let dir = unique_dir("ws-git-blame-quoted");
    init_repo(&dir);
    let old = "old \"q\" näme.txt";
    let new = "naïve file.txt";
    let first = commit_file(&dir, old, "a\nb\n", "first");
    assert_eq!(blame(&dir, BlameRev::Worktree, old, 2).filename, old);
    git(&dir, &["mv", old, new]);
    let second = commit_file(&dir, new, "a\nB\n", "rename");
    let changed = blame(&dir, BlameRev::Worktree, new, 2);
    assert_eq!(
        (changed.sha.as_str(), changed.filename.as_str()),
        (second.as_str(), new)
    );
    assert_eq!(changed.previous, Some((first.clone(), old.to_string())));
    assert_eq!(blame(&dir, BlameRev::Index, new, 1).filename, old);
    match previous_line_change(&dir, &changed).unwrap() {
        PreviousLineChange::Found(earlier) => {
            assert_eq!(earlier.sha, first);
            assert_eq!((earlier.filename.as_str(), earlier.orig_line), (old, 2));
        }
        other => panic!("{other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Git exits on an unborn HEAD before it reads `--contents -`; the
/// broken pipe is not an error.
#[test]
fn index_blame_on_unborn_head_with_a_large_blob_is_none() {
    let dir = unique_dir("ws-git-blame-epipe");
    init_repo_empty(&dir);
    let big: String = format!("{}\n", "a".repeat(79)).repeat(50_000);
    fs::write(dir.join("big.txt"), big).unwrap();
    git(&dir, &["add", "big.txt"]);
    assert_eq!(blame_line(&dir, &BlameRev::Index, "big.txt", 1), Ok(None));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remote_url_for_branch_follows_branch_remote_then_first_remote() {
    let dir = unique_dir("ws-git-remote-url");
    init_repo(&dir);
    assert_eq!(remote_url_for_branch(&dir, "main"), None, "no remote yet");

    // Not github.com: hosts (Claude Code cloud sessions among them) can
    // inject `url.<base>.insteadOf` rewrites through `GIT_CONFIG_COUNT`,
    // which `git remote get-url` honors and `git_env` does not clear.
    git(
        &dir,
        &[
            "remote",
            "add",
            "origin",
            "git@example.invalid:octo/demo.git",
        ],
    );
    git(
        &dir,
        &[
            "remote",
            "add",
            "upstream",
            "https://gitlab.com/group/sub/demo.git",
        ],
    );
    assert_eq!(
        remote_url_for_branch(&dir, "main").as_deref(),
        Some("git@example.invalid:octo/demo.git"),
        "no branch remote: first remote"
    );

    git(&dir, &["config", "branch.main.remote", "upstream"]);
    assert_eq!(
        remote_url_for_branch(&dir, "main").as_deref(),
        Some("https://gitlab.com/group/sub/demo.git"),
        "branch.<b>.remote wins"
    );

    git(&dir, &["config", "branch.topic.remote", "gone"]);
    assert_eq!(
        remote_url_for_branch(&dir, "topic"),
        None,
        "configured remote that does not exist"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A repo with a local bare `origin` (by file path) that has `main`
/// pushed. Returns `(repo, bare)`.
fn repo_with_bare_origin(prefix: &str) -> (PathBuf, PathBuf) {
    let dir = unique_dir(prefix);
    let bare = unique_dir(&format!("{prefix}-bare"));
    init_repo(&dir);
    git(
        &dir,
        &["init", "-q", "--bare", bare.to_str().expect("utf-8 path")],
    );
    git(
        &dir,
        &[
            "remote",
            "add",
            "origin",
            bare.to_str().expect("utf-8 path"),
        ],
    );
    git(&dir, &["push", "-q", "-u", "origin", "main"]);
    (dir, bare)
}

#[test]
fn remote_view_remote_names_follow_branch_then_current_then_first() {
    let dir = unique_dir("ws-git-remote-view-names");
    init_repo(&dir);
    assert_eq!(checkout_remote_name(&dir), "origin", "no remote: origin");
    git(&dir, &["remote", "add", "team", "/srv/git/team.git"]);
    // `git remote add` refuses a name nested under another remote, but a
    // hand-written config can still hold both.
    git(&dir, &["config", "remote.team/a.url", "/srv/git/a.git"]);
    assert_eq!(checkout_remote_name(&dir), "team", "first remote");
    git(&dir, &["config", "branch.main.remote", "team/a"]);
    assert_eq!(
        checkout_remote_name(&dir),
        "team/a",
        "current branch remote"
    );
    git(&dir, &["checkout", "-q", "--detach"]);
    assert_eq!(checkout_remote_name(&dir), "team", "detached: first remote");

    assert_eq!(
        split_remote_ref(&dir, "team/a/feature/x"),
        Some(("team/a".into(), "feature/x".into())),
        "longest remote name wins"
    );
    assert_eq!(
        split_remote_ref(&dir, "team/feature"),
        Some(("team".into(), "feature".into()))
    );
    assert_eq!(split_remote_ref(&dir, "origin/main"), None);
    assert_eq!(split_remote_ref(&dir, "team/"), None);
    assert_eq!(
        remote_url(&dir, "team").as_deref(),
        Some("/srv/git/team.git")
    );
    assert_eq!(remote_url(&dir, "gone"), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remote_view_branch_presence() {
    let (dir, bare) = repo_with_bare_origin("ws-git-remote-view-branch");
    assert!(remote_branch_exists(&dir, "origin", "main"), "pushed");
    assert!(!remote_branch_exists(&dir, "upstream", "main"), "no remote");

    git(&dir, &["branch", "local-only"]);
    assert!(
        !remote_branch_exists(&dir, "origin", "local-only"),
        "unpushed"
    );
    git(&dir, &["push", "-q", "origin", "local-only:feature/x"]);
    assert!(remote_branch_exists(&dir, "origin", "feature/x"));
    assert!(
        !remote_branch_exists(&dir, "origin", "local-only"),
        "pushed under another name only"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&bare);
}

#[test]
fn remote_view_commit_and_path_presence() {
    let (dir, bare) = repo_with_bare_origin("ws-git-remote-view-commit");
    let pushed = exec_git(&["rev-parse", "HEAD"], &dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/new.rs"), "fn main() {}\n").unwrap();
    git(&dir, &["add", "src/new.rs"]);
    git(&dir, &["commit", "-q", "-m", "local"]);
    let local = exec_git(&["rev-parse", "HEAD"], &dir);

    assert!(commit_on_remote(&dir, "origin", &pushed), "pushed commit");
    assert!(!commit_on_remote(&dir, "origin", &local), "local commit");
    assert!(!commit_on_remote(&dir, "originx", &pushed), "other remote");
    assert!(
        !commit_on_remote(&dir, "origin", "not-a-sha"),
        "unknown rev"
    );

    let remote_main = "refs/remotes/origin/main";
    assert!(rev_has_path(&dir, remote_main, "README.md"), "file");
    assert!(rev_has_path(&dir, remote_main, ""), "root tree");
    assert!(
        !rev_has_path(&dir, remote_main, "src/new.rs"),
        "missing file"
    );
    assert!(!rev_has_path(&dir, remote_main, "src"), "missing dir");
    assert!(rev_has_path(&dir, &local, "src"), "dir at the local commit");
    assert!(rev_has_path(&dir, &local, "src/new.rs"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&bare);
}

#[test]
fn remote_view_tag_presence_asks_the_remote() {
    let (dir, bare) = repo_with_bare_origin("ws-git-remote-view-tag");
    git(&dir, &["tag", "v1"]);
    git(&dir, &["tag", "-a", "v2", "-m", "annotated"]);
    git(&dir, &["push", "-q", "origin", "v1", "v2"]);
    git(&dir, &["tag", "local-only"]);

    assert_eq!(remote_has_tag(&dir, "origin", "v1"), Some(true));
    assert_eq!(
        remote_has_tag(&dir, "origin", "v2"),
        Some(true),
        "annotated"
    );
    assert_eq!(remote_has_tag(&dir, "origin", "local-only"), Some(false));
    assert_eq!(
        remote_has_tag(&dir, "origin", "v"),
        Some(false),
        "no prefix match"
    );
    git(
        &dir,
        &["remote", "add", "gone", "/nonexistent/ws-remote.git"],
    );
    assert_eq!(remote_has_tag(&dir, "gone", "v1"), None, "check failed");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&bare);
}

#[test]
fn commit_times_since_lists_recent_local_commits() {
    let dir = unique_dir("ws-git-commit-times");
    init_repo_empty(&dir);
    assert!(
        commit_times_since(&dir, 14).is_empty(),
        "no commits: no activity"
    );
    fs::write(dir.join("README.md"), "# seed\n").unwrap();
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "seed"]);
    git(&dir, &["checkout", "-q", "-b", "side"]);
    fs::write(dir.join("side.txt"), "side\n").unwrap();
    git(&dir, &["add", "side.txt"]);
    git(&dir, &["commit", "-q", "-m", "side"]);
    git(&dir, &["checkout", "-q", "main"]);

    let capture = |args: &[&str], env: &[(&str, &str)]| -> String {
        let mut cmd = Command::new(git_binary());
        cmd.args(args).current_dir(&dir);
        for (k, v) in crate::testutil::git_env() {
            cmd.env(k, v);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("git runs");
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // A commit 40 days old on a local branch is out of the window.
    let old_date = "@1000000000 +0000";
    let old = capture(
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "old"],
        &[
            ("GIT_AUTHOR_DATE", old_date),
            ("GIT_COMMITTER_DATE", old_date),
        ],
    );
    git(&dir, &["update-ref", "refs/heads/old", &old]);
    // A fresh commit only a remote-tracking ref holds is not local work.
    let fetched = capture(
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "fetched"],
        &[],
    );
    git(
        &dir,
        &["update-ref", "refs/remotes/origin/feature", &fetched],
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let times = commit_times_since(&dir, 14);
    assert_eq!(times.len(), 2, "seed on main and side only: {times:?}");
    assert!(
        times.iter().all(|t| (now - 600..=now + 600).contains(t)),
        "{times:?} vs {now}"
    );
    assert!(commit_times_since(&dir.join("missing"), 14).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

// ---- upstream rewrite (force-push) classification and reset ----

#[test]
fn classify_upstream_rewrite_r0_nothing_of_yours() {
    let fx = upstream_rewrite_fixture(RewriteKind::Dropped);
    let found = classify_upstream_rewrite(&fx.checkout, "main", "origin/main").unwrap();
    assert_eq!(found.upstream, "origin/main");
    assert_eq!(found.tip, fx.local_tip);
    assert_eq!(found.fork_point, fx.local_tip);
    assert_eq!(found.yours, 0);
    assert_eq!(found.dropped, 2);
}

#[test]
fn classify_upstream_rewrite_r1_counts_yours_and_dropped() {
    let fx = upstream_rewrite_fixture(RewriteKind::YourCommit);
    let found = classify_upstream_rewrite(&fx.checkout, "main", "origin/main").unwrap();
    assert_eq!(found.tip, fx.local_tip);
    assert_ne!(found.fork_point, fx.local_tip);
    assert_ne!(found.fork_point, fx.base);
    assert_eq!(found.yours, 1);
    assert_eq!(found.dropped, 2);
    let fork_parent = exec_git(
        &["rev-parse", &format!("{}^", found.fork_point)],
        &fx.checkout,
    );
    assert_eq!(fork_parent, fx.bot1);
}

#[test]
fn classify_upstream_rewrite_is_none_for_normal_divergence() {
    let fx = upstream_rewrite_fixture(RewriteKind::NormalDivergence);
    assert_eq!(
        ahead_behind("main", "origin/main", &fx.checkout),
        Some((1, 1))
    );
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "main", "origin/main"),
        None
    );
}

#[test]
fn classify_upstream_rewrite_is_none_unless_diverged() {
    let fx = upstream_rewrite_fixture(RewriteKind::Dropped);
    // behind only
    git(&fx.checkout, &["reset", "-q", "--hard", &fx.base]);
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "main", "origin/main"),
        None
    );
    // in sync
    git(&fx.checkout, &["reset", "-q", "--hard", "origin/main"]);
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "main", "origin/main"),
        None
    );
    // ahead only
    fs::write(fx.checkout.join("more.txt"), "more\n").unwrap();
    git(&fx.checkout, &["add", "-A"]);
    git(&fx.checkout, &["commit", "-q", "-m", "more"]);
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "main", "origin/main"),
        None
    );
}

#[test]
fn classify_upstream_rewrite_is_none_for_missing_refs() {
    let fx = upstream_rewrite_fixture(RewriteKind::Dropped);
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "main", "origin/missing"),
        None
    );
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "missing", "origin/main"),
        None
    );
    assert_eq!(
        classify_upstream_rewrite(&fx.root.join("nope"), "main", "origin/main"),
        None
    );
}

#[test]
fn classify_upstream_rewrite_is_none_without_remote_tracking_reflog() {
    let fx = upstream_rewrite_fixture(RewriteKind::Dropped);
    assert!(classify_upstream_rewrite(&fx.checkout, "main", "origin/main").is_some());
    git(&fx.checkout, &["reflog", "expire", "--expire=now", "--all"]);
    assert_eq!(
        classify_upstream_rewrite(&fx.checkout, "main", "origin/main"),
        None
    );
}

#[test]
fn classify_upstream_rewrite_works_in_a_linked_worktree() {
    let fx = upstream_rewrite_fixture(RewriteKind::Dropped);
    let linked = fx.root.join("linked");
    git(
        &fx.checkout,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    let found = classify_upstream_rewrite(&linked, "main", "origin/main").unwrap();
    assert_eq!((found.yours, found.dropped), (0, 2));
}

#[test]
fn classify_upstream_rewrite_is_none_for_a_local_branch_upstream() {
    // `feature` tracks the local `main` (`branch.feature.remote = .`). The
    // `main` reflog records your own local moves, so `--fork-point` would
    // call your unpushed commits "dropped, none yours".
    let dir = unique_dir("ws-rewrite-local-upstream");
    init_repo(&dir);
    git(
        &dir,
        &["checkout", "-q", "-b", "feature", "--track", "main"],
    );
    for name in ["f1.txt", "f2.txt"] {
        fs::write(dir.join(name), "work\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", name]);
    }
    let tip = exec_git(&["rev-parse", "feature"], &dir);
    git(&dir, &["checkout", "-q", "main"]);
    git(&dir, &["merge", "-q", "--ff-only", "feature"]);
    git(&dir, &["reset", "-q", "--hard", "ORIG_HEAD"]);
    fs::write(dir.join("m1.txt"), "main\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "m1"]);
    git(&dir, &["checkout", "-q", "feature"]);
    // the hazard is real: the local reflog puts the fork point at the tip
    assert_eq!(ahead_behind("feature", "main", &dir), Some((2, 1)));
    assert_eq!(
        exec_git(&["merge-base", "--fork-point", "main", "feature"], &dir),
        tip
    );

    assert_eq!(classify_upstream_rewrite(&dir, "feature", "main"), None);
    assert_eq!(
        reset_keep_to_upstream(&dir, "feature", "main", &tip),
        Err("main no longer looks rewritten".to_string())
    );
    assert_eq!(exec_git(&["rev-parse", "feature"], &dir), tip);
    let _ = fs::remove_dir_all(&dir);
}

fn rewrite_fixture_r0() -> (RewriteFixture, String) {
    let fx = upstream_rewrite_fixture(RewriteKind::Dropped);
    let tip = fx.local_tip.clone();
    (fx, tip)
}

#[test]
fn reset_keep_to_upstream_moves_branch_and_returns_old_tip() {
    let (fx, tip) = rewrite_fixture_r0();
    fs::write(fx.checkout.join("scratch.txt"), "untracked\n").unwrap();
    let old = reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &tip).unwrap();
    assert_eq!(old, tip);
    assert_eq!(
        exec_git(&["rev-parse", "main"], &fx.checkout),
        fx.upstream_tip
    );
    assert_eq!(
        ahead_behind("main", "origin/main", &fx.checkout),
        Some((0, 0))
    );
    assert_eq!(
        exec_git(&["branch", "--show-current"], &fx.checkout),
        "main"
    );
    assert_eq!(
        fs::read_to_string(fx.checkout.join("scratch.txt")).unwrap(),
        "untracked\n"
    );
    // the old tip stays reachable through the branch reflog (the undo)
    assert_eq!(exec_git(&["rev-parse", "main@{1}"], &fx.checkout), tip);
}

#[test]
fn reset_keep_to_upstream_refuses_tracked_changes() {
    let (fx, tip) = rewrite_fixture_r0();
    fs::write(fx.checkout.join("base.txt"), "edited\n").unwrap();
    assert_eq!(
        reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &tip),
        Err("working tree has changes".to_string())
    );
    assert_eq!(exec_git(&["rev-parse", "main"], &fx.checkout), tip);
}

#[test]
fn reset_keep_to_upstream_refuses_when_another_worktree_has_the_branch() {
    let (fx, tip) = rewrite_fixture_r0();
    let other = fx.root.join("other");
    git(
        &fx.checkout,
        &[
            "worktree",
            "add",
            "-q",
            "--force",
            other.to_str().unwrap(),
            "main",
        ],
    );
    let err = reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &tip).unwrap_err();
    assert!(
        err.starts_with("main is checked out in another worktree: ") && err.ends_with("other"),
        "{err}"
    );
    // the same guard fires from the linked side, naming the primary checkout
    let err = reset_keep_to_upstream(&other, "main", "origin/main", &tip).unwrap_err();
    assert!(
        err.starts_with("main is checked out in another worktree: ") && err.ends_with("checkout"),
        "{err}"
    );
    assert_eq!(exec_git(&["rev-parse", "main"], &fx.checkout), tip);
}

#[test]
fn reset_keep_to_upstream_ignores_a_prunable_worktree() {
    let (fx, tip) = rewrite_fixture_r0();
    let gone = fx.root.join("gone");
    git(
        &fx.checkout,
        &[
            "worktree",
            "add",
            "-q",
            "--force",
            gone.to_str().unwrap(),
            "main",
        ],
    );
    fs::remove_dir_all(&gone).unwrap();
    let old = reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &tip).unwrap();
    assert_eq!(old, tip);
}

#[test]
fn reset_keep_to_upstream_works_from_a_linked_worktree() {
    let (fx, tip) = rewrite_fixture_r0();
    // free `main` in the primary checkout, then move it to a linked worktree
    git(&fx.checkout, &["checkout", "-q", "--detach"]);
    let linked = fx.root.join("linked");
    git(
        &fx.checkout,
        &["worktree", "add", "-q", linked.to_str().unwrap(), "main"],
    );
    let old = reset_keep_to_upstream(&linked, "main", "origin/main", &tip).unwrap();
    assert_eq!(old, tip);
    assert_eq!(exec_git(&["rev-parse", "main"], &linked), fx.upstream_tip);
}

#[test]
fn reset_keep_to_upstream_refuses_a_stale_tip() {
    let (fx, tip) = rewrite_fixture_r0();
    assert_eq!(
        reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &fx.base),
        Err("main moved since the last refresh".to_string())
    );
    assert_eq!(exec_git(&["rev-parse", "main"], &fx.checkout), tip);
}

#[test]
fn reset_keep_to_upstream_refuses_when_head_is_not_on_the_branch() {
    let (fx, tip) = rewrite_fixture_r0();
    git(&fx.checkout, &["checkout", "-q", "-b", "other"]);
    assert_eq!(
        reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &tip),
        Err("HEAD is not on main".to_string())
    );
    git(&fx.checkout, &["checkout", "-q", "--detach"]);
    assert_eq!(
        reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &tip),
        Err("HEAD is not on main".to_string())
    );
}

#[test]
fn reset_keep_to_upstream_refuses_commits_of_yours() {
    let fx = upstream_rewrite_fixture(RewriteKind::YourCommit);
    assert_eq!(
        reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &fx.local_tip),
        Err("origin/main no longer looks rewritten".to_string())
    );
    assert_eq!(exec_git(&["rev-parse", "main"], &fx.checkout), fx.local_tip);
}

#[test]
fn reset_keep_to_upstream_refuses_normal_divergence() {
    let fx = upstream_rewrite_fixture(RewriteKind::NormalDivergence);
    assert_eq!(
        reset_keep_to_upstream(&fx.checkout, "main", "origin/main", &fx.local_tip),
        Err("origin/main no longer looks rewritten".to_string())
    );
}
