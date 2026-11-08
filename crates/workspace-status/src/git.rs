//! Git subprocess helpers. Prefer `/usr/bin/git` so WSL does not pick git.exe.
//!
//! Every git process `ws` starts must be built by [`git_process`] (the one
//! constructor) and run by [`git_output`] (the one runner). The runner puts
//! two bounds on each run, so a stalled network cannot hold a worker
//! forever:
//!
//! - **SSH deadline.** A remote command (`fetch`, `pull`, `push`,
//!   `ls-remote`, `clone`) gets a `GIT_SSH_COMMAND` with `ConnectTimeout`,
//!   `ServerAliveInterval` and `BatchMode=yes` (ssh fails at once instead
//!   of waiting on a prompt), unless the user set their own ssh command
//!   (see `default_ssh_command`).
//! - **Wall-clock deadline.** A run that takes longer than its time limit
//!   (120 s remote, 60 s local) is stopped and returns an `io::Error` of
//!   kind `TimedOut`. On Unix the stop reaches the whole process group, so
//!   `ssh` and other helpers that git started end with it: `SIGTERM` first
//!   (git removes its lockfiles), `SIGKILL` two seconds later.
//!
//! Git leads its own process group, so a terminal `SIGINT` / `SIGHUP` does
//! not reach it. `ws` ends running git itself on the way out:
//! [`stop_running_git`] on TUI quit, and the handler from
//! `stop_git_on_exit_signals` on `SIGINT` / `SIGTERM` / `SIGHUP`.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
#[cfg(unix)]
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use crate::file_index::paintable_line;

/// The git binary for this process. [`init_git_binary`] sets it once at
/// startup from the resolved settings. A process that never calls it (unit
/// tests, library use) resolves it on the first git call from
/// `WORKSPACE_STATUS_GIT` alone.
static GIT_BINARY: OnceLock<PathBuf> = OnceLock::new();

/// Seconds ssh waits for the TCP connection to the remote. ssh has no
/// connect timeout by default, so a host that drops SYN packets (a closed
/// VPN, a filtering firewall) holds `git fetch` until the kernel gives up,
/// many minutes later. Ten seconds is long enough for a slow working link.
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Seconds between ssh keepalive probes on a connected session. With
/// [`SSH_SERVER_ALIVE_COUNT_MAX`], a connection that goes silent mid-transfer
/// fails after about 30 s instead of waiting on TCP retransmits.
const SSH_SERVER_ALIVE_INTERVAL_SECS: u64 = 15;

/// Unanswered keepalive probes before ssh drops the connection. Two probes
/// let one lost packet pass without a false failure.
const SSH_SERVER_ALIVE_COUNT_MAX: u32 = 2;

/// Longest a remote git command (`fetch`, `pull`, `push`, `ls-remote`,
/// `clone`) may run before it is killed. The ssh options catch a dead
/// connection; this limit also catches a stalled https transfer or a
/// credential helper that waits for input. Two minutes leaves room for a
/// large fetch on a slow link.
const REMOTE_GIT_TIME_LIMIT: Duration = Duration::from_secs(120);

/// Longest a local git command may run before it is killed. Local commands
/// usually finish in milliseconds; a minute covers a large repository on a
/// slow disk and still frees a worker held by a hook or a stuck filesystem.
const LOCAL_GIT_TIME_LIMIT: Duration = Duration::from_secs(60);

/// Longest pause between two exit checks of a running git. The pause starts
/// at 1 ms and doubles, so a fast git call returns fast.
const MAX_EXIT_POLL_PAUSE: Duration = Duration::from_millis(25);

/// How long git's output pipes may stay open after git exits, before and
/// again after the kill of its process group.
const PIPE_CLOSE_GRACE: Duration = Duration::from_millis(200);

/// Global options before the subcommand that take their value as the next
/// argument (`git -C dir fetch`).
const GLOBAL_OPTIONS_WITH_VALUE: [&str; 5] =
    ["-c", "-C", "--git-dir", "--work-tree", "--namespace"];

/// Config that [`git_process`] sets on every git run as `-c <key>=<value>`
/// before the subcommand. `-c` beats every config file, so a user or
/// repository setting cannot change text that `ws` parses or a patch it
/// applies:
///
/// - `color.ui`, `color.diff`: `always` puts ANSI escapes into diff text.
///   `color.diff` beats `color.ui`, so both are pinned.
/// - `diff.noprefix`, `diff.mnemonicPrefix`, `diff.srcPrefix`,
///   `diff.dstPrefix`: keep the `a/` and `b/` path prefixes. A line stage
///   patch with other prefixes fails in `git apply -p1`. `diff.noprefix`
///   does not reset the two prefix keys (git 2.45 and later). An older git
///   ignores them.
/// - `log.showSignature`: `true` prints signature lines (`No signature`)
///   into `git log` output, ahead of the requested format.
///
/// Git passes `-c` values on to the git processes and hooks that it starts.
/// `diff.external` is not pinned here: an empty value makes git run an
/// empty command. [`DIFF_DRIVER_OFF`] turns it off instead.
const PINNED_CONFIG: [&str; 7] = [
    "color.ui=false",
    "color.diff=false",
    "diff.noprefix=false",
    "diff.mnemonicPrefix=false",
    "diff.srcPrefix=a/",
    "diff.dstPrefix=b/",
    "log.showSignature=false",
];

/// Options that [`with_diff_driver_off`] puts after the subcommand of a git
/// run that can print a diff. `--no-ext-diff` skips `diff.external`,
/// `GIT_EXTERNAL_DIFF` and `diff.<driver>.command`, which replace the
/// unified diff. `--no-textconv` skips `diff.<driver>.textconv` filters,
/// whose lines are not the blob's lines.
const DIFF_DRIVER_OFF: [&str; 2] = ["--no-ext-diff", "--no-textconv"];

/// The git binary: the value [`init_git_binary`] set, else
/// [`resolve_git_binary`] from `WORKSPACE_STATUS_GIT` with no config value.
pub fn git_binary() -> &'static Path {
    GIT_BINARY.get_or_init(|| {
        resolve_git_binary(std::env::var("WORKSPACE_STATUS_GIT").ok().as_deref(), None)
    })
}

/// Set the git binary for this process. Call it once at startup, before
/// the first git call. A later call (after the first git call, or a second
/// call) cannot change the binary: debug builds panic, so a git call that
/// moves ahead of startup init fails the CLI tests instead of silently
/// ignoring the config `git` key.
pub fn init_git_binary(bin: PathBuf) {
    let set = GIT_BINARY.set(bin).is_ok();
    debug_assert!(
        set,
        "init_git_binary called after the git binary was already resolved"
    );
}

/// Resolve the git binary: a non-empty `WORKSPACE_STATUS_GIT` (`env_value`)
/// wins, then the config `git` value, then `/usr/bin/git` if present, else
/// `git` on `PATH`.
pub fn resolve_git_binary(env_value: Option<&str>, config: Option<&Path>) -> PathBuf {
    if let Some(bin) = env_value.filter(|v| !v.is_empty()) {
        return PathBuf::from(bin);
    }
    if let Some(bin) = config {
        return bin.to_path_buf();
    }
    let usr = PathBuf::from("/usr/bin/git");
    if usr.is_file() {
        usr
    } else {
        PathBuf::from("git")
    }
}

/// Build a git subprocess for `bin`, with the env every `ws` git spawn needs.
///
/// This is the one constructor for git processes. Run the result with
/// [`git_output`] (never `output()` / `spawn()` directly), so the SSH and
/// wall-clock deadlines apply to it.
///
/// `GIT_OPTIONAL_LOCKS=0` stops git from taking `.git/index.lock` for
/// background refreshes (git-status(1), "BACKGROUND REFRESH"). `ws` polls
/// every repo every few seconds; without this, a poll can collide with a
/// concurrent `ws` instance or a manual `git add`/`commit`/`checkout` and
/// fail with `Unable to create '.git/index.lock': File exists`. The flag
/// only skips *optional* locks — write commands still take the locks they
/// need, so it is safe on every spawn, read or write.
///
/// The command starts with the [`PINNED_CONFIG`] `-c` options, so the
/// caller's arguments follow them.
pub(crate) fn git_process(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    for pin in PINNED_CONFIG {
        cmd.args(["-c", pin]);
    }
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Build a git subprocess that cannot steal the TUI's TTY.
///
/// Stdin is `/dev/null` so a credential prompt cannot deadlock against the
/// event loop. `GIT_TERMINAL_PROMPT=0` fails fast instead of waiting on a
/// hidden prompt. A run that can print a diff gets [`DIFF_DRIVER_OFF`]
/// ([`with_diff_driver_off`]). Run the result with [`git_output`] so both
/// deadlines apply.
fn git_command(bin: &Path, args: &[&str], cwd: &Path) -> Command {
    let mut cmd = git_process(bin);
    cmd.args(with_diff_driver_off(args))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// The index of the git subcommand in `args`: the first argument that is
/// not a global option (`-c k=v`, `-C dir`, `--no-pager`, …) or an
/// option's value.
fn git_subcommand_index(args: &[&str]) -> Option<usize> {
    let mut rest = args.iter().enumerate();
    while let Some((at, arg)) = rest.next() {
        if GLOBAL_OPTIONS_WITH_VALUE.contains(arg) {
            rest.next();
        } else if !arg.starts_with('-') {
            return Some(at);
        }
    }
    None
}

/// The git subcommand in `args` ([`git_subcommand_index`]).
fn git_subcommand<'a>(args: &[&'a str]) -> Option<&'a str> {
    git_subcommand_index(args).map(|at| args[at])
}

/// `args` with [`DIFF_DRIVER_OFF`] right after the subcommand when the run
/// can print a diff: `diff`, `show`, `log` and `stash show`. `blame` gets
/// `--no-textconv` only, so its line numbers are the blob's. Other runs
/// are unchanged.
fn with_diff_driver_off<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut out = args.to_vec();
    let Some(sub) = git_subcommand_index(args) else {
        return out;
    };
    let (at, options): (usize, &[&'static str]) = match (args[sub], args.get(sub + 1)) {
        ("diff" | "show" | "log", _) => (sub + 1, &DIFF_DRIVER_OFF),
        ("stash", Some(&"show")) => (sub + 2, &DIFF_DRIVER_OFF),
        ("blame", _) => (sub + 1, &DIFF_DRIVER_OFF[1..]),
        _ => return out,
    };
    out.splice(at..at, options.iter().copied());
    out
}

/// True when `args` run a git subcommand that talks to a remote.
fn is_remote_git(args: &[&str]) -> bool {
    matches!(
        git_subcommand(args),
        Some("fetch" | "pull" | "push" | "ls-remote" | "clone")
    )
}

/// The wall-clock limit for a git run with `args`.
fn git_time_limit(args: &[&str]) -> Duration {
    if is_remote_git(args) {
        REMOTE_GIT_TIME_LIMIT
    } else {
        LOCAL_GIT_TIME_LIMIT
    }
}

/// The `GIT_SSH_COMMAND` value for a remote git run, or `None` to leave
/// ssh as the user set it.
///
/// `None` when any of `GIT_SSH_COMMAND` / `GIT_SSH` (env, non-empty) or
/// `core.sshCommand` (git config) is set: a custom ssh command is never
/// replaced. Otherwise plain `ssh` with a connect timeout, keepalives and
/// `BatchMode=yes`. Git leads its own process group, so an ssh prompt on
/// `/dev/tty` (an unknown host key, a passphrase with no agent) would be
/// stopped by `SIGTTIN` and sit until the time limit; `BatchMode` makes ssh
/// fail at once with its own reason instead, the same intent as
/// `GIT_TERMINAL_PROMPT=0`. Plain `ssh -o …` still reads `~/.ssh/config`,
/// so `IdentityFile`, `ProxyJump` and other host settings stay in force.
fn default_ssh_command(
    env_ssh_command: Option<&OsStr>,
    env_ssh: Option<&OsStr>,
    config_ssh_command: Option<&str>,
) -> Option<String> {
    let env_set = |value: Option<&OsStr>| value.is_some_and(|v| !v.is_empty());
    if env_set(env_ssh_command)
        || env_set(env_ssh)
        || config_ssh_command.is_some_and(|v| !v.trim().is_empty())
    {
        return None;
    }
    Some(format!(
        "ssh -o ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS} \
         -o ServerAliveInterval={SSH_SERVER_ALIVE_INTERVAL_SECS} \
         -o ServerAliveCountMax={SSH_SERVER_ALIVE_COUNT_MAX} \
         -o BatchMode=yes"
    ))
}

/// The value `key` will have in `cmd`'s child: the command's own setting
/// (or removal) first, else this process's env.
fn child_env(cmd: &Command, key: &str) -> Option<OsString> {
    match cmd.get_envs().find(|(name, _)| *name == key) {
        Some((_, value)) => value.map(OsStr::to_os_string),
        None => std::env::var_os(key),
    }
}

/// `core.sshCommand` as `cmd`'s git binary sees it in `cmd`'s directory
/// (global and repository config), looked up within `limit`.
///
/// `Ok(None)` when it is unset (exit 1) or empty. `Err` when the lookup
/// did not answer cleanly (timeout, spawn error, any other exit): the
/// caller then cannot tell whether a custom ssh command is set.
fn configured_ssh_command(cmd: &Command, limit: Duration) -> io::Result<Option<String>> {
    let mut probe = git_process(Path::new(cmd.get_program()));
    probe
        .args(["config", "--get", "core.sshCommand"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = cmd.get_current_dir() {
        probe.current_dir(dir);
    }
    let out = git_output_within(probe, None, limit)?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.code() {
        Some(0) => Ok((!value.is_empty()).then_some(value)),
        Some(1) => Ok(None),
        _ => Err(io::Error::other(format!(
            "git config --get core.sshCommand exited with {}",
            out.status
        ))),
    }
}

/// Give a remote git run the ssh deadline, unless the user set their own
/// ssh command ([`default_ssh_command`]) or the `core.sshCommand` lookup
/// fails within `probe_limit` (then the env is left alone).
fn apply_ssh_deadline(cmd: &mut Command, probe_limit: Duration) {
    let env_ssh_command = child_env(cmd, "GIT_SSH_COMMAND");
    let env_ssh = child_env(cmd, "GIT_SSH");
    let env_ssh_command = env_ssh_command.as_deref();
    let env_ssh = env_ssh.as_deref();
    // Skip the config lookup when the env already decides.
    if default_ssh_command(env_ssh_command, env_ssh, None).is_none() {
        return;
    }
    let Ok(config) = configured_ssh_command(cmd, probe_limit) else {
        return;
    };
    if let Some(ssh) = default_ssh_command(env_ssh_command, env_ssh, config.as_deref()) {
        cmd.env("GIT_SSH_COMMAND", ssh);
    }
}

/// Run `job` on its own thread; the receiver gets its result.
fn on_own_thread<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx
}

/// Poll `child` until it exits (`Some`) or `deadline` passes (`None`).
fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        std::thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(MAX_EXIT_POLL_PAUSE);
    }
}

/// Slots in [`RUNNING_GIT_GROUPS`]. `ws` caps parallel git work well below
/// this; a run that finds every slot taken still runs, unregistered.
const MAX_TRACKED_GIT_GROUPS: usize = 256;

/// Process groups of the git runs in flight, one per slot, `0` for a free
/// slot. Plain atomics, so the exit-signal handler can read them safely.
/// A run registers its group right after spawn and clears it right after
/// reap. A run that finds no free slot is not registered: the stop paths
/// cannot reach it, and it still ends at its own deadline.
static RUNNING_GIT_GROUPS: [AtomicI32; MAX_TRACKED_GIT_GROUPS] =
    [const { AtomicI32::new(0) }; MAX_TRACKED_GIT_GROUPS];

/// Set once `ws` is exiting ([`stop_running_git`]). The runner then starts
/// no new git.
static GIT_SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// How long git gets to exit after `SIGTERM` (at its time limit, or when
/// `ws` exits) before its group gets `SIGKILL`. Git's `SIGTERM` handlers
/// remove the lockfiles they hold (`.git/index.lock`, ref locks); a
/// `SIGKILL` leaves them behind and blocks every later write in that repo.
/// Two seconds is plenty for that cleanup.
const GIT_TERM_GRACE: Duration = Duration::from_secs(2);

/// A slot in [`RUNNING_GIT_GROUPS`] that holds one run's process group. It
/// frees the slot on drop.
///
/// Git runs and the PR title command (`tui/pr_groups.rs`) register here,
/// so the exit paths ([`stop_running_git`], the exit-signal handler) end
/// them too.
pub(crate) struct TrackedGroup(Option<&'static AtomicI32>);

impl TrackedGroup {
    /// Register `pgid` in the first free slot (none when all are taken).
    pub(crate) fn register(pgid: u32) -> Self {
        let slot = i32::try_from(pgid)
            .ok()
            .filter(|pgid| *pgid > 0)
            .and_then(|pgid| {
                RUNNING_GIT_GROUPS.iter().find(|slot| {
                    slot.compare_exchange(0, pgid, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                })
            });
        Self(slot)
    }
}

impl Drop for TrackedGroup {
    fn drop(&mut self) {
        if let Some(slot) = self.0.take() {
            slot.store(0, Ordering::SeqCst);
        }
    }
}

/// True once `ws` is exiting ([`stop_running_git`]): a child process that
/// registers in [`RUNNING_GIT_GROUPS`] must not start.
pub(crate) fn exiting() -> bool {
    GIT_SHUTDOWN.load(Ordering::SeqCst)
}

/// True while `pgid` holds a slot in [`RUNNING_GIT_GROUPS`].
#[cfg(test)]
pub(crate) fn is_tracked(pgid: i32) -> bool {
    RUNNING_GIT_GROUPS
        .iter()
        .any(|slot| slot.load(Ordering::SeqCst) == pgid)
}

/// Send `signal` to process group `pgid`: a group `process_group(0)` made for
/// a git run, or for a PR title command (`tui/pr_groups.rs`).
#[cfg(unix)]
pub(crate) fn signal_group(pgid: i32, signal: libc::c_int) {
    if pgid > 0 {
        // SAFETY: a plain syscall with no pointers. `pgid` is a group
        // `process_group(0)` made for a child. While the child is not
        // reaped (timeout, stop) the id is still ours. Where git is already
        // reaped (post-exit pipe holder, a slot not yet cleared) the id is
        // not guaranteed ours, but Linux keeps a pid allocated while it is
        // in use as a pgid, and reuse in that short window would need a
        // full pid wrap plus the new process leading its own group.
        unsafe {
            libc::kill(-pgid, signal);
        }
    }
}

/// End a git that is still running and reap it. Unix: `SIGTERM` to its
/// group first, so git removes its lockfiles; `SIGKILL` to the group when
/// git has not exited after [`GIT_TERM_GRACE`]. Elsewhere: kill git.
fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let pgid = i32::try_from(child.id()).unwrap_or(0);
        signal_group(pgid, libc::SIGTERM);
        if let Ok(Some(_)) = wait_until(child, Instant::now() + GIT_TERM_GRACE) {
            return;
        }
        signal_group(pgid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// `SIGTERM` every group in `slots`, wait up to `grace` for those groups to
/// leave their slots (their runs reap git and free them), then `SIGKILL`
/// the ones still there. A group that registers after the `SIGTERM` pass
/// is left alone: it is a run allowed while `ws` exits (an auto-stash
/// restore), and a `SIGKILL` could cut it off halfway.
fn stop_groups(slots: &[AtomicI32], grace: Duration) {
    let stopped: Vec<i32> = slots
        .iter()
        .map(|slot| slot.load(Ordering::SeqCst))
        .filter(|pgid| *pgid > 0)
        .collect();
    #[cfg(unix)]
    for &pgid in &stopped {
        signal_group(pgid, libc::SIGTERM);
    }
    let still_running = || -> Vec<i32> {
        slots
            .iter()
            .map(|slot| slot.load(Ordering::SeqCst))
            .filter(|pgid| stopped.contains(pgid))
            .collect()
    };
    let deadline = Instant::now() + grace;
    while !still_running().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    #[cfg(unix)]
    for pgid in still_running() {
        signal_group(pgid, libc::SIGKILL);
    }
}

/// Stop every git run in flight and refuse new ones: `ws` is exiting.
///
/// Sets the shutdown flag (the runner then fails new runs with `git <sub>
/// not started: ws is exiting`), sends `SIGTERM` to every registered git
/// process group, waits up to two seconds for them to exit, then sends
/// `SIGKILL` to what is left. Git runs in its own process group, so a
/// terminal `SIGINT` / `SIGHUP` does not reach it; this is how `ws` ends
/// its git on the way out. It sleeps, so it is not for a signal handler.
/// On non-Unix targets it only sets the flag. The auto-stash pop after a
/// stopped pull still runs ([`run_restoring_user_work`]), so the user's
/// changes come back.
pub(crate) fn stop_running_git() {
    GIT_SHUTDOWN.store(true, Ordering::SeqCst);
    stop_groups(&RUNNING_GIT_GROUPS, GIT_TERM_GRACE);
}

/// Pulls in flight that stashed the user's changes and have not popped them
/// back yet ([`pull_quiet_detailed`], from the auto-stash push to the pop).
/// Quit waits for it to reach zero ([`wait_for_user_work_restore`]) before
/// it drops the worker threads.
static RESTORING_USER_WORK: AtomicUsize = AtomicUsize::new(0);

/// Longest quit waits for [`RESTORING_USER_WORK`]: the auto-stash pop's own
/// time limit plus the `SIGTERM` grace of a stopped git. A pop that was
/// already running has ended by then. A pull whose git started after the
/// stop pass may still be running when the wait gives up; if the process
/// ends before its pop, the changes stay in `git stash list`.
const USER_WORK_RESTORE_WAIT: Duration = LOCAL_GIT_TIME_LIMIT.saturating_add(GIT_TERM_GRACE);

/// Counts one run in [`RESTORING_USER_WORK`] while it lives.
struct RestoringUserWork;

impl RestoringUserWork {
    fn start() -> Self {
        RESTORING_USER_WORK.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for RestoringUserWork {
    fn drop(&mut self) {
        RESTORING_USER_WORK.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wait, at most a bounded time, while a pull still has to give the
/// user's stashed changes back. Quit calls it after [`stop_running_git`]
/// and before it stops waiting for worker threads, so the process does
/// not end between the auto-stash push and its pop. It sleeps, so it is
/// not for the event loop.
pub(crate) fn wait_for_user_work_restore() {
    let _ = wait_while_restoring_user_work(USER_WORK_RESTORE_WAIT);
}

/// [`wait_for_user_work_restore`] with the bound `limit`. `false` when the
/// bound ran out first.
fn wait_while_restoring_user_work(limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while RESTORING_USER_WORK.load(Ordering::SeqCst) > 0 {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

/// `Err` when `exiting` (the shutdown flag) is set: no new git may start.
/// A run that `restores_user_work` (an auto-stash pop) may still start:
/// skipping it would leave the user's changes in a stash entry.
fn check_may_start(exiting: bool, restores_user_work: bool, sub: Option<&str>) -> io::Result<()> {
    if exiting && !restores_user_work {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("{} not started: ws is exiting", git_label(sub)),
        ));
    }
    Ok(())
}

/// Make `SIGINT`, `SIGTERM` and `SIGHUP` stop running git before `ws`
/// dies. Call it once at CLI startup, never from tests.
///
/// The handler sends `SIGTERM` to every registered git process group,
/// calls the hook from [`set_exit_signal_hook`] (the TUI terminal restore),
/// then restores the default action and raises the signal again, so `ws` exits
/// with the usual status. A signal whose handler is not the default (an
/// ignored `SIGHUP` under `nohup`, say) is left alone.
#[cfg(unix)]
pub(crate) fn stop_git_on_exit_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: `sigaction` with zeroed, then filled, structs; the
        // handler only calls async-signal-safe functions.
        unsafe {
            let mut current: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, std::ptr::null(), &mut current) != 0
                || current.sa_sigaction != libc::SIG_DFL
            {
                continue;
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction =
                on_exit_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            action.sa_flags = libc::SA_RESETHAND;
            // Block the other exit signals while the handler runs, so a
            // second one cannot end `ws` part way through the terminal
            // restore. They stay pending and arrive once it returns.
            libc::sigemptyset(&mut action.sa_mask);
            for blocked in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                libc::sigaddset(&mut action.sa_mask, blocked);
            }
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// The `fn()` the exit-signal handler calls after it stops git, stored as
/// a pointer; null for none. Set by [`set_exit_signal_hook`].
#[cfg(unix)]
static EXIT_SIGNAL_HOOK: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

/// Make the exit-signal handler call `hook` after it stops running git and
/// before the signal ends `ws`. The TUI registers its terminal restore
/// here, so this module does not depend on the TUI. `hook` runs inside a
/// signal handler: it must call only async-signal-safe functions (no
/// allocation, no lock).
#[cfg(unix)]
pub(crate) fn set_exit_signal_hook(hook: fn()) {
    EXIT_SIGNAL_HOOK.store(hook as *mut (), Ordering::SeqCst);
}

/// The exit-signal handler: async-signal-safe calls only (atomic loads,
/// `kill`, the registered hook, `signal`, `raise`).
#[cfg(unix)]
extern "C" fn on_exit_signal(signal: libc::c_int) {
    for slot in &RUNNING_GIT_GROUPS {
        let pgid = slot.load(Ordering::SeqCst);
        if pgid > 0 {
            // SAFETY: a plain syscall with no pointers (see `signal_group`).
            unsafe {
                libc::kill(-pgid, libc::SIGTERM);
            }
        }
    }
    let hook = EXIT_SIGNAL_HOOK.load(Ordering::SeqCst);
    if !hook.is_null() {
        // SAFETY: a non-null value is only ever a `fn()` that
        // `set_exit_signal_hook` cast to `*mut ()`; casting it back gives
        // that same function pointer.
        let hook = unsafe { std::mem::transmute::<*mut (), fn()>(hook) };
        hook();
    }
    // SAFETY: both are async-signal-safe; the signal is blocked while this
    // handler runs, so it is delivered with the default action on return.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

/// `git <sub>`, or `git` when the argv has no subcommand.
fn git_label(sub: Option<&str>) -> String {
    sub.map_or_else(|| "git".to_string(), |sub| format!("git {sub}"))
}

/// The error for a git run killed at its time limit.
fn timed_out(sub: Option<&str>, limit: Duration, remote: bool) -> io::Error {
    let label = git_label(sub);
    let secs = limit.as_secs();
    let message = if remote {
        format!("{label} timed out after {secs}s: the remote did not answer")
    } else {
        format!("{label} timed out after {secs}s")
    };
    io::Error::new(io::ErrorKind::TimedOut, message)
}

/// The error for a git that exited while a process outside its group kept
/// its stdout or stderr open.
fn output_held_open(sub: Option<&str>) -> io::Error {
    io::Error::other(format!(
        "{} exited but a process it started kept its output open",
        git_label(sub)
    ))
}

/// Take the result of an output reader into `slot` if it arrives by
/// `until`. A missing pipe (not piped) is an empty output.
fn take_output(
    slot: &mut Option<io::Result<Vec<u8>>>,
    reader: Option<&mpsc::Receiver<io::Result<Vec<u8>>>>,
    until: Instant,
) {
    if slot.is_some() {
        return;
    }
    let Some(reader) = reader else {
        *slot = Some(Ok(Vec::new()));
        return;
    };
    match reader.recv_timeout(until.saturating_duration_since(Instant::now())) {
        Ok(read) => *slot = Some(read),
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            *slot = Some(Err(io::Error::other("git output reader stopped")));
        }
    }
}

/// Run a git `cmd` from [`git_process`] / [`git_command`] under both
/// deadlines, with the time limit picked from its argv (remote or local).
///
/// See [`git_output_within`] for the run itself.
pub(crate) fn git_output(cmd: Command, stdin: Option<&[u8]>) -> io::Result<Output> {
    run_bounded(cmd, stdin, None, false)
}

/// Run a git `cmd` from [`git_process`] / [`git_command`], killed after
/// `limit`.
///
/// Stdin gets `stdin` through a pipe, or `/dev/null` when it is `None`;
/// stdout and stderr keep the caller's setting. A broken pipe while writing
/// `stdin` means git exited before it read all of it; the exit status then
/// says why, so that is not an `Err`. A remote run also gets the ssh
/// deadline ([`default_ssh_command`]); its `core.sshCommand` lookup counts
/// against `limit`. On Unix git leads its own process group, so the stop at
/// `limit` (`SIGTERM`, then `SIGKILL` after [`GIT_TERM_GRACE`]) reaches
/// `ssh` too. The group is registered for [`stop_running_git`] while git
/// runs.
///
/// `Err` of kind `TimedOut` at `limit`, with the message `git <sub> timed
/// out after <N>s`, plus `: the remote did not answer` for a remote run.
/// `Err` of kind `Interrupted` (`git <sub> not started: ws is exiting`)
/// once [`stop_running_git`] ran. When git exits but a process it started
/// keeps stdout or stderr open, that process group is killed and the run
/// returns git's own status.
pub(crate) fn git_output_within(
    cmd: Command,
    stdin: Option<&[u8]>,
    limit: Duration,
) -> io::Result<Output> {
    run_bounded(cmd, stdin, Some(limit), false)
}

/// The run behind [`git_output`] and [`git_output_within`]. `limit` `None`
/// picks the limit from the argv. `restores_user_work` lets the run start
/// while `ws` exits ([`check_may_start`]); the deadlines and the group
/// registration apply all the same.
fn run_bounded(
    mut cmd: Command,
    stdin: Option<&[u8]>,
    limit: Option<Duration>,
    restores_user_work: bool,
) -> io::Result<Output> {
    let args: Vec<String> = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let limit = limit.unwrap_or_else(|| git_time_limit(&argv));
    let remote = is_remote_git(&argv);
    let sub = git_subcommand(&argv);
    check_may_start(GIT_SHUTDOWN.load(Ordering::SeqCst), restores_user_work, sub)?;
    // The deadline starts before the ssh config lookup, so the lookup
    // cannot stretch a remote run past `limit`.
    let deadline = Instant::now() + limit;
    if remote {
        apply_ssh_deadline(&mut cmd, deadline.saturating_duration_since(Instant::now()));
    }
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    let pgid = child.id();
    let tracked = TrackedGroup::register(pgid);
    // A stop that ran between the check above and the registration did not
    // see this group: end it here.
    if let Err(err) = check_may_start(GIT_SHUTDOWN.load(Ordering::SeqCst), restores_user_work, sub)
    {
        stop_child(&mut child);
        return Err(err);
    }
    // Write and read on their own threads: git answers as it reads, so a
    // full stdout pipe must not stall the stdin write, and a stalled git
    // must not stall this thread past `deadline`.
    let writer = match (stdin, child.stdin.take()) {
        (Some(bytes), Some(mut pipe)) => {
            let bytes = bytes.to_vec();
            Some(on_own_thread(move || pipe.write_all(&bytes)))
        }
        _ => None,
    };
    let stdout = child
        .stdout
        .take()
        .map(|mut pipe| on_own_thread(move || read_all(&mut pipe)));
    let stderr = child
        .stderr
        .take()
        .map(|mut pipe| on_own_thread(move || read_all(&mut pipe)));
    let status = match wait_until(&mut child, deadline) {
        Ok(Some(status)) => status,
        Ok(None) => {
            stop_child(&mut child);
            return Err(timed_out(sub, limit, remote));
        }
        Err(err) => {
            stop_child(&mut child);
            return Err(err);
        }
    };
    // Git is reaped: its group id may be reused from here on.
    drop(tracked);
    // Git exited. A process it left behind may still hold a pipe open:
    // give the pipes a short grace, then kill git's group (where such a
    // process lives) and give them one more.
    let (mut out, mut err) = (None, None);
    for round in 0..2 {
        let until = Instant::now() + PIPE_CLOSE_GRACE;
        take_output(&mut out, stdout.as_ref(), until);
        take_output(&mut err, stderr.as_ref(), until);
        if out.is_some() && err.is_some() {
            break;
        }
        #[cfg(unix)]
        if round == 0 {
            signal_group(i32::try_from(pgid).unwrap_or(0), libc::SIGKILL);
        }
    }
    let (Some(out), Some(err)) = (out, err) else {
        return Err(output_held_open(sub));
    };
    let (stdout, stderr) = (out?, err?);
    let write_err = writer
        .and_then(|rx| rx.recv_timeout(PIPE_CLOSE_GRACE).ok())
        .and_then(Result::err)
        .filter(|err| err.kind() != io::ErrorKind::BrokenPipe);
    if let Some(err) = write_err {
        return Err(err);
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Run `bin --version` through the runner (null stdin, captured output,
/// the local time limit), in this process's directory. The doctor report
/// uses it to probe the configured git binary.
pub(crate) fn git_version_output(bin: &Path) -> io::Result<Output> {
    git_output(git_command(bin, &["--version"], Path::new(".")), None)
}

/// Read `pipe` to its end.
fn read_all(pipe: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn run(args: &[&str], cwd: &Path) -> io::Result<Output> {
    git_output(git_command(git_binary(), args, cwd), None)
}

/// Like [`run`], but the run starts even while `ws` exits. Only for git
/// that puts the user's work back (the auto-stash pop after a pull, and the
/// `refs/stash` reads that decide it): a quit during the pull must not
/// leave their changes in a stash entry. The time
/// limit, the `SIGTERM` / `SIGKILL` stop and the group registration still
/// apply.
fn run_restoring_user_work(args: &[&str], cwd: &Path) -> io::Result<Output> {
    run_bounded(git_command(git_binary(), args, cwd), None, None, true)
}

/// Run git with `stdin` piped: the `git apply` wrappers and index blame.
///
/// A broken pipe while writing means git exited before it read all of
/// `stdin`; the exit status then says why, so that is not an `Err`.
fn run_with_stdin(args: &[&str], cwd: &Path, stdin: &[u8]) -> io::Result<Output> {
    git_output(git_command(git_binary(), args, cwd), Some(stdin))
}

/// Run git and return trimmed stdout. Empty string on failure.
pub fn exec_git(args: &[&str], cwd: &Path) -> String {
    match run(args, cwd) {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Run git and return the exit code (`-1` when the process did not start,
/// timed out, or was killed by a signal).
pub fn exec_git_status(args: &[&str], cwd: &Path) -> i32 {
    match run(args, cwd) {
        Ok(out) => out.status.code().unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Run git. `Err` when the process exits non-zero or fails to start.
///
/// The error is git's own reason line ([`git_reason_line`]), or
/// `git <sub> exited with code N` when git printed none.
pub fn exec_git_checked(args: &[&str], cwd: &Path) -> Result<(), String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

fn exit_code_message(args: &[&str], out: &std::process::Output) -> String {
    format!(
        "git {} exited with code {}",
        args.first().copied().unwrap_or("git"),
        out.status.code().unwrap_or(-1)
    )
}

/// One line that says why a git run failed, without the `fatal:` / `error:` tag.
///
/// Order: a push `! [rejected]` line, then the first `fatal:` / `error:`
/// line of stderr, then the first other stderr line that is not a `hint:`,
/// then a stdout `CONFLICT` line (`stash apply` prints its conflict there).
/// `None` when git printed nothing useful.
pub(crate) fn git_reason_line(out: &std::process::Output) -> Option<String> {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines = || stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    let tagged = |line: &str| {
        ["fatal:", "error:"]
            .iter()
            .find_map(|tag| line.strip_prefix(tag))
            .map(|rest| rest.trim().to_string())
    };
    lines()
        .find_map(|line| line.strip_prefix("! ").filter(|rest| rest.starts_with('[')))
        .map(|rest| rest.split_whitespace().collect::<Vec<_>>().join(" "))
        .or_else(|| lines().find_map(tagged))
        .or_else(|| {
            lines()
                .find(|line| !line.starts_with("hint:"))
                .map(str::to_string)
        })
        .or_else(|| {
            stdout
                .lines()
                .map(str::trim)
                .find(|line| line.starts_with("CONFLICT"))
                .map(str::to_string)
        })
}

/// Binary-safe blob bytes for `<rev>:<path>` (`git cat-file blob`). Missing path → `None`.
///
/// Unlike [`exec_git`], this does not UTF-8-decode or trim stdout.
/// TTY callers must run this on `spawn_blocking`, not the event loop.
pub fn blob_bytes(cwd: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let spec = format!("{rev}:{path}");
    match run(&["cat-file", "blob", &spec], cwd) {
        Ok(out) if out.status.success() => Some(out.stdout),
        _ => None,
    }
}

/// True when the worktree or index has tracked changes.
///
/// A check that git could not answer (timeout, error exit) counts as
/// changes, so the guards that call this refuse to write. Use
/// [`local_changes`] to tell the two apart.
pub fn repo_has_local_changes(cwd: &Path) -> bool {
    local_changes(cwd).unwrap_or(true)
}

/// Whether the worktree or index has tracked changes
/// (`diff --quiet`, then `diff --cached --quiet`).
///
/// `Err` holds git's reason when a check did not exit 0 (clean) or 1
/// (changes): a timeout, a start failure, or a git error.
pub fn local_changes(cwd: &Path) -> Result<bool, String> {
    let differs = |args: &[&str]| match run(args, cwd) {
        Ok(out) => match out.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(git_failure_message(args, &out)),
        },
        Err(err) => Err(err.to_string()),
    };
    Ok(differs(&["diff", "--quiet"])? || differs(&["diff", "--cached", "--quiet"])?)
}

/// Resolve `ref` to a commit SHA. Missing refs return `None`.
pub fn rev_parse_quiet(git_ref: &str, cwd: &Path) -> Option<String> {
    let sha = exec_git(&["rev-parse", "--verify", "--quiet", git_ref], cwd);
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Checkout an existing branch, or create it tracking `origin/<branch>`.
pub fn checkout_branch(branch: &str, cwd: &Path) -> bool {
    if exec_git_status(&["checkout", branch, "--quiet"], cwd) == 0 {
        return true;
    }
    let origin = format!("origin/{branch}");
    exec_git_status(&["checkout", "-b", branch, &origin, "--quiet"], cwd) == 0
}

/// Fast-forward HEAD to an already-fetched remote-tracking ref (no fetch, no reset).
///
/// Accepts `origin/foo` or `refs/remotes/origin/foo`. Uses `git merge --ff-only`
/// so an ahead or diverged local tip is left unchanged (no merge commit).
///
/// Returns true when HEAD now matches the remote-tracking tip.
pub fn fast_forward_to_remote_ref(remote_ref: &str, cwd: &Path) -> bool {
    let git_ref = if remote_ref.starts_with("refs/") {
        remote_ref.to_string()
    } else {
        format!("refs/remotes/{remote_ref}")
    };
    let Some(target_sha) = rev_parse_quiet(&git_ref, cwd) else {
        return false;
    };
    if exec_git_status(&["merge", "--ff-only", "--quiet", &git_ref], cwd) != 0 {
        return false;
    }
    rev_parse_quiet("HEAD", cwd).as_deref() == Some(target_sha.as_str())
}

/// Commits only on `left` and only on `right` (`rev-list --left-right --count left...right`).
///
/// `None` when either rev does not resolve or git fails.
pub fn ahead_behind(left: &str, right: &str, cwd: &Path) -> Option<(usize, usize)> {
    let range = format!("{left}...{right}");
    let out = exec_git(&["rev-list", "--left-right", "--count", &range, "--"], cwd);
    let mut counts = out.split_whitespace().map(str::parse::<usize>);
    match (counts.next(), counts.next(), counts.next()) {
        (Some(Ok(ahead)), Some(Ok(behind)), None) => Some((ahead, behind)),
        _ => None,
    }
}

/// A local branch whose upstream was rewritten (force-pushed) under it.
///
/// Only a diverged branch (ahead and behind) can be one. The upstream
/// reflog tells which local commits the old upstream history held: those
/// are the commits the rewrite dropped. See [`classify_upstream_rewrite`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamRewrite {
    /// Upstream ref as given, for example `origin/main`.
    pub upstream: String,
    /// Full sha of `git merge-base --fork-point <upstream> <branch>`: the
    /// newest local commit that was once on the upstream.
    pub fork_point: String,
    /// Full sha of the local branch tip used for the classification.
    /// Omitted from `--json`; the `p` confirm checks it against the branch.
    #[serde(skip)]
    pub tip: String,
    /// Local commits above the fork point: work that is not on the old upstream.
    pub yours: usize,
    /// Local commits between the merge base and the fork point: commits the
    /// upstream held and then dropped.
    pub dropped: usize,
}

/// `rev-list --count <range>`. `None` when git fails.
fn rev_list_count(cwd: &Path, range: &str) -> Option<usize> {
    exec_git(&["rev-list", "--count", range, "--"], cwd)
        .parse()
        .ok()
}

/// Tell a force-pushed upstream apart from a normal divergence.
///
/// Returns `None` unless `upstream` resolves to a remote-tracking ref
/// (`refs/remotes/...`) and `branch` is ahead of and behind it. Only a
/// remote-tracking ref moves on fetch or push alone, so only its reflog
/// tells "was on the upstream" apart from "is yours". The reflog of a local
/// upstream branch (`branch.<b>.remote = .`) records your own local moves.
/// With `F` = `merge-base --fork-point`, `MB` = `merge-base` and `tip` = the
/// branch tip:
///
/// - `F` == `tip`: the upstream dropped commits and none are yours
///   (`yours: 0`, `dropped: count(MB..tip)`).
/// - `F` found, not `tip`, not `MB`: the upstream dropped commits and you
///   have commits on top (`yours: count(F..tip)`, `dropped: count(MB..F)`).
/// - No `F` (reflog expired or missing), or `F` == `MB`: a normal
///   divergence, `None`. Never guessed.
///
/// Any git failure or missing ref is `None`. Runs a few cheap git commands;
/// call it once per diverged repo, off the event loop.
pub fn classify_upstream_rewrite(
    cwd: &Path,
    branch: &str,
    upstream: &str,
) -> Option<UpstreamRewrite> {
    let upstream_ref = exec_git(&["rev-parse", "--symbolic-full-name", upstream], cwd);
    if !upstream_ref.starts_with("refs/remotes/") {
        return None;
    }
    let tip = rev_parse_quiet(&format!("refs/heads/{branch}"), cwd)?;
    let upstream_tip = rev_parse_quiet(&upstream_ref, cwd)?;
    let (ahead, behind) = ahead_behind(&tip, &upstream_tip, cwd)?;
    if ahead == 0 || behind == 0 {
        return None;
    }
    let merge_base = merge_base(cwd, &tip, &upstream_tip).ok().flatten()?;
    let fork_point = exec_git(&["merge-base", "--fork-point", &upstream_ref, &tip], cwd);
    if fork_point.is_empty() || fork_point == merge_base {
        return None;
    }
    let (yours, dropped) = if fork_point == tip {
        (0, rev_list_count(cwd, &format!("{merge_base}..{tip}"))?)
    } else {
        (
            rev_list_count(cwd, &format!("{fork_point}..{tip}"))?,
            rev_list_count(cwd, &format!("{merge_base}..{fork_point}"))?,
        )
    };
    Some(UpstreamRewrite {
        upstream: upstream.to_string(),
        fork_point,
        tip,
        yours,
        dropped,
    })
}

/// Path of another worktree that has `branch` checked out, when there is one.
///
/// Reads `git worktree list --porcelain` and skips the worktree at `cwd`
/// (compared by canonical path).
fn other_worktree_on_branch(cwd: &Path, branch: &str) -> Option<String> {
    let own = PathBuf::from(exec_git(&["rev-parse", "--show-toplevel"], cwd));
    let own = std::fs::canonicalize(&own).unwrap_or(own);
    let wanted = format!("refs/heads/{branch}");
    list_worktrees_porcelain(cwd)
        .split("\n\n")
        .filter(|block| !block.lines().any(|l| l.starts_with("prunable")))
        .filter_map(|block| {
            let path = block.lines().find_map(|l| l.strip_prefix("worktree "))?;
            let on = block.lines().find_map(|l| l.strip_prefix("branch "))?;
            (on == wanted).then_some(path)
        })
        .find(|path| {
            let path = Path::new(path);
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()) != own
        })
        .map(str::to_string)
}

/// Move `branch` to `upstream` with `git reset --keep`, for a rewritten upstream.
///
/// Guards, re-checked right before the write and in this order: HEAD is on
/// `branch`; the branch tip equals `expected_tip` (the tip the confirm
/// showed); no other worktree has `branch` checked out; no tracked changes;
/// [`classify_upstream_rewrite`] still finds a rewrite with nothing of yours
/// (so `upstream` must be a remote-tracking ref).
/// The reset runs on the upstream sha resolved once before the checks, so a
/// fetch in between cannot move the target. `--keep` never discards local
/// changes. The old tip stays reachable through
/// the branch reflog, which is the undo.
///
/// Returns the old tip (full sha). `Err` carries a short human reason, or
/// git's own reason line when the reset fails. Blocking: call it off the
/// event loop.
pub fn reset_keep_to_upstream(
    cwd: &Path,
    branch: &str,
    upstream: &str,
    expected_tip: &str,
) -> Result<String, String> {
    let head = exec_git(&["symbolic-ref", "-q", "HEAD"], cwd);
    if head != format!("refs/heads/{branch}") {
        return Err(format!("HEAD is not on {branch}"));
    }
    let tip = rev_parse_quiet(&format!("refs/heads/{branch}"), cwd)
        .filter(|tip| tip == expected_tip)
        .ok_or_else(|| format!("{branch} moved since the last refresh"))?;
    if let Some(path) = other_worktree_on_branch(cwd, branch) {
        return Err(format!(
            "{branch} is checked out in another worktree: {path}"
        ));
    }
    if repo_has_local_changes(cwd) {
        return Err("working tree has changes".to_string());
    }
    // Pin the target sha once: a fetch between the check and the write must
    // not move it, so the reset runs on the sha the check saw.
    let rewritten = || format!("{upstream} no longer looks rewritten");
    let target = rev_parse_quiet(upstream, cwd).ok_or_else(rewritten)?;
    match classify_upstream_rewrite(cwd, branch, upstream) {
        Some(found) if found.yours == 0 && found.tip == tip => {}
        _ => return Err(rewritten()),
    }
    if rev_parse_quiet(upstream, cwd).as_deref() != Some(target.as_str()) {
        return Err(rewritten());
    }
    exec_git_checked(&["reset", "--keep", &target, "--"], cwd)?;
    Ok(tip)
}

/// Outcome of merging a rev into the current HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeIntoHeadResult {
    /// HEAD already contained `rev`.
    AlreadyUpToDate,
    /// HEAD fast-forwarded to `rev`.
    FastForward,
    /// Created a merge commit (`--no-ff` after a failed fast-forward).
    MergeCommit,
    /// Conflicts left in the worktree. The merge is not aborted or continued.
    Conflict,
    /// Merge did not start or failed without leaving a merge in progress.
    Failed(String),
}

fn run_merge(args: &[&str], cwd: &Path) -> io::Result<Output> {
    let mut cmd = git_command(git_binary(), args, cwd);
    cmd.env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no");
    git_output(cmd, None)
}

fn merge_failed_message(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.is_empty() {
        format!("git merge exited {}", out.status.code().unwrap_or(-1))
    } else {
        err
    }
}

/// Merge `rev` into HEAD. Fast-forward when that is possible, otherwise a
/// merge commit. Does not rebase, does not abort on conflict, and does not
/// open an editor (`--no-edit`).
///
/// Tries `git merge --ff-only`, then `git merge --no-ff --no-edit`. Conflicts
/// stay uncommitted (`MERGE_HEAD` remains). Callers refuse a dirty worktree
/// before invoking this.
pub fn merge_into_head(rev: &str, cwd: &Path) -> MergeIntoHeadResult {
    let before = rev_parse_quiet("HEAD", cwd);
    match run_merge(&["merge", "--ff-only", "--quiet", "--", rev], cwd) {
        Ok(out) if out.status.success() => {
            let after = rev_parse_quiet("HEAD", cwd);
            if before == after {
                return MergeIntoHeadResult::AlreadyUpToDate;
            }
            return MergeIntoHeadResult::FastForward;
        }
        Ok(_) => {}
        Err(err) => return MergeIntoHeadResult::Failed(err.to_string()),
    }
    if rev_parse_quiet("MERGE_HEAD", cwd).is_some() {
        return MergeIntoHeadResult::Conflict;
    }
    match run_merge(
        &["merge", "--no-ff", "--no-edit", "--quiet", "--", rev],
        cwd,
    ) {
        Ok(out) if out.status.success() => MergeIntoHeadResult::MergeCommit,
        Ok(out) => {
            if rev_parse_quiet("MERGE_HEAD", cwd).is_some() {
                MergeIntoHeadResult::Conflict
            } else {
                MergeIntoHeadResult::Failed(merge_failed_message(&out))
            }
        }
        Err(err) => MergeIntoHeadResult::Failed(err.to_string()),
    }
}

const AUTO_STASH_MESSAGE: &str = "ws-status: auto-stash before pull";

/// What [`pull_quiet_detailed`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullQuietResult {
    /// The pull ran and any auto-stash came back cleanly.
    pub ok: bool,
    /// The auto-stash push made a stash entry (it holds the tracked local
    /// changes until the pop).
    pub stashed: bool,
    /// The auto-stash entry was not given back. The entry stays in
    /// `git stash list` unless the note says it is gone. With no
    /// [`stash_note`](Self::stash_note) the pop conflicted and the tree has
    /// conflict markers.
    pub stash_pop_failed: bool,
    /// Why the auto-stash entry was kept when the reason is not a pop
    /// conflict (a newer stash entry on top of it, a pop that could not
    /// run), and which `git stash list` entry holds the changes.
    pub stash_note: Option<String>,
    /// Why the pull did not finish: `local changes check failed: …`,
    /// `auto-stash failed: …`, `auto-stash check failed: …`, or git's
    /// reason line when the pull failed.
    pub error: Option<String>,
}

/// `git pull --quiet`, stashing tracked local changes first when needed.
///
/// The pop after the pull runs only when the auto-stash push made a new
/// stash entry (`stash_push_entry`). A dirty submodule makes
/// `diff --quiet` report changes while `stash push` saves nothing; a pop
/// then would apply and drop the user's own `stash@{0}`. A dirty check
/// that git could not answer stops before the pull and touches no stash.
/// A push that failed or was stopped after it saved an entry skips the
/// pull and gives that entry back at once (`restore_auto_stash`).
pub fn pull_quiet_detailed(cwd: &Path) -> PullQuietResult {
    let failed = |error: String| PullQuietResult {
        ok: false,
        stashed: false,
        stash_pop_failed: false,
        stash_note: None,
        error: Some(error),
    };
    let dirty = match local_changes(cwd) {
        Ok(dirty) => dirty,
        Err(err) => return failed(format!("local changes check failed: {err}")),
    };
    let mut entry = None;
    // Held from before the stash push to after the pop, so a quit waits
    // for the user's changes to come back.
    let _restoring = dirty.then(RestoringUserWork::start);
    if dirty {
        let args = ["stash", "push", "-m", AUTO_STASH_MESSAGE, "--quiet"];
        let pushed = match stash_push_entry(&args, cwd) {
            Ok(pushed) => pushed,
            Err(err) => return failed(format!("auto-stash failed: {err}")),
        };
        entry = match pushed.entry {
            Ok(entry) => entry,
            Err(err) => {
                return failed(format!(
                    "auto-stash check failed: {err} — your changes may be in `git stash list`"
                ))
            }
        };
        if let Err(err) = pushed.pushed {
            // The push failed or a quit stopped it, maybe after it saved an
            // entry. The worktree state is unknown: do not pull over it.
            let mut result = failed(format!("auto-stash failed: {err}"));
            if let Some(entry) = entry {
                result.stashed = true;
                (result.stash_pop_failed, result.stash_note) = restore_auto_stash(cwd, &entry);
            }
            return result;
        }
    }

    let pulled = exec_git_checked(&["pull", "--quiet"], cwd);
    // The pop runs even when a quit stopped the pull: it gives the user's
    // changes back.
    let (stash_pop_failed, stash_note) = match &entry {
        Some(entry) => restore_auto_stash(cwd, entry),
        None => (false, None),
    };
    PullQuietResult {
        ok: pulled.is_ok() && !stash_pop_failed,
        stashed: entry.is_some(),
        stash_pop_failed,
        stash_note,
        error: pulled.err(),
    }
}

/// Pop the auto-stash `entry` (its commit id) when it is still `stash@{0}`.
///
/// Returns `(stash_pop_failed, stash_note)` for [`PullQuietResult`]. The
/// user may have stashed in another terminal during the pull: a plain
/// `stash pop` would then apply their entry, so a newer entry, a missing
/// one, or a failed read keeps the auto-stash and says where it is. A pop
/// that conflicts gives no note (the tree has conflict markers); a pop that
/// fails for another reason, or cannot run, gives git's reason. Every git
/// run here still starts while `ws` exits.
fn restore_auto_stash(cwd: &Path, entry: &str) -> (bool, Option<String>) {
    let short = entry.get(..12).unwrap_or(entry);
    let kept = |reason: String| {
        let note = format!(
            "{reason}; your changes are in `git stash list` as {short} \"{AUTO_STASH_MESSAGE}\""
        );
        (true, Some(note))
    };
    match stash_tip(cwd, run_restoring_user_work) {
        Ok(Some(tip)) if tip == entry => {}
        Ok(Some(_)) => {
            return kept("auto-stash not restored: a newer stash entry is on top of it".into())
        }
        Ok(None) => {
            let note =
                format!("auto-stash not restored: entry {short} is no longer in `git stash list`");
            return (true, Some(note));
        }
        Err(err) => return kept(format!("auto-stash not restored: {err}")),
    }
    // Not `--quiet`: a conflict prints its `CONFLICT` lines on stdout.
    let args = ["stash", "pop"];
    match run_restoring_user_work(&args, cwd) {
        Ok(out) if out.status.success() => (false, None),
        Ok(out) if String::from_utf8_lossy(&out.stdout).contains("CONFLICT") => (true, None),
        Ok(out) => kept(format!(
            "restoring the auto-stash failed: {}",
            git_failure_message(&args, &out)
        )),
        Err(err) => kept(format!("restoring the auto-stash failed: {err}")),
    }
}

pub fn pull_quiet(cwd: &Path) -> bool {
    pull_quiet_detailed(cwd).ok
}

/// Whether `maybe_ancestor` is an ancestor of `tip`. `None` when git cannot decide.
pub fn is_ancestor(cwd: &Path, maybe_ancestor: &str, tip: &str) -> Option<bool> {
    match exec_git_status(&["merge-base", "--is-ancestor", maybe_ancestor, tip], cwd) {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

/// True when `HEAD` and `git_ref` resolve to the same commit SHA.
///
/// A just-created branch at the default tip is this case. Same-commit is
/// not merged work (`classify_merged_into_default`).
pub fn head_equals_ref(cwd: &Path, git_ref: &str) -> bool {
    match (rev_parse_quiet("HEAD", cwd), rev_parse_quiet(git_ref, cwd)) {
        (Some(head), Some(tip)) if !head.is_empty() && !tip.is_empty() => head == tip,
        _ => false,
    }
}

/// First existing tip among `origin/<default>` then `<default>`.
pub fn resolve_default_branch_tip_ref(cwd: &Path, default_branch: &str) -> Option<String> {
    let origin = format!("origin/{default_branch}");
    for git_ref in [origin.as_str(), default_branch] {
        let verify = format!("{git_ref}^{{commit}}");
        if exec_git_status(&["rev-parse", "--verify", "--quiet", &verify], cwd) == 0 {
            return Some(git_ref.to_string());
        }
    }
    None
}

/// The repository's default branch when it is known: the non-empty
/// configured override, else the branch `origin/HEAD` points at. `None`
/// when neither names one; no branch name is guessed. Merge-into-default
/// classification uses `main` for `None`.
pub fn known_default_branch_name(cwd: &Path, override_name: Option<&str>) -> Option<String> {
    if let Some(name) = override_name {
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let remote_head = exec_git(
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        cwd,
    );
    if remote_head.is_empty() {
        return None;
    }
    Some(
        remote_head
            .strip_prefix("origin/")
            .map_or_else(|| remote_head.clone(), str::to_string),
    )
}

/// Default branch used by `--default-branch` (origin/HEAD, then develop/main/master).
pub fn get_default_branch(cwd: &Path, override_name: Option<&str>) -> Option<String> {
    if let Some(name) = override_name {
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let remote_head = exec_git(
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        cwd,
    );
    if !remote_head.is_empty() {
        if let Some(rest) = remote_head.strip_prefix("origin/") {
            return Some(rest.to_string());
        }
        return Some(remote_head);
    }
    for name in ["develop", "main", "master"] {
        let remote = format!("refs/remotes/origin/{name}");
        if exec_git_status(&["show-ref", "--verify", &remote], cwd) == 0 {
            return Some(name.to_string());
        }
    }
    for name in ["develop", "main", "master"] {
        let local = format!("refs/heads/{name}");
        if exec_git_status(&["show-ref", "--verify", &local], cwd) == 0 {
            return Some(name.to_string());
        }
    }
    None
}

/// `git worktree list --porcelain` stdout (empty on failure).
pub fn list_worktrees_porcelain(cwd: &Path) -> String {
    exec_git(&["worktree", "list", "--porcelain"], cwd)
}

/// `path` as a pathspec that matches only that path: `:(literal)<path>`.
///
/// A bare pathspec is a glob, so `a[b].txt` also matches `ab.txt` and a
/// write command would act on both. Write commands that take a file path
/// pass it through this. When `GIT_LITERAL_PATHSPECS` is on, git reads
/// every pathspec literally and does not parse the `:(literal)` prefix, so
/// the path goes through as-is.
fn literal_pathspec(path: &str) -> String {
    let literal_global = std::env::var("GIT_LITERAL_PATHSPECS")
        .map(|v| git_env_bool(&v))
        .unwrap_or(false);
    literal_pathspec_with(path, literal_global)
}

fn literal_pathspec_with(path: &str, literal_global: bool) -> String {
    if literal_global {
        path.to_string()
    } else {
        format!(":(literal){path}")
    }
}

/// A git boolean env value: `true`/`yes`/`on` or a non-zero integer is on;
/// `false`/`no`/`off`, empty, zero, or anything else is off.
fn git_env_bool(value: &str) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => true,
        other => other.parse::<i64>().is_ok_and(|n| n != 0),
    }
}

/// Stage one path (`git add -- :(literal)<path>`).
pub fn stage_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["add", "--", &literal_pathspec(file_path)], cwd)
}

/// Unstage one path (`git restore --staged -- :(literal)<path>`).
pub fn unstage_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(
        &["restore", "--staged", "--", &literal_pathspec(file_path)],
        cwd,
    )
}

/// Apply a unified patch to the index (`git apply --cached`).
///
/// `reverse` is `git apply --reverse --cached` (unstage selected lines).
/// Stdin carries the patch. Other git wrappers attach stdin to `/dev/null`.
pub fn apply_cached_patch(cwd: &Path, patch: &str, reverse: bool) -> Result<(), String> {
    let mut args: Vec<&str> = vec!["apply", "--cached", "--unidiff-zero", "--whitespace=nowarn"];
    if reverse {
        args.push("--reverse");
    }
    args.push("-");
    apply_patch_stdin(cwd, patch, &args)
}

/// Discard a unified patch from the worktree (`git apply --reverse`).
///
/// No `--cached`: the index stays untouched. Stdin carries the patch.
pub fn apply_worktree_patch_reverse(cwd: &Path, patch: &str) -> Result<(), String> {
    apply_patch_stdin(
        cwd,
        patch,
        &[
            "apply",
            "--reverse",
            "--unidiff-zero",
            "--whitespace=nowarn",
            "-",
        ],
    )
}

fn apply_patch_stdin(cwd: &Path, patch: &str, args: &[&str]) -> Result<(), String> {
    if patch.trim().is_empty() {
        return Err("empty patch".into());
    }
    match run_with_stdin(args, cwd, patch.as_bytes()) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Discard worktree changes to a tracked path
/// (`git restore -- :(literal)<path>`).
pub fn revert_tracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["restore", "--", &literal_pathspec(file_path)], cwd)
}

/// Start of every [`ensure_compare_head_clean`] refusal.
///
/// The TUI shows a refusal as-is (warn), not as a failed git write.
pub const COMPARE_REVERT_ABORTED: &str = "revert aborted: ";

/// Refuse a compare-tab write unless the checkout still is the compare head.
///
/// `Err` when `HEAD` is not `head`, or when `git status` lists any of
/// `paths` (staged, unstaged, untracked, or ignored). A compare tab writes
/// the worktree only while it equals `head` for those paths, so this runs
/// on the write worker right before the write: the TUI snapshot can be a
/// watch tick old. `paths` are matched literally ([`literal_pathspec`]).
pub fn ensure_compare_head_clean(cwd: &Path, head: &str, paths: &[&str]) -> Result<(), String> {
    match rev_parse_commit(cwd, "HEAD")? {
        Some(now) if now == head => {}
        _ => return Err(format!("{COMPARE_REVERT_ABORTED}HEAD moved")),
    }
    let specs: Vec<String> = paths.iter().map(|p| literal_pathspec(p)).collect();
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--ignored",
        "--",
    ];
    args.extend(specs.iter().map(String::as_str));
    let out = exec_git_stdout(&args, cwd)?;
    if out.trim().is_empty() {
        Ok(())
    } else {
        let path = paths.first().copied().unwrap_or_default();
        Err(format!(
            "{COMPARE_REVERT_ABORTED}{path} has uncommitted changes"
        ))
    }
}

/// Restore `paths` in the worktree from `rev`
/// (`git restore --source=<rev> --worktree -- :(literal)<path>...`).
///
/// The index stays untouched. A tracked path that `rev` does not have is
/// removed from the worktree; a path only `rev` has is written.
pub fn restore_worktree_from(cwd: &Path, rev: &str, paths: &[&str]) -> Result<(), String> {
    let source = format!("--source={rev}");
    let specs: Vec<String> = paths.iter().map(|p| literal_pathspec(p)).collect();
    let mut args = vec!["restore", source.as_str(), "--worktree", "--"];
    args.extend(specs.iter().map(String::as_str));
    exec_git_stdout(&args, cwd).map(|_| ())
}

/// Paths a compare whole-file revert touches: `old_path` (renames) first.
pub fn compare_revert_paths<'a>(path: &'a str, old_path: Option<&'a str>) -> Vec<&'a str> {
    match old_path {
        Some(old) if old != path => vec![old, path],
        _ => vec![path],
    }
}

/// Compare `x` on a file: restore it in the worktree from `merge_base`.
///
/// Checks [`ensure_compare_head_clean`] first. Status `M` restores the
/// content, `A` deletes the file, `D` writes it back, and `R` writes
/// `old_path` back and deletes `path`. The index stays untouched.
pub fn revert_compare_file(
    cwd: &Path,
    merge_base: &str,
    head: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<(), String> {
    let paths = compare_revert_paths(path, old_path);
    ensure_compare_head_clean(cwd, head, &paths)?;
    restore_worktree_from(cwd, merge_base, &paths)
}

/// Compare `x` on highlighted lines: `git apply --reverse` of `patch`.
///
/// Checks [`ensure_compare_head_clean`] for `path` first. The patch is a
/// slice of the committed `merge_base...head` diff, so its post-image is
/// the head blob, which the check proves is the worktree file.
pub fn revert_compare_patch(cwd: &Path, head: &str, path: &str, patch: &str) -> Result<(), String> {
    ensure_compare_head_clean(cwd, head, &[path])?;
    apply_worktree_patch_reverse(cwd, patch)
}

/// Delete an untracked path (`git clean -f -- :(literal)<path>`). Destructive.
pub fn remove_untracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["clean", "-f", "--", &literal_pathspec(file_path)], cwd)
}

/// What [`push_quiet`] will do in a checkout, read before it runs.
///
/// The TUI push confirm names this branch and remote, and says when the
/// push creates the remote branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushPlan {
    /// Current branch (`git branch --show-current`).
    pub branch: String,
    /// Remote the push goes to: `branch.<branch>.remote`, else the first
    /// configured remote, else `origin`.
    pub remote: String,
    /// True when the push publishes the branch (`git push -u <remote> HEAD`):
    /// no upstream yet, or the upstream is not `<remote>/<branch>`.
    pub publish: bool,
}

/// Read the [`PushPlan`] for `cwd`. `Err` on a detached HEAD.
pub fn push_plan(cwd: &Path) -> Result<PushPlan, String> {
    let branch = exec_git(&["branch", "--show-current"], cwd);
    if branch.is_empty() {
        return Err("detached HEAD cannot push".into());
    }
    let publish = needs_upstream_publish(cwd, &branch);
    let remote = push_remote_name(cwd, &branch);
    Ok(PushPlan {
        branch,
        remote,
        publish,
    })
}

/// `git push --quiet`. First publish uses `git push -u <remote> HEAD`.
pub fn push_quiet(cwd: &Path) -> Result<(), String> {
    let plan = push_plan(cwd)?;
    if plan.publish {
        exec_git_checked(&["push", "-u", &plan.remote, "HEAD", "--quiet"], cwd)
    } else {
        exec_git_checked(&["push", "--quiet"], cwd)
    }
}

fn needs_upstream_publish(cwd: &Path, branch: &str) -> bool {
    let upstream = exec_git(&["rev-parse", "--abbrev-ref", "@{upstream}"], cwd);
    if upstream.is_empty() {
        return true;
    }
    let key = format!("branch.{branch}.remote");
    let remote = exec_git(&["config", "--get", &key], cwd);
    let remote = if remote.is_empty() {
        "origin".to_string()
    } else {
        remote
    };
    let prefix = format!("{remote}/");
    if !upstream.starts_with(&prefix) {
        return true;
    }
    &upstream[prefix.len()..] != branch
}

/// Name of the remote `push` would use for `branch`.
///
/// `branch.<branch>.remote`, else the first configured remote (`git
/// remote`), else `origin`. The name is not checked: it may name a remote
/// that does not exist.
pub fn push_remote_name(cwd: &Path, branch: &str) -> String {
    let key = format!("branch.{branch}.remote");
    let configured = exec_git(&["config", "--get", &key], cwd);
    if !configured.is_empty() {
        return configured;
    }
    first_remote_name(cwd)
}

/// The first configured remote (`git remote`), else `origin`.
fn first_remote_name(cwd: &Path) -> String {
    exec_git(&["remote"], cwd)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("origin")
        .to_string()
}

/// Name of the remote for an entity of the whole checkout (repo home,
/// commit, tag).
///
/// The push remote of the current branch (`symbolic-ref --short -q HEAD`,
/// then [`push_remote_name`]). On a detached HEAD: the first configured
/// remote, else `origin`. The name is not checked.
pub fn checkout_remote_name(cwd: &Path) -> String {
    let current = exec_git(&["symbolic-ref", "--short", "-q", "HEAD"], cwd);
    if current.is_empty() {
        first_remote_name(cwd)
    } else {
        push_remote_name(cwd, &current)
    }
}

/// Split a remote-tracking short name such as `origin/feature/x` into the
/// remote and the branch on it: `("origin", "feature/x")`.
///
/// The remote is the longest configured remote (`git remote`) whose name
/// followed by `/` starts `short_ref`, so a remote named `team/a` wins over
/// `team`. `None` when no configured remote matches or the branch part is
/// empty.
pub fn split_remote_ref(cwd: &Path, short_ref: &str) -> Option<(String, String)> {
    exec_git(&["remote"], cwd)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter_map(|name| {
            let rest = short_ref.strip_prefix(name)?.strip_prefix('/')?;
            (!rest.is_empty()).then(|| (name.to_string(), rest.to_string()))
        })
        .max_by_key(|(name, _)| name.len())
}

/// Fetch URL of remote `remote` (`git remote get-url <remote>`), or `None`.
///
/// `url.<base>.insteadOf` rewrites apply. `None` when the remote does not
/// exist or git prints no URL.
pub fn remote_url(cwd: &Path, remote: &str) -> Option<String> {
    exec_git_stdout(&["remote", "get-url", remote], cwd)
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Fetch URL of the remote that `branch` pushes to, or `None`.
///
/// The remote is the one `push` would use ([`push_remote_name`]). The URL
/// comes from [`remote_url`].
pub fn remote_url_for_branch(cwd: &Path, branch: &str) -> Option<String> {
    remote_url(cwd, &push_remote_name(cwd, branch))
}

/// True when the remote-tracking ref `refs/remotes/<remote>/<branch>`
/// exists (`show-ref --verify --quiet`). Local only: it reads what the last
/// fetch or push left.
pub fn remote_branch_exists(cwd: &Path, remote: &str, branch: &str) -> bool {
    let full = format!("refs/remotes/{remote}/{branch}");
    exec_git_status(&["show-ref", "--verify", "--quiet", &full], cwd) == 0
}

/// True when some remote-tracking branch of `remote` contains commit `sha`
/// (`for-each-ref --count=1 --contains <sha> refs/remotes/<remote>/`).
///
/// Local only: it reads the remote-tracking refs. An unknown `sha` is
/// `false`.
pub fn commit_on_remote(cwd: &Path, remote: &str, sha: &str) -> bool {
    let refs = format!("refs/remotes/{remote}/");
    let args = [
        "for-each-ref",
        "--count=1",
        "--contains",
        sha,
        "--format=%(refname)",
        &refs,
    ];
    !exec_git(&args, cwd).is_empty()
}

/// True when `<rev>:<path>` names an object (`cat-file -e`): a file or a
/// directory at `path` in `rev`. An empty `path` is the root tree.
pub fn rev_has_path(cwd: &Path, rev: &str, path: &str) -> bool {
    let spec = format!("{rev}:{path}");
    exec_git_status(&["cat-file", "-e", &spec], cwd) == 0
}

/// Whether remote `remote` has tag `tag`, asked over the network with
/// `ls-remote --tags --exit-code <remote> refs/tags/<tag>`.
///
/// `Some(true)` on exit 0, `Some(false)` on exit 2 (no matching ref),
/// `None` when the check failed (no answer, auth failure, timeout). The run
/// has the remote deadlines of [`git_output`].
pub fn remote_has_tag(cwd: &Path, remote: &str, tag: &str) -> Option<bool> {
    let pattern = format!("refs/tags/{tag}");
    let args = ["ls-remote", "--tags", "--exit-code", remote, &pattern];
    match exec_git_status(&args, cwd) {
        0 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Stash worktree changes. Includes untracked files (`-u`).
///
/// Non-empty `paths` limit the stash to those paths, matched literally
/// ([`literal_pathspec`]).
pub fn stash_push(cwd: &Path, paths: &[String]) -> Result<(), String> {
    let specs: Vec<String> = paths.iter().map(|p| literal_pathspec(p)).collect();
    let mut args: Vec<&str> = vec!["stash", "push", "-u"];
    if !specs.is_empty() {
        args.push("--");
        args.extend(specs.iter().map(String::as_str));
    }
    let pushed = stash_push_entry(&args, cwd)?;
    pushed.pushed?;
    match pushed.entry? {
        Some(_) => Ok(()),
        None => Err("no local changes to save".into()),
    }
}

/// What a `stash push` did ([`stash_push_entry`]).
struct StashPushed {
    /// The push's own result: git's reason when it failed or was stopped.
    pushed: Result<(), String>,
    /// The new entry's commit id when `refs/stash` moved, `None` when it
    /// did not, `Err` when it could not be read after the push.
    entry: Result<Option<String>, String>,
}

/// Run the `stash push` in `args` and find the entry it made, if any.
///
/// `stash push` exits 0 and prints `No local changes to save` when it
/// saves nothing, and it can fail or be stopped after it saved an entry,
/// so the exit code cannot say. This compares `refs/stash` before and
/// after the push, whatever the push returned. The read after the push
/// still starts while `ws` exits: a quit during or right after the push
/// must not hide the entry from the caller that gives the work back.
/// `Err` only when `refs/stash` could not be read before the push (the
/// push did not run).
fn stash_push_entry(args: &[&str], cwd: &Path) -> Result<StashPushed, String> {
    let before = stash_tip(cwd, run)?;
    let pushed = exec_git_checked(args, cwd);
    let entry = stash_tip(cwd, run_restoring_user_work)
        .map(|after| after.filter(|after| Some(after) != before.as_ref()));
    Ok(StashPushed { pushed, entry })
}

/// The commit `refs/stash` points at, or `None` when there is no stash.
///
/// `Err` when git could not say (a timeout, an error exit other than the
/// missing-ref exit 1), so a failed read never looks like "no stash".
fn stash_tip(
    cwd: &Path,
    runner: fn(&[&str], &Path) -> io::Result<Output>,
) -> Result<Option<String>, String> {
    let args = ["rev-parse", "-q", "--verify", "refs/stash"];
    match runner(&args, cwd) {
        Ok(out) if out.status.success() => Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        )),
        Ok(out) if out.status.code() == Some(1) => Ok(None),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Apply a stash and keep the entry.
pub fn stash_apply(cwd: &Path, stash_ref: &str) -> Result<(), String> {
    exec_git_checked(&["stash", "apply", stash_ref], cwd)
}

/// Pop a stash entry (apply then drop).
pub fn stash_pop(cwd: &Path, stash_ref: &str) -> Result<(), String> {
    exec_git_checked(&["stash", "pop", stash_ref], cwd)
}

/// Drop a stash entry.
pub fn stash_drop(cwd: &Path, stash_ref: &str) -> Result<(), String> {
    exec_git_checked(&["stash", "drop", stash_ref], cwd)
}

/// Stash refs newest first (`stash@{0}`, …).
pub fn list_stash_refs(cwd: &Path) -> Vec<String> {
    exec_git(&["stash", "list", "--format=%gd"], cwd)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Newest stash ref, if any.
pub fn latest_stash_ref(cwd: &Path) -> Option<String> {
    list_stash_refs(cwd).into_iter().next()
}

/// One local branch for the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBranch {
    pub name: String,
    pub current: bool,
    pub authordate: i64,
}

/// Local branches only (no remotes).
pub fn list_local_branches(cwd: &Path) -> Vec<LocalBranch> {
    let raw = exec_git(
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(authordate:unix)\t%(HEAD)",
            "refs/heads/",
        ],
        cwd,
    );
    raw.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.to_string();
            if name.is_empty() {
                return None;
            }
            let authordate = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let current = parts.next() == Some("*");
            Some(LocalBranch {
                name,
                current,
                authordate,
            })
        })
        .collect()
}

/// `origin/<branch>` when that ref exists and differs from the local tip.
pub fn origin_out_of_sync(cwd: &Path, branch: &str) -> Option<String> {
    let origin = format!("origin/{branch}");
    let local = rev_parse_quiet(branch, cwd)?;
    let remote = rev_parse_quiet(&origin, cwd)?;
    if local == remote {
        None
    } else {
        Some(origin)
    }
}

/// Drop a linked worktree. Runs `git worktree remove [--force] <path>` from the primary.
pub fn remove_worktree(primary_abs: &Path, worktree_abs: &Path, force: bool) -> Result<(), String> {
    let porcelain = list_worktrees_porcelain(primary_abs);
    let entries = crate::worktrees::parse_worktree_list_porcelain(&porcelain);
    let target =
        crate::worktrees::resolve_worktree_remove_target(&entries, primary_abs, worktree_abs);
    let path = target.git_path.to_string_lossy().into_owned();
    if force {
        exec_git_checked(&["worktree", "remove", "--force", &path], &target.git_cwd)
    } else {
        exec_git_checked(&["worktree", "remove", &path], &target.git_cwd)
    }
}

/// Create and check out a new branch at HEAD.
pub fn create_branch_checkout(cwd: &Path, name: &str) -> Result<(), String> {
    exec_git_checked(&["checkout", "-b", name, "--quiet"], cwd)
}

/// Argv for `git branch -- <name> <commitId>` (ref only, no checkout).
pub fn create_branch_at_args<'a>(name: &'a str, commit_id: &'a str) -> [&'a str; 4] {
    ["branch", "--", name, commit_id]
}

/// Create a local branch at `commit_id` without checking it out.
pub fn create_branch_at(cwd: &Path, name: &str, commit_id: &str) -> Result<(), String> {
    exec_git_checked(&create_branch_at_args(name, commit_id), cwd)
}

/// Added and deleted line counts for one file, from `git --numstat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStat {
    pub added: u32,
    pub deleted: u32,
}

/// One path from `git diff --name-status` / `diff-tree` / `stash show`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameStatus {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
    /// Line counts. `None` for binary and untracked files, and when numstat failed.
    pub stat: Option<LineStat>,
}

/// Run git and return stdout as-is (lossy UTF-8, not trimmed).
///
/// For `-z` output, where a leading or trailing space belongs to a path.
/// Failure is the same `Err` as [`exec_git_stdout`].
pub fn exec_git_z(args: &[&str], cwd: &Path) -> Result<String, String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Parse `--name-status -z` output (`M\0path\0`, `R100\0old\0new\0`).
///
/// Paths are raw, never C-quoted. A record cut short is dropped.
pub fn parse_name_status_z(stdout: &str) -> Vec<NameStatus> {
    let mut out = Vec::new();
    let mut tokens = stdout.split('\0');
    while let Some(status) = tokens.next() {
        let Some(letter) = status.chars().next() else {
            continue;
        };
        let old_path = if matches!(letter, 'R' | 'C') {
            match tokens.next().filter(|p| !p.is_empty()) {
                Some(old) => Some(old.to_string()),
                None => continue,
            }
        } else {
            None
        };
        let Some(path) = tokens.next().filter(|p| !p.is_empty()) else {
            continue;
        };
        out.push(NameStatus {
            status: letter.to_string(),
            path: path.to_string(),
            old_path,
            stat: None,
        });
    }
    out
}

/// Parse `git --numstat -z` output into `(path, counts)` rows.
///
/// A plain row is `added\tdeleted\tpath\0`. A rename or copy row is
/// `added\tdeleted\t\0old\0new\0`; the path is the new one. Binary files
/// print `-\t-` and give `None`.
pub fn parse_numstat_z(stdout: &str) -> Vec<(String, Option<LineStat>)> {
    let mut out = Vec::new();
    let mut tokens = stdout.split('\0').filter(|t| !t.is_empty());
    while let Some(token) = tokens.next() {
        let mut parts = token.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let stat = match (added.parse::<u32>(), deleted.parse::<u32>()) {
            (Ok(added), Ok(deleted)) => Some(LineStat { added, deleted }),
            _ => None,
        };
        let path = if path.is_empty() {
            let _old = tokens.next();
            match tokens.next() {
                Some(new) => new,
                None => continue,
            }
        } else {
            path
        };
        out.push((path.to_string(), stat));
    }
    out
}

/// Fill `stat` on each file from `--numstat -z` output, matched by new path.
fn attach_numstat(files: &mut [NameStatus], numstat_z: &str) {
    for (path, stat) in parse_numstat_z(numstat_z) {
        if let Some(file) = files.iter_mut().find(|f| f.path == path) {
            file.stat = stat;
        }
    }
}

/// First-parent files in `commit_id`. Root commits fall back to `--root`.
///
/// Line counts come from a second `--numstat -z` call with the same revisions.
/// A numstat failure leaves `stat` as `None`; the list still loads.
pub fn list_commit_name_status(cwd: &Path, commit_id: &str) -> Vec<NameStatus> {
    let parent = format!("{commit_id}^");
    let out = exec_git_z(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-z",
            "-r",
            &parent,
            commit_id,
        ],
        cwd,
    )
    .unwrap_or_default();
    if !out.is_empty() {
        let mut files = parse_name_status_z(&out);
        let numstat = exec_git(
            &[
                "diff-tree",
                "--no-commit-id",
                "--numstat",
                "-z",
                "-r",
                &parent,
                commit_id,
            ],
            cwd,
        );
        attach_numstat(&mut files, &numstat);
        return files;
    }
    let root = exec_git_z(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-z",
            "-r",
            "--root",
            commit_id,
        ],
        cwd,
    )
    .unwrap_or_default();
    let mut files = parse_name_status_z(&root);
    let numstat = exec_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "--numstat",
            "-z",
            "-r",
            "--root",
            commit_id,
        ],
        cwd,
    );
    attach_numstat(&mut files, &numstat);
    files
}

/// Files recorded in a stash entry, with line counts from `--numstat -z`.
pub fn list_stash_name_status(cwd: &Path, stash_ref: &str) -> Vec<NameStatus> {
    let mut files = parse_name_status_z(
        &exec_git_z(&["stash", "show", "--name-status", "-z", stash_ref], cwd).unwrap_or_default(),
    );
    let numstat = exec_git(&["stash", "show", "--numstat", "-z", stash_ref], cwd);
    attach_numstat(&mut files, &numstat);
    files
}

/// Worktree + index changes versus HEAD, plus untracked files.
///
/// Tracked files carry line counts from `diff HEAD --numstat -z`. Untracked
/// files have `stat: None`.
pub fn list_worktree_name_status(cwd: &Path) -> Vec<NameStatus> {
    let mut files = parse_name_status_z(
        &exec_git_z(&["diff", "HEAD", "--name-status", "-z"], cwd).unwrap_or_default(),
    );
    let numstat = exec_git(&["diff", "HEAD", "--numstat", "-z"], cwd);
    attach_numstat(&mut files, &numstat);
    let untracked =
        exec_git_z(&["ls-files", "-z", "--others", "--exclude-standard"], cwd).unwrap_or_default();
    for path in untracked.split('\0').filter(|p| !p.is_empty()) {
        if files.iter().any(|f| f.path == path) {
            continue;
        }
        files.push(NameStatus {
            status: "?".into(),
            path: path.to_string(),
            old_path: None,
            stat: None,
        });
    }
    files
}

fn lines_or_empty_diff(text: &str) -> Vec<String> {
    if text.is_empty() {
        vec!["(no diff)".into()]
    } else {
        text.lines().map(str::to_string).collect()
    }
}

/// Context large enough to show a typical source file in one hunk.
pub const FULL_DIFF_CONTEXT_LINES: u32 = 999_999;

/// Build `git diff` argv, inserting `-U{n}` when `context` is set.
pub fn git_diff_args(base: &[&str], path: &str, context: Option<u32>) -> Vec<String> {
    let mut args: Vec<String> = base.iter().map(|s| (*s).to_string()).collect();
    if let Some(n) = context {
        args.push(format!("-U{n}"));
    }
    args.push("--".into());
    // Literal: a line stage / unstage / revert builds its patch from this
    // diff, so a glob match of a sibling file would put that file's hunks
    // under this file's header.
    args.push(literal_pathspec(path));
    args
}

fn exec_git_owned(args: &[String], cwd: &Path) -> String {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    exec_git(&refs, cwd)
}

/// Git's reason line ([`git_reason_line`]), or `git <sub> exited with code N`.
///
/// A quoted `:(literal)` pathspec prefix ([`literal_pathspec`]) is dropped,
/// so the message names the path the user picked.
fn git_failure_message(args: &[&str], out: &std::process::Output) -> String {
    git_reason_line(out)
        .map(|line| line.replace("':(literal)", "'"))
        .unwrap_or_else(|| exit_code_message(args, out))
}

/// Run git and return stdout. Failure is `Err`, never an empty success.
pub fn exec_git_stdout(args: &[&str], cwd: &Path) -> Result<String, String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
        }
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// [`exec_git_stdout`] without the trim: every byte of stdout, trailing
/// whitespace and `\r` included.
///
/// For text a patch is built from (a `git diff` that `V` stage / unstage /
/// revert rewrites). Bytes that are not UTF-8 still decode lossily to
/// U+FFFD; `build_partial_patch` refuses a patch that holds one.
pub fn exec_git_stdout_untrimmed(args: &[&str], cwd: &Path) -> Result<String, String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Committer times (unix seconds) of the commits on local branches in the
/// last `days` days, newest first.
///
/// Runs `git log --branches --since=<days+1>.days.ago --format=%ct`. Local
/// branches only: the count shows local work, not commits other people
/// pushed and a fetch brought in. The extra day covers the oldest bucket in
/// full; the caller drops anything older. A repo git cannot read (no
/// commits, not a repo) has no activity: the result is empty.
pub fn commit_times_since(cwd: &Path, days: u32) -> Vec<i64> {
    let since = format!("--since={}.days.ago", u64::from(days) + 1);
    exec_git_stdout(&["log", "--branches", &since, "--format=%ct"], cwd)
        .map(|out| {
            out.lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Files git would show in a checkout: tracked plus untracked, minus ignored.
///
/// Runs `ls-files -z --cached --others --exclude-standard`, so `.gitignore`,
/// `.git/info/exclude`, and `core.excludesFile` apply. Exclude pathspecs keep
/// `target/` and `node_modules/` trees (at any depth) out of the listing
/// even when they are tracked or not ignored. Paths are relative to `cwd`
/// and decoded lossily. Failure is git's reason line, or
/// `git ls-files exited with code N`.
pub fn list_checkout_files(cwd: &Path) -> Result<Vec<String>, String> {
    let args = [
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
        ".",
        ":(exclude,glob)**/target/**",
        ":(exclude,glob)**/node_modules/**",
    ];
    match run(&args, cwd) {
        Ok(out) if out.status.success() => Ok(out
            .stdout
            .split(|b| *b == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect()),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Resolve `<ref>^{commit}`. Missing ref is `Ok(None)`. Other failures are `Err`.
pub fn rev_parse_commit(cwd: &Path, git_ref: &str) -> Result<Option<String>, String> {
    let verify = format!("{git_ref}^{{commit}}");
    let args = ["rev-parse", "--verify", "--quiet", verify.as_str()];
    match run(&args, cwd) {
        Ok(out) if out.status.success() => {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((!sha.is_empty()).then_some(sha))
        }
        Ok(out) if out.status.code() == Some(1) => Ok(None),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Three-dot merge base. Unrelated histories are `Ok(None)`.
pub fn merge_base(cwd: &Path, a: &str, b: &str) -> Result<Option<String>, String> {
    let args = ["merge-base", a, b];
    match run(&args, cwd) {
        Ok(out) if out.status.success() => {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((!sha.is_empty()).then_some(sha))
        }
        Ok(out) if out.status.code() == Some(1) => Ok(None),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Committed `base...HEAD` paths. Empty stdout is an empty list, not a failure.
///
/// Line counts come from a second `--numstat -z` call. Its failure leaves
/// `stat` as `None`; it does not fail the list.
pub fn list_compare_name_status(
    cwd: &Path,
    base_sha: &str,
    head_sha: &str,
) -> Result<Vec<NameStatus>, String> {
    let range = format!("{base_sha}...{head_sha}");
    let stdout = exec_git_z(
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            &range,
            "--",
        ],
        cwd,
    )?;
    let mut files = parse_name_status_z(&stdout);
    let numstat = exec_git_stdout(
        &["diff", "--numstat", "-z", "--find-renames", &range, "--"],
        cwd,
    )
    .unwrap_or_default();
    attach_numstat(&mut files, &numstat);
    Ok(files)
}

/// Patch key per path of `base...HEAD`, from `diff --raw -z --no-abbrev
/// --find-renames`.
///
/// The key holds the old and new modes, the old and new blob ids, the
/// status letter and the path(s). Two keys are equal only when the patch
/// for that path is the same, whatever commits produced it. Keyed by the
/// raw new path, the same spelling [`list_compare_name_status`] lists (both
/// read `-z` output). A failed git call gives an empty map; the file list
/// does not depend on it.
pub fn list_compare_patch_keys(
    cwd: &Path,
    base_sha: &str,
    head_sha: &str,
) -> std::collections::HashMap<String, String> {
    let range = format!("{base_sha}...{head_sha}");
    let raw = exec_git_z(
        &[
            "diff",
            "--raw",
            "-z",
            "--no-abbrev",
            "--find-renames",
            &range,
            "--",
        ],
        cwd,
    )
    .unwrap_or_default();
    parse_raw_patch_keys(&raw)
}

/// Parse `git diff --raw -z` output into new path → patch key.
///
/// A record is `:<old mode> <new mode> <old id> <new id> <status>[score]\0<path>\0`.
/// A rename or copy record ends `\0<old path>\0<new path>\0`. Paths are raw,
/// like [`parse_name_status_z`]. The similarity score is left out of the
/// key: the blob ids already fix it.
fn parse_raw_patch_keys(stdout: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut tokens = stdout.split('\0');
    while let Some(token) = tokens.next() {
        let Some(meta) = token.strip_prefix(':') else {
            continue;
        };
        let fields: Vec<&str> = meta.split(' ').collect();
        let [old_mode, new_mode, old_id, new_id, status] = fields[..] else {
            continue;
        };
        let Some(letter) = status.chars().next() else {
            continue;
        };
        let Some(first) = tokens.next().filter(|p| !p.is_empty()) else {
            continue;
        };
        let (old_path, path) = if matches!(letter, 'R' | 'C') {
            let Some(new) = tokens.next().filter(|p| !p.is_empty()) else {
                continue;
            };
            (first, new)
        } else {
            (first, first)
        };
        out.insert(
            path.to_string(),
            format!("{old_mode} {new_mode} {old_id} {new_id} {letter}\0{old_path}\0{path}"),
        );
    }
    out
}

/// `git diff` argv for one compare path, with optional rename old path.
pub fn git_compare_diff_args(
    base_sha: &str,
    head_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Vec<String> {
    let range = format!("{base_sha}...{head_sha}");
    let mut args = vec!["diff".to_string()];
    if let Some(n) = context {
        args.push(format!("-U{n}"));
    }
    args.push(range);
    args.push("--".into());
    if let Some(old) = old_path {
        if old != path {
            args.push(old.to_string());
        }
    }
    args.push(path.into());
    args
}

/// Unified compare diff for one path. Success with empty stdout is `(no diff)`.
///
/// Lines split on `\n` only and keep their bytes (a CRLF line keeps its
/// `\r`, trailing whitespace stays), because a compare revert builds its
/// patch from them.
pub fn diff_compare_file_ctx(
    cwd: &Path,
    base_sha: &str,
    head_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Result<Vec<String>, String> {
    let args = git_compare_diff_args(base_sha, head_sha, path, old_path, context);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let stdout = exec_git_stdout_untrimmed(&refs, cwd)?;
    if stdout.trim().is_empty() {
        return Ok(vec!["(no diff)".into()]);
    }
    let body = stdout.strip_suffix('\n').unwrap_or(&stdout);
    Ok(body.split('\n').map(str::to_string).collect())
}

/// `git diff` argv for one path, commit `base_sha` against the working tree.
///
/// `diff [-U<n>] -M <base> -- [<old>] <path>`: git's own commit-vs-worktree
/// form, so no revision stands for the working tree. `old_path` joins the
/// pathspec when it differs from `path`, so `-M` can pair a rename.
pub fn git_worktree_compare_diff_args(
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Vec<String> {
    let mut args = vec!["diff".to_string()];
    if let Some(n) = context {
        args.push(format!("-U{n}"));
    }
    args.push("-M".into());
    args.push(base_sha.into());
    args.push("--".into());
    args.extend(
        compare_revert_paths(path, old_path)
            .into_iter()
            .map(String::from),
    );
    args
}

/// The one path of a commit-vs-working-tree compare, as a name-status row.
///
/// Runs `diff --name-status -z -M <base> -- [<old>] <path>`, then the same with
/// `--numstat -z` for line counts (its failure leaves `stat` as `None`).
/// Empty stdout (the worktree file equals the commit) is an empty list.
pub fn list_worktree_vs_commit_name_status(
    cwd: &Path,
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<Vec<NameStatus>, String> {
    let paths = compare_revert_paths(path, old_path);
    let mut args = vec!["diff", "--name-status", "-z", "-M", base_sha, "--"];
    args.extend_from_slice(&paths);
    let mut files = parse_name_status_z(&exec_git_z(&args, cwd)?);
    let mut numstat_args = vec!["diff", "--numstat", "-z", "-M", base_sha, "--"];
    numstat_args.extend_from_slice(&paths);
    let numstat = exec_git_stdout(&numstat_args, cwd).unwrap_or_default();
    attach_numstat(&mut files, &numstat);
    Ok(files)
}

/// Unified diff of one path, commit `base_sha` against the working tree.
///
/// Argv from [`git_worktree_compare_diff_args`]. Success with empty stdout
/// is `(no diff)`.
pub fn diff_worktree_vs_commit_file_ctx(
    cwd: &Path,
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Result<Vec<String>, String> {
    let args = git_worktree_compare_diff_args(base_sha, path, old_path, context);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(lines_or_empty_diff(&exec_git_stdout(&refs, cwd)?))
}

/// Local branches plus `origin/*`, excluding `origin/HEAD` and the current local.
pub fn list_compare_picker_branches(cwd: &Path) -> Result<Vec<LocalBranch>, String> {
    let raw = exec_git_stdout(
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(authordate:unix)\t%(HEAD)",
            "refs/heads/",
            "refs/remotes/origin/",
        ],
        cwd,
    )?;
    Ok(parse_compare_picker_branches(&raw))
}

/// Pure filter for [`list_compare_picker_branches`].
pub fn parse_compare_picker_branches(raw: &str) -> Vec<LocalBranch> {
    raw.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.to_string();
            if name.is_empty() || name == "origin/HEAD" {
                return None;
            }
            let authordate = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let current = parts.next() == Some("*");
            if current {
                return None;
            }
            Some(LocalBranch {
                name,
                current: false,
                authordate,
            })
        })
        .collect()
}

/// Newest HEAD ancestors the compare commit picker lists (HEAD itself is
/// not counted). Older history is cut off so a huge repo still opens the
/// picker quickly; type a sha or subject to find a listed commit.
pub const COMPARE_PICKER_COMMIT_LIMIT: usize = 10_000;

/// One ancestor row in the compare commit picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AncestorCommit {
    /// Full object id.
    pub id: String,
    /// First line of the message.
    pub subject: String,
}

/// HEAD's ancestors, newest first, without HEAD: at most
/// [`COMPARE_PICKER_COMMIT_LIMIT`] rows. A root HEAD lists none.
///
/// Runs `git log HEAD` with `--max-count=<limit + 1>` and drops the first
/// row (the walk starts at HEAD), which leaves at most `limit` rows.
/// `HEAD^@` is not used: on a root commit it expands to nothing and
/// `git log` would fall back to HEAD.
pub fn list_compare_picker_commits(cwd: &Path) -> Result<Vec<AncestorCommit>, String> {
    list_compare_picker_commits_capped(cwd, COMPARE_PICKER_COMMIT_LIMIT)
}

fn list_compare_picker_commits_capped(
    cwd: &Path,
    limit: usize,
) -> Result<Vec<AncestorCommit>, String> {
    let max_count = format!("--max-count={}", limit.saturating_add(1));
    let raw = exec_git_stdout(&["log", "--format=%H%x09%s", &max_count, "HEAD", "--"], cwd)?;
    Ok(raw
        .lines()
        .skip(1)
        .filter_map(parse_compare_picker_line)
        .collect())
}

/// One `%H%x09%s` line of the compare picker log. The subject goes through
/// [`paintable_line`], so a tab or a control byte in a commit subject
/// cannot reach the terminal. `None` for a line with no id.
fn parse_compare_picker_line(line: &str) -> Option<AncestorCommit> {
    let (id, subject) = line.split_once('\t').unwrap_or((line, ""));
    (!id.is_empty()).then(|| AncestorCommit {
        id: id.to_string(),
        subject: paintable_line(subject),
    })
}

/// First-parent unified diff for one path in a commit.
pub fn diff_commit_file(cwd: &Path, commit_id: &str, path: &str) -> Vec<String> {
    diff_commit_file_ctx(cwd, commit_id, path, None)
}

/// First-parent unified diff with optional `-U` context.
pub fn diff_commit_file_ctx(
    cwd: &Path,
    commit_id: &str,
    path: &str,
    context: Option<u32>,
) -> Vec<String> {
    let parent = format!("{commit_id}^");
    let args = git_diff_args(&["diff", &parent, commit_id], path, context);
    let primary = exec_git_owned(&args, cwd);
    if !primary.is_empty() {
        return primary.lines().map(str::to_string).collect();
    }
    lines_or_empty_diff(&exec_git(
        &["show", "--first-parent", commit_id, "--", path],
        cwd,
    ))
}

/// First-parent unified diff for one path inside a stash.
pub fn diff_stash_file(cwd: &Path, stash_ref: &str, path: &str) -> Vec<String> {
    diff_stash_file_ctx(cwd, stash_ref, path, None)
}

/// Stash-file unified diff with optional `-U` context.
pub fn diff_stash_file_ctx(
    cwd: &Path,
    stash_ref: &str,
    path: &str,
    context: Option<u32>,
) -> Vec<String> {
    let parent = format!("{stash_ref}^1");
    let args = git_diff_args(&["diff", &parent, stash_ref], path, context);
    lines_or_empty_diff(&exec_git_owned(&args, cwd))
}

/// Which version of a file [`blame_line`] reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlameRev {
    /// The working-tree file (no revision).
    Worktree,
    /// The index blob `:<path>`, fed through `--contents -`.
    Index,
    /// A revision git can resolve (`HEAD`, a full sha, `<sha>^`, a stash ref).
    Commit(String),
}

/// Who last changed one line, from `git blame --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBlame {
    /// Full commit id. All zeros when the line is not committed.
    pub sha: String,
    /// `author` header, paintable (tabs expanded, control characters as
    /// U+FFFD). For an uncommitted line it is a git placeholder; use
    /// [`LineBlame::uncommitted`], not this name, to detect that case.
    pub author: String,
    /// `author-time`, unix seconds.
    pub author_time: i64,
    /// `summary` header: the commit subject, paintable like `author`.
    pub summary: String,
    /// 1-based line number in that commit's version of the file.
    pub orig_line: u32,
    /// Path of the file in that commit (differs after a rename).
    pub filename: String,
    /// `previous <sha> <path>`: the commit blame looked at before `sha`
    /// and the file's path there. `None` when the file was added in `sha`.
    pub previous: Option<(String, String)>,
    /// `boundary`: git did not look past `sha` (a root commit here).
    pub boundary: bool,
    /// The line is not committed yet (all-zero sha).
    pub uncommitted: bool,
}

/// Parse `git blame --porcelain` output for one line. `None` when malformed.
///
/// Reads the first entry only: `<sha> <orig> <final> [<count>]`, then
/// headers up to the tab-prefixed content line.
pub fn parse_blame_porcelain(stdout: &str) -> Option<LineBlame> {
    let mut lines = stdout.lines();
    let mut head = lines.next()?.split_whitespace();
    let sha = head.next()?.to_string();
    if sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let orig_line = head.next()?.parse().ok()?;
    let mut blame = LineBlame {
        uncommitted: sha.bytes().all(|b| b == b'0'),
        sha,
        author: String::new(),
        author_time: 0,
        summary: String::new(),
        orig_line,
        filename: String::new(),
        previous: None,
        boundary: false,
    };
    for line in lines {
        if line.starts_with('\t') {
            break;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            // Author and subject only paint: control bytes must not reach
            // the terminal.
            "author" => blame.author = paintable_line(value),
            "author-time" => blame.author_time = value.parse().unwrap_or(0),
            "summary" => blame.summary = paintable_line(value),
            "filename" => blame.filename = unquote_c_path(value),
            "boundary" => blame.boundary = true,
            "previous" => {
                blame.previous = value
                    .split_once(' ')
                    .map(|(sha, path)| (sha.to_string(), unquote_c_path(path)));
            }
            _ => {}
        }
    }
    (!blame.filename.is_empty()).then_some(blame)
}

/// Undo git's C-style path quoting (`"na\303\257ve.txt"` → `naïve.txt`).
///
/// Git quotes a path that holds `"`, `\`, or a control character (and
/// any non-ASCII byte unless `core.quotePath=false`). Octal escapes are
/// raw bytes, decoded as UTF-8 (lossy). An unquoted value is returned as is.
fn unquote_c_path(value: &str) -> String {
    match value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        Some(_) => String::from_utf8_lossy(&unquote_c_path_bytes(value)).into_owned(),
        None => value.to_string(),
    }
}

/// Raw bytes of a path git may have C-quoted (`"a b"`, `"\303\251.rs"`).
///
/// A value in surrounding `"` has its escapes decoded (`\"`, `\\`, `\t`,
/// `\n`, the other C letter escapes, and 1-3 digit octal bytes). Any other
/// value comes back as its own bytes, untouched.
fn unquote_c_path_bytes(value: &str) -> Vec<u8> {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.as_bytes().to_vec();
    };
    let mut bytes = Vec::with_capacity(inner.len());
    let mut iter = inner.bytes().peekable();
    while let Some(b) = iter.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        let Some(esc) = iter.next() else {
            bytes.push(b);
            break;
        };
        let decoded = match esc {
            b'a' => 0x07,
            b'b' => 0x08,
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => 0x0b,
            b'f' => 0x0c,
            b'r' => b'\r',
            b'0'..=b'7' => {
                let mut n = u32::from(esc - b'0');
                for _ in 0..2 {
                    match iter.peek() {
                        Some(d @ b'0'..=b'7') => {
                            n = n * 8 + u32::from(d - b'0');
                            iter.next();
                        }
                        _ => break,
                    }
                }
                n as u8
            }
            other => other,
        };
        bytes.push(decoded);
    }
    bytes
}

/// Blame line `line` (1-based) of `path` in `rev`.
///
/// Runs `git -c blame.showRoot=false -c core.quotePath=false blame
/// --porcelain -L n,n [<rev>] -- <path>` (the runner adds `--no-textconv`,
/// see `with_diff_driver_off`). Paths in the result are unquoted.
/// [`BlameRev::Index`] feeds the index blob through `--contents -`, so lines
/// that differ from HEAD come back uncommitted. `Ok(None)` when git exits
/// non-zero (untracked or missing path, unborn HEAD, line out of range) or
/// the path has no index entry. `Err` only when git cannot start.
/// TTY callers must run this on `spawn_blocking`, not the event loop.
pub fn blame_line(
    cwd: &Path,
    rev: &BlameRev,
    path: &str,
    line: u32,
) -> Result<Option<LineBlame>, String> {
    if line == 0 {
        return Ok(None);
    }
    let range = format!("{line},{line}");
    let mut args = vec![
        "-c",
        "blame.showRoot=false",
        "-c",
        "core.quotePath=false",
        "blame",
        "--porcelain",
        "-L",
        &range,
    ];
    match rev {
        BlameRev::Worktree => {}
        BlameRev::Index => args.extend_from_slice(&["--contents", "-"]),
        BlameRev::Commit(rev) => args.push(rev),
    }
    args.extend_from_slice(&["--", path]);
    let out = match rev {
        BlameRev::Index => {
            let Some(blob) = blob_bytes(cwd, "", path) else {
                return Ok(None);
            };
            run_with_stdin(&args, cwd, &blob)
        }
        _ => run(&args, cwd),
    }
    .map_err(|err| err.to_string())?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(parse_blame_porcelain(&String::from_utf8_lossy(&out.stdout)))
}

/// Where the change before a blamed line's commit came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviousLineChange {
    /// Blame of the matching line in the earlier version. A `boundary`
    /// result is a root commit: it has no parent of its own.
    Found(LineBlame),
    /// The blamed commit added the line (or the whole file).
    AddedIn,
}

/// The change to `blamed`'s line before `blamed.sha`.
///
/// Uses porcelain `previous <prev-sha> <prev-path>` (no `previous` means
/// the file was added). Then `diff -U0 -M <prev-sha> <sha> -- <prev-path>
/// <path>` maps the line into the earlier version with
/// [`map_line_to_parent`], and [`blame_line`] blames it there. For a
/// non-merge commit `<prev-sha>` is `<sha>^`. `Err` for an uncommitted
/// line or a failed git call.
pub fn previous_line_change(cwd: &Path, blamed: &LineBlame) -> Result<PreviousLineChange, String> {
    if blamed.uncommitted {
        return Err("line is not committed yet".into());
    }
    let Some((prev_sha, prev_path)) = &blamed.previous else {
        return Ok(PreviousLineChange::AddedIn);
    };
    let paths = compare_revert_paths(&blamed.filename, Some(prev_path.as_str()));
    let mut args = vec!["diff", "-U0", "-M", prev_sha, &blamed.sha, "--"];
    args.extend_from_slice(&paths);
    let diff = exec_git_stdout(&args, cwd)?;
    let Some(mapped) = map_line_to_parent(&diff, blamed.orig_line) else {
        return Ok(PreviousLineChange::AddedIn);
    };
    match blame_line(cwd, &BlameRev::Commit(prev_sha.clone()), prev_path, mapped)? {
        Some(earlier) => Ok(PreviousLineChange::Found(earlier)),
        None => Err(format!("no blame for {prev_path}:{mapped}")),
    }
}

/// Map new-side line `line` of a `diff -U0` onto the old side.
///
/// A line outside every hunk shifts by the line-count change of the
/// hunks above it. A line inside a hunk maps to the old line at the same
/// offset when the hunk's old side is that long. `None` when the line has
/// no old counterpart (pure insertion, or past the end of a grown hunk).
pub fn map_line_to_parent(diff_u0: &str, line: u32) -> Option<u32> {
    let mut delta: i64 = 0;
    for header in diff_u0.lines().filter(|l| l.starts_with("@@")) {
        let mut parts = header.split_whitespace().skip(1);
        let (old_start, old_count) = hunk_range(parts.next()?.strip_prefix('-')?)?;
        let (new_start, new_count) = hunk_range(parts.next()?.strip_prefix('+')?)?;
        // A pure deletion (`+n,0`) sits after new line `n`.
        let below = if new_count == 0 {
            line > new_start
        } else {
            line >= new_start + new_count
        };
        if below {
            delta += i64::from(old_count) - i64::from(new_count);
            continue;
        }
        if line < new_start || new_count == 0 {
            break;
        }
        let offset = line - new_start;
        return (offset < old_count).then_some(old_start + offset);
    }
    u32::try_from(i64::from(line) + delta).ok()
}

/// `n` or `n,count` from a hunk header side. A bare `n` counts one line.
fn hunk_range(token: &str) -> Option<(u32, u32)> {
    match token.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((token.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests;
