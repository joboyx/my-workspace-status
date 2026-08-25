//! Bounded parallel map for independent per-repo git work.
//!
//! Ink used `mapWithConcurrency` with `FETCH_CONCURRENCY = 4`. Fetch, pull,
//! push, and snapshot collect ([`crate::discovery::process_repo`]) share that
//! cap. Writes that must stay exclusive on one checkout (stage, commit, merge
//! into HEAD) stay serial on the event loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::vec::IntoIter;

/// Default in-flight cap for independent per-repo git (Ink `FETCH_CONCURRENCY`).
pub const FETCH_CONCURRENCY: usize = 4;

/// Cap from `WS_STATUS_FETCH_CONCURRENCY`. Missing / invalid / `0` → [`FETCH_CONCURRENCY`].
/// Values below 1 clamp to 1.
pub fn fetch_concurrency(raw: Option<&str>) -> usize {
    let Some(raw) = raw else {
        return FETCH_CONCURRENCY;
    };
    if raw.is_empty() {
        return FETCH_CONCURRENCY;
    }
    let Ok(parsed) = raw.parse::<i64>() else {
        return FETCH_CONCURRENCY;
    };
    if parsed <= 0 {
        return FETCH_CONCURRENCY;
    }
    (parsed as usize).max(1)
}

/// Cap from the process environment (`WS_STATUS_FETCH_CONCURRENCY`).
pub fn env_fetch_concurrency() -> usize {
    fetch_concurrency(std::env::var("WS_STATUS_FETCH_CONCURRENCY").ok().as_deref())
}

/// Run `f` on each item with at most `cap` worker threads.
///
/// Output order matches input. Panics if a worker panics before sending
/// its result (the slot would be missing).
pub fn map_with_concurrency<T, U, F>(items: Vec<T>, cap: usize, f: F) -> Vec<U>
where
    T: Send + 'static,
    U: Send + 'static,
    F: Fn(T) -> U + Send + Sync + 'static,
{
    CappedBatch::start(items, cap, f)
        .wait_all()
        .into_iter()
        .map(|slot| slot.expect("capped worker panicked before sending a result"))
        .collect()
}

/// Streaming capped map so the TUI can paint `Fetching n/N` as completions land.
pub struct CappedBatch<U> {
    rx: Receiver<(usize, U)>,
    cancel: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
    slots: Vec<Option<U>>,
    done: usize,
    finished: bool,
}

impl<U: Send + 'static> CappedBatch<U> {
    /// Spawn up to `cap` workers that pull from `items`.
    pub fn start<T, F>(items: Vec<T>, cap: usize, f: F) -> Self
    where
        T: Send + 'static,
        F: Fn(T) -> U + Send + Sync + 'static,
    {
        let total = items.len();
        let slots: Vec<Option<U>> = (0..total).map(|_| None).collect();
        if total == 0 {
            let (tx, rx) = mpsc::channel();
            drop(tx);
            return Self {
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                handles: Vec::new(),
                slots,
                done: 0,
                finished: true,
            };
        }
        let cap = cap.max(1).min(total);
        let (tx, rx) = mpsc::channel();
        let queued: Vec<(usize, T)> = items.into_iter().enumerate().collect();
        let queue = Arc::new(Mutex::new(queued.into_iter()));
        let f = Arc::new(f);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::with_capacity(cap);
        for _ in 0..cap {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let f = Arc::clone(&f);
            let cancel = Arc::clone(&cancel);
            handles.push(thread::spawn(move || {
                worker_loop(queue.as_ref(), &tx, f.as_ref(), &cancel);
            }));
        }
        drop(tx);
        Self {
            rx,
            cancel,
            handles,
            slots,
            done: 0,
            finished: false,
        }
    }

    /// Take one completion. Returns the new completed count (`1..=N`).
    ///
    /// Counts **finishes**, not starts. `None` means no completion is ready
    /// (or the batch has finished).
    pub fn try_recv(&mut self) -> Option<usize> {
        match self.rx.try_recv() {
            Ok((idx, value)) => {
                self.slots[idx] = Some(value);
                self.done += 1;
                Some(self.done)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.finished = true;
                None
            }
        }
    }

    /// True when every worker has exited (in-flight work included).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Stop taking new items. In-flight `f` calls still run to completion.
    ///
    /// Workers that dequeue after this drop the item without calling `f`.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Block until every worker has sent or exited.
    pub fn wait_all(mut self) -> Vec<Option<U>> {
        loop {
            match self.rx.recv() {
                Ok((idx, value)) => {
                    self.slots[idx] = Some(value);
                    self.done += 1;
                }
                Err(_) => {
                    self.finished = true;
                    break;
                }
            }
        }
        self.join()
    }

    /// Join workers and return per-index results (`None` if skipped or panicked).
    pub fn join(self) -> Vec<Option<U>> {
        for handle in self.handles {
            let _ = handle.join();
        }
        self.slots
    }
}

fn worker_loop<T, U, F>(
    queue: &Mutex<IntoIter<(usize, T)>>,
    tx: &mpsc::Sender<(usize, U)>,
    f: &F,
    cancel: &AtomicBool,
) where
    F: Fn(T) -> U,
{
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let next = queue.lock().expect("capped queue").next();
        let Some((idx, item)) = next else {
            break;
        };
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let _ = tx.send((idx, f(item)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspaceStatusConfig;
    use crate::discovery::collect_snapshots;
    use crate::git::{exec_git_checked, git_binary, pull_quiet_detailed, push_quiet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    const SLOW_MS: u64 = 250;

    fn git_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("GIT_AUTHOR_NAME", "workspace-status test"),
            ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
            ("GIT_COMMITTER_NAME", "workspace-status test"),
            (
                "GIT_COMMITTER_EMAIL",
                "workspace-status-test@example.invalid",
            ),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]
    }

    fn git(cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new(git_binary());
        cmd.args(args).current_dir(cwd);
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        let status = cmd.status().expect("git");
        assert!(status.success(), "git {args:?}");
    }

    fn unique_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ws-parallel-{tag}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_exec(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    struct SlowWorkspace {
        root: PathBuf,
        workspace: PathBuf,
        repos: Vec<PathBuf>,
    }

    impl SlowWorkspace {
        fn new(n: usize) -> Self {
            let root = unique_root("git");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).unwrap();
            let git_bin = git_binary().display().to_string();
            let upload = root.join("slow-upload-pack");
            let receive = root.join("slow-receive-pack");
            write_exec(
                &upload,
                &format!(
                    "#!/bin/sh\nsleep {sleep}\nexec '{git}' upload-pack \"$@\"\n",
                    sleep = SLOW_MS as f64 / 1000.0,
                    git = git_bin.replace('\'', "'\\''"),
                ),
            );
            write_exec(
                &receive,
                &format!(
                    "#!/bin/sh\nsleep {sleep}\nexec '{git}' receive-pack \"$@\"\n",
                    sleep = SLOW_MS as f64 / 1000.0,
                    git = git_bin.replace('\'', "'\\''"),
                ),
            );
            let mut repos = Vec::new();
            for i in 0..n {
                let name = format!("r{i}");
                let remote = root.join(format!("{name}.git"));
                let repo = workspace.join(&name);
                let _ = Command::new(git_binary())
                    .args(["init", "-q", "--bare", remote.to_str().unwrap()])
                    .envs(git_env())
                    .status()
                    .unwrap();
                fs::create_dir_all(&repo).unwrap();
                let init = Command::new(git_binary())
                    .args(["init", "-q", "-b", "main"])
                    .current_dir(&repo)
                    .envs(git_env())
                    .status();
                if init.map(|s| s.success()).unwrap_or(false) == false {
                    git(&repo, &["init", "-q"]);
                    git(&repo, &["checkout", "-q", "-b", "main"]);
                }
                git(&repo, &["config", "user.name", "workspace-status test"]);
                git(
                    &repo,
                    &[
                        "config",
                        "user.email",
                        "workspace-status-test@example.invalid",
                    ],
                );
                fs::write(repo.join("README.md"), format!("# {name}\n")).unwrap();
                git(&repo, &["add", "README.md"]);
                git(&repo, &["commit", "-q", "-m", "seed"]);
                git(
                    &repo,
                    &["remote", "add", "origin", remote.to_str().unwrap()],
                );
                git(&repo, &["push", "-u", "origin", "main", "--quiet"]);
                git(
                    &repo,
                    &[
                        "config",
                        "remote.origin.uploadpack",
                        upload.to_str().unwrap(),
                    ],
                );
                git(
                    &repo,
                    &[
                        "config",
                        "remote.origin.receivepack",
                        receive.to_str().unwrap(),
                    ],
                );
                repos.push(repo);
            }
            Self {
                root,
                workspace,
                repos,
            }
        }
    }

    impl Drop for SlowWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn assert_overlap(elapsed: Duration, max_inflight: usize, label: &str) {
        assert!(
            max_inflight >= 3,
            "{label}: expected overlapping in-flight git, max was {max_inflight}"
        );
        let serial_floor = Duration::from_millis(SLOW_MS * 4);
        assert!(
            elapsed < serial_floor.saturating_sub(Duration::from_millis(80)),
            "{label}: {elapsed:?} looks serial (4 × {SLOW_MS}ms is ~{serial_floor:?})"
        );
    }

    fn with_inflight<T>(
        dirs: Vec<PathBuf>,
        cap: usize,
        work: impl Fn(&Path) -> T + Send + Sync + 'static,
    ) -> (Duration, usize, Vec<T>)
    where
        T: Send + 'static,
    {
        let inflight = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        let results = map_with_concurrency(dirs, cap, {
            let inflight = Arc::clone(&inflight);
            let max = Arc::clone(&max);
            move |dir| {
                let n = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(n, Ordering::SeqCst);
                let out = work(&dir);
                inflight.fetch_sub(1, Ordering::SeqCst);
                out
            }
        });
        (start.elapsed(), max.load(Ordering::SeqCst), results)
    }

    #[test]
    fn default_cap_is_ink_four() {
        assert_eq!(FETCH_CONCURRENCY, 4);
        assert_eq!(fetch_concurrency(None), 4);
        assert_eq!(fetch_concurrency(Some("")), 4);
        assert_eq!(fetch_concurrency(Some("nope")), 4);
        assert_eq!(fetch_concurrency(Some("0")), 4);
        assert_eq!(fetch_concurrency(Some("-1")), 4);
        assert_eq!(fetch_concurrency(Some("8")), 8);
        assert_eq!(fetch_concurrency(Some("1")), 1);
    }

    #[test]
    fn map_preserves_order() {
        let out = map_with_concurrency(vec![1, 2, 3, 4, 5], 2, |n| n * 10);
        assert_eq!(out, vec![10, 20, 30, 40, 50]);
    }

    #[test]
    fn cap_one_stays_serial() {
        let inflight = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        map_with_concurrency(vec![(); 3], 1, {
            let inflight = Arc::clone(&inflight);
            let max = Arc::clone(&max);
            move |_| {
                let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(cur, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(20));
                inflight.fetch_sub(1, Ordering::SeqCst);
            }
        });
        assert_eq!(max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_skips_queued_jobs() {
        let ran = Arc::new(AtomicUsize::new(0));
        let batch = CappedBatch::start(vec![(); 8], 2, {
            let ran = Arc::clone(&ran);
            move |_| {
                ran.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(40));
            }
        });
        thread::sleep(Duration::from_millis(10));
        batch.cancel();
        let slots = batch.wait_all();
        let started = ran.load(Ordering::SeqCst);
        assert!(started <= 4, "cancel must drop queued work, ran {started}");
        assert!(
            slots.iter().filter(|s| s.is_none()).count() >= 4,
            "skipped jobs leave None slots, got {slots:?}"
        );
    }

    #[test]
    fn sleep_jobs_overlap_under_cap() {
        let inflight = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        let out = map_with_concurrency((0..6).collect::<Vec<_>>(), 4, {
            let inflight = Arc::clone(&inflight);
            let max = Arc::clone(&max);
            move |n| {
                let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(cur, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(60));
                inflight.fetch_sub(1, Ordering::SeqCst);
                n
            }
        });
        let elapsed = start.elapsed();
        assert_eq!(out, vec![0, 1, 2, 3, 4, 5]);
        assert!(
            max.load(Ordering::SeqCst) >= 3,
            "cap 4 should overlap, max in-flight {}",
            max.load(Ordering::SeqCst)
        );
        assert!(
            max.load(Ordering::SeqCst) <= 4,
            "must not exceed cap, max in-flight {}",
            max.load(Ordering::SeqCst)
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "6 × 60ms with cap 4 should be two waves, took {elapsed:?}"
        );
    }

    #[test]
    fn progress_counts_completions_not_starts() {
        let mut batch = CappedBatch::start(vec![(); 4], 4, |_| {
            thread::sleep(Duration::from_millis(80));
        });
        thread::sleep(Duration::from_millis(25));
        assert!(
            batch.try_recv().is_none(),
            "nothing should complete just because 4 workers started"
        );
        assert!(!batch.is_finished());
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(800);
        while !batch.is_finished() {
            if let Some(done) = batch.try_recv() {
                seen.push(done);
            }
            if Instant::now() > deadline {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        while let Some(done) = batch.try_recv() {
            seen.push(done);
        }
        let _ = batch.join();
        assert_eq!(seen, vec![1, 2, 3, 4]);
    }

    #[test]
    fn fetch_pull_push_of_several_repos_overlap() {
        let fixture = SlowWorkspace::new(4);
        let dirs = fixture.repos.clone();

        let (elapsed, max, results) = with_inflight(dirs.clone(), 4, |dir| {
            exec_git_checked(&["fetch", "--quiet"], dir)
        });
        assert!(results.iter().all(Result::is_ok), "fetch: {results:?}");
        assert_overlap(elapsed, max, "fetch");

        let (elapsed, max, results) =
            with_inflight(dirs.clone(), 4, |dir| pull_quiet_detailed(dir));
        assert!(results.iter().all(|r| r.ok), "pull: {results:?}");
        assert_overlap(elapsed, max, "pull");

        for (i, dir) in dirs.iter().enumerate() {
            fs::write(dir.join("README.md"), format!("# push {i}\n")).unwrap();
            git(dir, &["add", "README.md"]);
            git(dir, &["commit", "-q", "-m", "ahead"]);
        }
        let (elapsed, max, results) = with_inflight(dirs, 4, |dir| push_quiet(dir));
        assert!(results.iter().all(Result::is_ok), "push: {results:?}");
        assert_overlap(elapsed, max, "push");
    }

    #[test]
    fn collect_snapshots_fetch_overlaps() {
        let fixture = SlowWorkspace::new(4);
        let start = Instant::now();
        let snaps = collect_snapshots(
            &fixture.workspace,
            true,
            &WorkspaceStatusConfig::with_defaults(),
            None,
        );
        let elapsed = start.elapsed();
        assert_eq!(snaps.len(), 4);
        assert_eq!(
            snaps.iter().map(|s| s.repo.as_str()).collect::<Vec<_>>(),
            vec!["r0", "r1", "r2", "r3"]
        );
        let serial_floor = Duration::from_millis(SLOW_MS * 4);
        assert!(
            elapsed < serial_floor.saturating_sub(Duration::from_millis(80)),
            "collect_snapshots fetch took {elapsed:?}; serial 4 × {SLOW_MS}ms is ~{serial_floor:?}"
        );
    }
}
