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
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

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
pub(crate) fn git_process(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Build a git subprocess that cannot steal the TUI's TTY.
///
/// Stdin is `/dev/null` so a credential prompt cannot deadlock against the
/// event loop. `GIT_TERMINAL_PROMPT=0` fails fast instead of waiting on a
/// hidden prompt. Run the result with [`git_output`] so both deadlines
/// apply.
fn git_command(bin: &Path, args: &[&str], cwd: &Path) -> Command {
    let mut cmd = git_process(bin);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// The git subcommand in `args`: the first argument that is not a global
/// option (`-c k=v`, `-C dir`, `--no-pager`, …) or an option's value.
fn git_subcommand<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if GLOBAL_OPTIONS_WITH_VALUE.contains(arg) {
            rest.next();
        } else if !arg.starts_with('-') {
            return Some(arg);
        }
    }
    None
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
/// The handler sends `SIGTERM` to every registered git process group, then
/// restores the default action and raises the signal again, so `ws` exits
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
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// The exit-signal handler: async-signal-safe calls only (atomic loads,
/// `kill`, `signal`, `raise`).
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
/// that puts the user's work back (the auto-stash pop after a pull): a quit
/// during the pull must not leave their changes in a stash entry. The time
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
pub fn repo_has_local_changes(cwd: &Path) -> bool {
    exec_git_status(&["diff", "--quiet"], cwd) != 0
        || exec_git_status(&["diff", "--cached", "--quiet"], cwd) != 0
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
    /// Tracked local changes were stashed before the pull.
    pub stashed: bool,
    /// The pull ran, but popping the auto-stash conflicted. The stash is kept.
    pub stash_pop_failed: bool,
    /// Git's reason line when the auto-stash push or the pull failed.
    pub error: Option<String>,
}

/// `git pull --quiet`, stashing tracked local changes first when needed.
pub fn pull_quiet_detailed(cwd: &Path) -> PullQuietResult {
    let dirty = repo_has_local_changes(cwd);
    let mut stashed = false;
    if dirty {
        let args = ["stash", "push", "-m", AUTO_STASH_MESSAGE, "--quiet"];
        if let Err(err) = exec_git_checked(&args, cwd) {
            return PullQuietResult {
                ok: false,
                stashed: false,
                stash_pop_failed: false,
                error: Some(format!("auto-stash failed: {err}")),
            };
        }
        stashed = true;
    }

    let pulled = exec_git_checked(&["pull", "--quiet"], cwd);
    // The pop runs even when a quit stopped the pull: it gives the user's
    // changes back.
    let stash_pop_failed = stashed
        && !matches!(
            run_restoring_user_work(&["stash", "pop", "--quiet"], cwd),
            Ok(out) if out.status.success()
        );
    PullQuietResult {
        ok: pulled.is_ok() && !stash_pop_failed,
        stashed,
        stash_pop_failed,
        error: pulled.err(),
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

/// Default branch name for merge-into-default classification: the known
/// default ([`known_default_branch_name`]), else `main`.
pub fn resolve_default_branch_name(cwd: &Path, override_name: Option<&str>) -> String {
    known_default_branch_name(cwd, override_name).unwrap_or_else(|| "main".to_string())
}

/// The repository's default branch when it is known: the non-empty
/// configured override, else the branch `origin/HEAD` points at. `None`
/// when neither names one; no branch name is guessed.
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

/// Stage one path (`git add --`).
pub fn stage_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["add", "--", file_path], cwd)
}

/// Unstage one path (`git restore --staged --`).
pub fn unstage_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["restore", "--staged", "--", file_path], cwd)
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

/// Discard worktree changes to a tracked path (`git restore --`).
pub fn revert_tracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["restore", "--", file_path], cwd)
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
/// watch tick old.
pub fn ensure_compare_head_clean(cwd: &Path, head: &str, paths: &[&str]) -> Result<(), String> {
    match rev_parse_commit(cwd, "HEAD")? {
        Some(now) if now == head => {}
        _ => return Err(format!("{COMPARE_REVERT_ABORTED}HEAD moved")),
    }
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--ignored",
        "--",
    ];
    args.extend_from_slice(paths);
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
/// (`git restore --source=<rev> --worktree --`).
///
/// The index stays untouched. A tracked path that `rev` does not have is
/// removed from the worktree; a path only `rev` has is written.
pub fn restore_worktree_from(cwd: &Path, rev: &str, paths: &[&str]) -> Result<(), String> {
    let source = format!("--source={rev}");
    let mut args = vec!["restore", source.as_str(), "--worktree", "--"];
    args.extend_from_slice(paths);
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

/// Delete an untracked path (`git clean -f --`). Destructive.
pub fn remove_untracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["clean", "-f", "--", file_path], cwd)
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
pub fn stash_push(cwd: &Path, paths: &[String]) -> Result<(), String> {
    let before = exec_git(&["stash", "list"], cwd);
    let mut args: Vec<&str> = vec!["stash", "push", "-u"];
    if !paths.is_empty() {
        args.push("--");
        for path in paths {
            args.push(path);
        }
    }
    exec_git_checked(&args, cwd)?;
    let after = exec_git(&["stash", "list"], cwd);
    if after == before {
        return Err("no local changes to save".into());
    }
    Ok(())
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

/// Parse newline `name-status` (`M\\tpath` or `R100\\told\\tnew`).
pub fn parse_name_status_lines(stdout: &str) -> Vec<NameStatus> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("").to_string();
        if status.is_empty() {
            continue;
        }
        if status.starts_with('R') || status.starts_with('C') {
            let old_path = parts.next().map(str::to_string).filter(|s| !s.is_empty());
            let Some(path) = parts.next().map(str::to_string).filter(|s| !s.is_empty()) else {
                continue;
            };
            out.push(NameStatus {
                status: status.chars().next().unwrap_or('M').to_string(),
                path,
                old_path,
                stat: None,
            });
            continue;
        }
        let Some(path) = parts.next().map(str::to_string).filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(NameStatus {
            status: status.chars().next().unwrap_or('M').to_string(),
            path,
            old_path: None,
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
    let out = exec_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            &parent,
            commit_id,
        ],
        cwd,
    );
    if !out.is_empty() {
        let mut files = parse_name_status_lines(&out);
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
    let root = exec_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            "--root",
            commit_id,
        ],
        cwd,
    );
    let mut files = parse_name_status_lines(&root);
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
    let mut files = parse_name_status_lines(&exec_git(
        &["stash", "show", "--name-status", stash_ref],
        cwd,
    ));
    let numstat = exec_git(&["stash", "show", "--numstat", "-z", stash_ref], cwd);
    attach_numstat(&mut files, &numstat);
    files
}

/// Worktree + index changes versus HEAD, plus untracked files.
///
/// Tracked files carry line counts from `diff HEAD --numstat -z`. Untracked
/// files have `stat: None`.
pub fn list_worktree_name_status(cwd: &Path) -> Vec<NameStatus> {
    let mut files = parse_name_status_lines(&exec_git(&["diff", "HEAD", "--name-status"], cwd));
    let numstat = exec_git(&["diff", "HEAD", "--numstat", "-z"], cwd);
    attach_numstat(&mut files, &numstat);
    let untracked = exec_git(&["ls-files", "--others", "--exclude-standard"], cwd);
    for path in untracked.lines().map(str::trim).filter(|l| !l.is_empty()) {
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
    args.push(path.into());
    args
}

fn exec_git_owned(args: &[String], cwd: &Path) -> String {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    exec_git(&refs, cwd)
}

/// Git's reason line ([`git_reason_line`]), or `git <sub> exited with code N`.
fn git_failure_message(args: &[&str], out: &std::process::Output) -> String {
    git_reason_line(out).unwrap_or_else(|| exit_code_message(args, out))
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
    let stdout = exec_git_stdout(
        &["diff", "--name-status", "--find-renames", &range, "--"],
        cwd,
    )?;
    let mut files = parse_name_status_lines(&stdout);
    let numstat = exec_git_stdout(
        &["diff", "--numstat", "-z", "--find-renames", &range, "--"],
        cwd,
    )
    .unwrap_or_default();
    attach_numstat(&mut files, &numstat);
    Ok(files)
}

/// Patch key per path of `base...HEAD`, from `diff --raw --no-abbrev
/// --find-renames`.
///
/// The key holds the old and new modes, the old and new blob ids, the
/// status letter and the path(s). Two keys are equal only when the patch
/// for that path is the same, whatever commits produced it. Keyed by the
/// new path, spelled as [`list_compare_name_status`] lists it: no `-z`, so
/// git quotes unusual paths the same way in both. A failed git call gives
/// an empty map; the file list does not depend on it.
pub fn list_compare_patch_keys(
    cwd: &Path,
    base_sha: &str,
    head_sha: &str,
) -> std::collections::HashMap<String, String> {
    let range = format!("{base_sha}...{head_sha}");
    let raw = exec_git_stdout(
        &[
            "diff",
            "--raw",
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

/// Parse newline `git diff --raw` output into new path → patch key.
///
/// A row is `:<old mode> <new mode> <old id> <new id> <status>[score]\t<path>`.
/// A rename or copy row ends `\t<old path>\t<new path>`. Paths stay as git
/// prints them (quoted when unusual), like [`parse_name_status_lines`]. The
/// similarity score is left out of the key: the blob ids already fix it.
fn parse_raw_patch_keys(stdout: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for line in stdout.lines() {
        let Some(line) = line.strip_prefix(':') else {
            continue;
        };
        let mut parts = line.split('\t');
        let meta = parts.next().unwrap_or("");
        let fields: Vec<&str> = meta.split(' ').collect();
        let [old_mode, new_mode, old_id, new_id, status] = fields[..] else {
            continue;
        };
        let Some(letter) = status.chars().next() else {
            continue;
        };
        let Some(first) = parts.next().filter(|p| !p.is_empty()) else {
            continue;
        };
        let (old_path, path) = if matches!(letter, 'R' | 'C') {
            let Some(new) = parts.next().filter(|p| !p.is_empty()) else {
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
    let stdout = exec_git_stdout(&refs, cwd)?;
    if stdout.trim().is_empty() {
        Ok(vec!["(no diff)".into()])
    } else {
        Ok(stdout.lines().map(str::to_string).collect())
    }
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
/// Runs `diff --name-status -M <base> -- [<old>] <path>`, then the same with
/// `--numstat -z` for line counts (its failure leaves `stat` as `None`).
/// Empty stdout (the worktree file equals the commit) is an empty list.
pub fn list_worktree_vs_commit_name_status(
    cwd: &Path,
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<Vec<NameStatus>, String> {
    let paths = compare_revert_paths(path, old_path);
    let mut args = vec!["diff", "--name-status", "-M", base_sha, "--"];
    args.extend_from_slice(&paths);
    let mut files = parse_name_status_lines(&exec_git_stdout(&args, cwd)?);
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
        .filter_map(|line| {
            let (id, subject) = line.split_once('\t').unwrap_or((line, ""));
            (!id.is_empty()).then(|| AncestorCommit {
                id: id.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect())
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
    /// `author` header. For an uncommitted line it is a git placeholder;
    /// use [`LineBlame::uncommitted`], not this name, to detect that case.
    pub author: String,
    /// `author-time`, unix seconds.
    pub author_time: i64,
    /// `summary` header: the commit subject.
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
            "author" => blame.author = value.to_string(),
            "author-time" => blame.author_time = value.parse().unwrap_or(0),
            "summary" => blame.summary = value.to_string(),
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
/// --porcelain -L n,n [<rev>] -- <path>`. Paths in the result are unquoted.
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
mod tests {
    use super::*;
    use crate::testutil::{
        git, init_repo, init_repo_empty, unique_dir, upstream_rewrite_fixture, RewriteFixture,
        RewriteKind,
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
        let dir = std::env::temp_dir().join(format!(
            "ws-git-stdin-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
        let give_up = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while stash_count() == 0 {
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
        let normal = git_diff_args(&["diff"], "README.md", None);
        assert_eq!(normal, vec!["diff", "--", "README.md"]);
        let full = git_diff_args(&["diff"], "README.md", Some(FULL_DIFF_CONTEXT_LINES));
        assert_eq!(
            full,
            vec![
                "diff".to_string(),
                format!("-U{FULL_DIFF_CONTEXT_LINES}"),
                "--".into(),
                "README.md".into(),
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

    #[test]
    fn apply_cached_patch_stages_and_unstages_one_hunk() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-apply-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
        let dir = std::env::temp_dir().join(format!(
            "ws-git-merge-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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

    #[test]
    fn parse_name_status_and_commit_files_fixture() {
        assert_eq!(
            parse_name_status_lines("M\tsrc/a.rs\nR100\told.rs\tnew.rs\n"),
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
        assert!(
            files.iter().any(|f| f.path.starts_with('"')),
            "git quotes an unusual path: {files:?}"
        );
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
            ":100644 100644 {o} {z} M\tsrc/a.rs\n:100644 100644 {o} {z} R087\told.rs\tnew.rs\n"
        );
        let keys = parse_raw_patch_keys(&raw);
        assert_eq!(keys.len(), 2);
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
        let dir = std::env::temp_dir().join(format!(
            "ws-git-blob-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
        let dir = std::env::temp_dir().join(format!(
            "ws-git-blob-miss-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        assert_eq!(blob_bytes(&dir, "HEAD", "nope.txt"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compare_wrappers_ahead_behind_diverged_rename_and_failure() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-compare-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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
            renamed.iter().any(|row| {
                row.path == "renamed.txt" && row.old_path.as_deref() == Some("one.txt")
            }),
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
        let root = unique_dir("ws-git-pull-conflict");
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
        fs::write(up.join("README.md"), "# upstream\n").unwrap();
        git(&up, &["commit", "-qam", "upstream"]);
        fs::write(down.join("README.md"), "# local edit\n").unwrap();

        let result = pull_quiet_detailed(&down);
        assert!(result.stashed && result.stash_pop_failed && !result.ok);
        assert_eq!(result.error, None, "the pull itself worked");
        assert_eq!(list_stash_refs(&down).len(), 1, "stash entry is kept");
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
            diff_worktree_vs_commit_file_ctx(&dir, &base, "new.txt", Some("old.txt"), Some(0))
                .unwrap();
        assert!(diff.iter().any(|l| l == "rename from old.txt"), "{diff:?}");
        assert!(diff.iter().any(|l| l == "+four"), "{diff:?}");

        let added = list_worktree_vs_commit_name_status(&dir, &base, "f.txt", None).unwrap();
        assert_eq!(added[0].status, "A", "f.txt is newer than base: {added:?}");
        assert!(
            diff_worktree_vs_commit_file_ctx(&dir, "no-such-ref", "f.txt", None, None).is_err()
        );
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
            err.starts_with("main is checked out in another worktree: ")
                && err.ends_with("checkout"),
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
}
