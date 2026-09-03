//! Effect scheduler for the live TTY loop and Headless e2e.
//!
//! Turns [`Effect`](super::action::Effect) into capped jobs. The live loop
//! `spawn_blocking`s them onto a `JoinSet`. Headless runs the same jobs on
//! the test thread. [`Scheduler`] decides which [`TaskResult`] values may
//! touch [`super::state::AppState`]. [`super::effect::Interpreter`] applies
//! those results. Workers never draw.

use std::collections::{HashMap, HashSet, VecDeque};

/// Why a workspace collect was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind {
    /// Live watch poll.
    Watch,
    /// `r` on the workspace / No-updates row, or a post-write refresh.
    Reload,
}

/// User / pane work vs background `git status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobClass {
    /// Pane load, writes, pickers, fetch/pull/push, focused status.
    User,
    /// Background checkout status for a collect.
    Status,
}

/// Work the loop may spawn on the blocking pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    /// Walk + `git worktree list` only.
    Discover { gen: u64, kind: CollectionKind },
    /// One [`crate::discovery::process_repo`].
    ProcessRepo {
        gen: u64,
        path: String,
        focused: bool,
    },
    /// Right-pane `git log` / diff.
    LoadPane { req_id: u64 },
    /// Slot reserved for a user git op (write, remote, picker, autoload).
    UserWork { tag: UserTag },
}

/// User-priority git that is not a collect `process_repo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserTag {
    Pane,
    Write,
    BulkRemote,
    DefaultBranch,
    Prepare,
    /// External-diff blob + temp prepare (`E`).
    DiffPrepare,
    Autoload,
}

/// A job the loop should `spawn_blocking`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnRequest {
    pub id: u64,
    pub class: JobClass,
    pub kind: SpawnKind,
}

/// Typed worker result. Stale generations are discarded by [`Scheduler`].
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TaskResult {
    Discovered {
        gen: u64,
        paths: Vec<String>,
    },
    RepoStatus {
        gen: u64,
        path: String,
        applied: bool,
    },
    Pane {
        req_id: u64,
        accepted: bool,
    },
    UserDone {
        tag: UserTag,
    },
}

/// What the loop should do after a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyDecision {
    Ignore,
    ApplyRepo {
        path: String,
        load_pane_if_focused: bool,
    },
    StartDiscover {
        gen: u64,
        kind: CollectionKind,
    },
    QueueRepos,
    CollectionIdle,
    #[allow(dead_code)]
    ApplyPane,
}

/// Caps, generations, and queues for the live loop.
pub struct Scheduler {
    cap: usize,
    next_job_id: u64,
    next_collection_gen: u64,
    next_write_gen: u64,
    next_pane_id: u64,
    collection: Option<CollectionState>,
    latched_watch: bool,
    user_queue: VecDeque<SpawnKind>,
    status_queue: VecDeque<SpawnKind>,
    inflight: HashMap<u64, Inflight>,
    pane_latest: u64,
    pane_inflight: Option<u64>,
    pane_pending: bool,
    next_autoload_id: u64,
    autoload_latest: u64,
    next_commit_files_id: u64,
    commit_files_latest: u64,
    next_commit_diff_id: u64,
    commit_diff_latest: u64,
    next_prepare_stash_id: u64,
    prepare_stash_latest: u64,
    next_prepare_branches_id: u64,
    prepare_branches_latest: u64,
    exclusive_write: bool,
    default_branch_busy: bool,
    /// Highest status generation accepted per checkout path.
    path_gen: HashMap<String, u64>,
}

struct CollectionState {
    gen: u64,
    kind: CollectionKind,
    pending: HashSet<String>,
    discovered: bool,
    focused: Option<String>,
}

struct Inflight {
    #[allow(dead_code)]
    class: JobClass,
    kind: SpawnKind,
}

impl Scheduler {
    /// `cap` is [`crate::parallel::env_fetch_concurrency`] (default 10).
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            next_job_id: 1,
            next_collection_gen: 1,
            next_write_gen: 1,
            next_pane_id: 1,
            collection: None,
            latched_watch: false,
            user_queue: VecDeque::new(),
            status_queue: VecDeque::new(),
            inflight: HashMap::new(),
            pane_latest: 0,
            pane_inflight: None,
            pane_pending: false,
            next_autoload_id: 1,
            autoload_latest: 0,
            next_commit_files_id: 1,
            commit_files_latest: 0,
            next_commit_diff_id: 1,
            commit_diff_latest: 0,
            next_prepare_stash_id: 1,
            prepare_stash_latest: 0,
            next_prepare_branches_id: 1,
            prepare_branches_latest: 0,
            exclusive_write: false,
            default_branch_busy: false,
            path_gen: HashMap::new(),
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn inflight_count(&self) -> usize {
        self.inflight.len()
    }

    pub fn queued_user(&self) -> usize {
        self.user_queue.len()
    }

    pub fn queued_status(&self) -> usize {
        self.status_queue.len()
    }

    pub fn collection_gen(&self) -> Option<u64> {
        self.collection.as_ref().map(|c| c.gen)
    }

    pub fn collection_active(&self) -> bool {
        self.collection.is_some()
    }

    pub fn latched_watch(&self) -> bool {
        self.latched_watch
    }

    pub fn busy_for_writes(&self) -> bool {
        self.exclusive_write
            || self.inflight.values().any(|job| {
                matches!(
                    job.kind,
                    SpawnKind::UserWork {
                        tag: UserTag::Write | UserTag::BulkRemote | UserTag::DefaultBranch
                    }
                )
            })
            || self.user_queue.iter().any(|kind| {
                matches!(
                    kind,
                    SpawnKind::UserWork {
                        tag: UserTag::Write | UserTag::BulkRemote | UserTag::DefaultBranch
                    }
                )
            })
    }

    /// Watch tick. Starts a collect, or latches one rerun if one is running.
    pub fn on_watch_tick(&mut self, focused: Option<String>) -> bool {
        if self.collection.is_some() {
            self.latched_watch = true;
            return false;
        }
        self.start_collection(CollectionKind::Watch, focused);
        true
    }

    /// Full workspace reload (`r` on workspace / after a write).
    pub fn on_reload_snapshot(&mut self, focused: Option<String>) -> bool {
        if self.collection.is_some() {
            self.latched_watch = true;
            if let Some(col) = self.collection.as_mut() {
                col.kind = CollectionKind::Reload;
            }
            return false;
        }
        self.start_collection(CollectionKind::Reload, focused);
        true
    }

    fn start_collection(&mut self, kind: CollectionKind, focused: Option<String>) {
        let gen = self.next_collection_gen;
        self.next_collection_gen += 1;
        self.latched_watch = false;
        self.collection = Some(CollectionState {
            gen,
            kind,
            pending: HashSet::new(),
            discovered: false,
            focused,
        });
        self.user_queue.push_back(SpawnKind::Discover { gen, kind });
    }

    /// Single-checkout `r`. Bumps that path's expected gen via a new collect token.
    pub fn on_reload_repo(&mut self, path: String) {
        let gen = self.next_collection_gen;
        self.next_collection_gen += 1;
        self.path_gen.insert(path.clone(), gen);
        self.user_queue.push_front(SpawnKind::ProcessRepo {
            gen,
            path,
            focused: true,
        });
    }

    /// Discover finished. Queue focused checkout first, then the rest.
    pub fn on_discovered(&mut self, gen: u64, mut paths: Vec<String>) -> ApplyDecision {
        let Some(col) = self.collection.as_mut() else {
            return ApplyDecision::Ignore;
        };
        if col.gen != gen {
            return ApplyDecision::Ignore;
        }
        col.discovered = true;
        let focused = col.focused.clone();
        if let Some(focus) = focused.as_ref() {
            if let Some(idx) = paths.iter().position(|p| p == focus) {
                let hit = paths.remove(idx);
                paths.insert(0, hit);
            }
        }
        let mut queued = Vec::new();
        for path in paths {
            let existing = self.path_gen.get(&path).copied().unwrap_or(0);
            if existing > gen {
                continue;
            }
            self.path_gen.insert(path.clone(), gen);
            queued.push(path);
        }
        col.pending = queued.iter().cloned().collect();
        for (i, path) in queued.into_iter().enumerate() {
            let focused_job = focused.as_deref() == Some(path.as_str());
            let kind = SpawnKind::ProcessRepo {
                gen,
                path,
                focused: focused_job,
            };
            if i == 0 && focused_job {
                self.user_queue.push_front(kind);
            } else if focused_job {
                self.user_queue.push_back(kind);
            } else {
                self.status_queue.push_back(kind);
            }
        }
        ApplyDecision::QueueRepos
    }

    /// True when this status result may replace that checkout.
    pub fn accept_repo_result(&self, gen: u64, path: &str) -> bool {
        if self.path_gen.get(path) == Some(&gen) {
            return true;
        }
        if let Some(col) = self.collection.as_ref() {
            if col.gen == gen && !self.path_gen.contains_key(path) {
                return true;
            }
        }
        !self.path_gen.contains_key(path) && gen + 1 == self.next_collection_gen
    }

    /// Record that a checkout result was applied (or discarded after accept check).
    pub fn note_repo_done(&mut self, _gen: u64, path: &str) -> ApplyDecision {
        let focused = self.collection.as_ref().and_then(|c| c.focused.as_deref()) == Some(path);
        if let Some(col) = self.collection.as_mut() {
            col.pending.remove(path);
        }
        let finished = self
            .collection
            .as_ref()
            .is_some_and(|c| c.discovered && c.pending.is_empty());
        if finished {
            let (kind, focused) = self
                .collection
                .as_ref()
                .map(|c| (c.kind, c.focused.clone()))
                .unwrap_or((CollectionKind::Watch, None));
            self.collection = None;
            if self.latched_watch {
                self.latched_watch = false;
                self.start_collection(kind, focused);
                return ApplyDecision::StartDiscover {
                    gen: self.collection.as_ref().map(|c| c.gen).unwrap_or(0),
                    kind,
                };
            }
            return ApplyDecision::CollectionIdle;
        }
        ApplyDecision::ApplyRepo {
            path: path.to_string(),
            load_pane_if_focused: focused,
        }
    }

    pub fn request_pane(&mut self) -> u64 {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        self.pane_latest = id;
        self.user_queue
            .retain(|kind| !matches!(kind, SpawnKind::LoadPane { .. }));
        if self.pane_inflight.is_some() {
            self.pane_pending = true;
            return id;
        }
        self.user_queue
            .push_front(SpawnKind::LoadPane { req_id: id });
        self.pane_pending = false;
        id
    }

    pub fn accept_pane_result(&mut self, req_id: u64) -> bool {
        self.pane_inflight = None;
        let ok = req_id == self.pane_latest;
        if !ok || self.pane_pending {
            self.user_queue
                .retain(|kind| !matches!(kind, SpawnKind::LoadPane { .. }));
            self.user_queue.push_front(SpawnKind::LoadPane {
                req_id: self.pane_latest,
            });
            self.pane_pending = false;
        }
        ok
    }

    pub fn latest_pane_id(&self) -> u64 {
        self.pane_latest
    }

    /// Bump the autoload generation on each enqueue, and when a pane
    /// load replaces the graph window.
    pub fn request_autoload(&mut self) -> u64 {
        let id = self.next_autoload_id;
        self.next_autoload_id += 1;
        self.autoload_latest = id;
        id
    }

    /// True when `gen` is still the latest autoload request.
    pub fn accept_autoload_result(&self, gen: u64) -> bool {
        gen == self.autoload_latest
    }

    /// Bump the commit-files generation on each `LoadCommitFiles`.
    pub fn request_commit_files(&mut self) -> u64 {
        let id = self.next_commit_files_id;
        self.next_commit_files_id += 1;
        self.commit_files_latest = id;
        id
    }

    /// True when `gen` is still the latest commit-files request.
    pub fn accept_commit_files_result(&self, gen: u64) -> bool {
        gen == self.commit_files_latest
    }

    /// Bump the commit-diff generation on each `LoadCommitDiff` or `DropCommitDiff`.
    pub fn request_commit_diff(&mut self) -> u64 {
        let id = self.next_commit_diff_id;
        self.next_commit_diff_id += 1;
        self.commit_diff_latest = id;
        id
    }

    /// True when `gen` is still the latest commit-diff request.
    pub fn accept_commit_diff_result(&self, gen: u64) -> bool {
        gen == self.commit_diff_latest
    }

    /// Bump the stash-menu generation on each `PrepareStashMenu`.
    pub fn request_prepare_stash(&mut self) -> u64 {
        let id = self.next_prepare_stash_id;
        self.next_prepare_stash_id += 1;
        self.prepare_stash_latest = id;
        id
    }

    /// True when `gen` is still the latest stash-menu request.
    pub fn accept_prepare_stash_result(&self, gen: u64) -> bool {
        gen == self.prepare_stash_latest
    }

    /// Bump on each `PrepareBranchPicker` / `PrepareGraphFocusPicker`.
    pub fn request_prepare_branches(&mut self) -> u64 {
        let id = self.next_prepare_branches_id;
        self.next_prepare_branches_id += 1;
        self.prepare_branches_latest = id;
        id
    }

    /// True when `gen` is still the latest branch / graph-focus picker request.
    pub fn accept_prepare_branches_result(&self, gen: u64) -> bool {
        gen == self.prepare_branches_latest
    }

    pub fn bump_write_gen(&mut self) -> u64 {
        let gen = self.next_write_gen;
        self.next_write_gen += 1;
        gen
    }

    pub fn accept_write(&self, gen: u64) -> bool {
        gen + 1 == self.next_write_gen
    }

    pub fn enqueue_user(&mut self, tag: UserTag) {
        if tag == UserTag::Write {
            self.exclusive_write = true;
        }
        if tag == UserTag::DefaultBranch {
            if self.default_branch_busy {
                self.user_queue.push_back(SpawnKind::UserWork { tag });
                return;
            }
            self.default_branch_busy = true;
        }
        self.user_queue.push_back(SpawnKind::UserWork { tag });
    }

    pub fn enqueue_user_front(&mut self, tag: UserTag) {
        if tag == UserTag::Write {
            self.exclusive_write = true;
        }
        self.user_queue.push_front(SpawnKind::UserWork { tag });
    }

    pub fn note_user_done(&mut self, tag: UserTag) {
        if tag == UserTag::Write {
            self.exclusive_write = false;
        }
        if tag == UserTag::DefaultBranch {
            self.default_branch_busy = false;
        }
    }

    /// Next job under the cap. User queue first, then background status.
    pub fn spawn_next(&mut self) -> Option<SpawnRequest> {
        if self.inflight.len() >= self.cap {
            return None;
        }
        let kind = self
            .user_queue
            .pop_front()
            .or_else(|| self.status_queue.pop_front())?;
        if let SpawnKind::UserWork {
            tag: UserTag::DefaultBranch,
        } = &kind
        {
            if self.inflight.values().any(|job| {
                matches!(
                    job.kind,
                    SpawnKind::UserWork {
                        tag: UserTag::DefaultBranch
                    }
                )
            }) {
                self.user_queue.push_front(kind);
                if self.status_queue.is_empty() {
                    return None;
                }
                let kind = self.status_queue.pop_front()?;
                return Some(self.take_inflight(kind));
            }
        }
        Some(self.take_inflight(kind))
    }

    fn take_inflight(&mut self, kind: SpawnKind) -> SpawnRequest {
        let class = match &kind {
            SpawnKind::ProcessRepo { focused: false, .. } => JobClass::Status,
            SpawnKind::Discover { .. } => JobClass::User,
            _ => JobClass::User,
        };
        let id = self.next_job_id;
        self.next_job_id += 1;
        if let SpawnKind::LoadPane { req_id } = &kind {
            self.pane_inflight = Some(*req_id);
        }
        self.inflight.insert(
            id,
            Inflight {
                class,
                kind: kind.clone(),
            },
        );
        SpawnRequest { id, class, kind }
    }

    pub fn note_job_finished(&mut self, id: u64) {
        self.inflight.remove(&id);
    }

    /// Drain every ready spawn (for tests). Stops at the cap.
    pub fn spawn_ready(&mut self) -> Vec<SpawnRequest> {
        let mut out = Vec::new();
        while let Some(req) = self.spawn_next() {
            out.push(req);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("r{i}")).collect()
    }

    #[test]
    fn stale_collection_results_are_ignored() {
        let mut s = Scheduler::new(4);
        assert!(s.on_watch_tick(Some("r0".into())));
        let gen1 = s.collection_gen().unwrap();
        s.on_discovered(gen1, vec!["r0".into(), "r1".into()]);
        let _ = s.spawn_ready();
        s.note_job_finished(1);
        s.note_job_finished(2);
        s.note_job_finished(3);
        assert!(s.accept_repo_result(gen1, "r0"));
        s.note_repo_done(gen1, "r0");
        s.note_repo_done(gen1, "r1");
        assert!(!s.collection_active());

        assert!(s.on_watch_tick(Some("r0".into())));
        let gen2 = s.collection_gen().unwrap();
        assert_ne!(gen1, gen2);
        s.on_discovered(gen2, vec!["r0".into()]);
        assert!(!s.accept_repo_result(gen1, "r0"));
        assert!(s.accept_repo_result(gen2, "r0"));
    }

    #[test]
    fn focused_status_and_pane_outrank_background_status() {
        let mut s = Scheduler::new(4);
        s.on_watch_tick(Some("focus".into()));
        let gen = s.collection_gen().unwrap();
        let discover = s.spawn_next().unwrap();
        assert!(matches!(discover.kind, SpawnKind::Discover { .. }));
        s.note_job_finished(discover.id);
        s.on_discovered(
            gen,
            vec!["a".into(), "focus".into(), "b".into(), "c".into()],
        );
        s.request_pane();
        let first = s.spawn_next().unwrap();
        assert!(
            matches!(first.kind, SpawnKind::LoadPane { .. }),
            "pane must beat background status: {first:?}"
        );
        let second = s.spawn_next().unwrap();
        match &second.kind {
            SpawnKind::ProcessRepo { path, focused, .. } => {
                assert_eq!(path, "focus");
                assert!(*focused);
            }
            other => panic!("expected focused repo, got {other:?}"),
        }
        assert_eq!(first.class, JobClass::User);
        assert_eq!(second.class, JobClass::User);
        let rest: Vec<_> = s
            .spawn_ready()
            .into_iter()
            .filter_map(|req| match req.kind {
                SpawnKind::ProcessRepo { path, focused, .. } => Some((path, focused)),
                _ => None,
            })
            .collect();
        assert!(rest.iter().all(|(_, focused)| !*focused));
        assert!(rest.iter().any(|(p, _)| p == "a"));
    }

    #[test]
    fn four_process_cap() {
        let mut s = Scheduler::new(4);
        s.on_watch_tick(None);
        let gen = s.collection_gen().unwrap();
        let d = s.spawn_next().unwrap();
        s.note_job_finished(d.id);
        s.on_discovered(gen, paths(10));
        let spawned = s.spawn_ready();
        assert_eq!(spawned.len(), 4);
        assert_eq!(s.inflight_count(), 4);
        assert!(s.spawn_next().is_none());
        s.note_job_finished(spawned[0].id);
        assert_eq!(s.inflight_count(), 3);
        assert!(s.spawn_next().is_some());
        assert_eq!(s.inflight_count(), 4);
    }

    #[test]
    fn watch_tick_during_collect_latches_one_rerun() {
        let mut s = Scheduler::new(4);
        assert!(s.on_watch_tick(Some("r0".into())));
        let gen1 = s.collection_gen().unwrap();
        assert!(!s.on_watch_tick(Some("r0".into())));
        assert!(s.latched_watch());
        assert!(!s.on_watch_tick(None));
        s.on_discovered(gen1, vec!["r0".into()]);
        let _ = s.spawn_ready();
        let decision = s.note_repo_done(gen1, "r0");
        match decision {
            ApplyDecision::StartDiscover { gen, kind } => {
                assert_eq!(kind, CollectionKind::Watch);
                assert_eq!(Some(gen), s.collection_gen());
                assert_ne!(gen, gen1);
            }
            other => panic!("expected latched rerun, got {other:?}"),
        }
        assert!(!s.latched_watch());
        assert!(!s.on_watch_tick(None));
        assert!(s.latched_watch());
    }

    #[test]
    fn thirty_repo_results_apply_one_by_one() {
        let mut s = Scheduler::new(4);
        s.on_watch_tick(Some("r0".into()));
        let gen = s.collection_gen().unwrap();
        let names = paths(30);
        s.on_discovered(gen, names.clone());
        let mut applied = Vec::new();
        for (i, path) in names.iter().enumerate() {
            assert!(
                s.accept_repo_result(gen, path),
                "repo {path} gen {gen} should apply"
            );
            let decision = s.note_repo_done(gen, path);
            match decision {
                ApplyDecision::ApplyRepo { path: p, .. } => applied.push(p),
                ApplyDecision::CollectionIdle | ApplyDecision::StartDiscover { .. } => {
                    applied.push(path.clone());
                    assert_eq!(i, 29);
                }
                other => panic!("unexpected {other:?} at {i}"),
            }
        }
        assert_eq!(applied.len(), 30);
        assert_eq!(applied, names);
    }

    #[test]
    fn reload_repo_supersedes_inflight_collect_gen() {
        let mut s = Scheduler::new(4);
        s.on_watch_tick(Some("r0".into()));
        let gen1 = s.collection_gen().unwrap();
        s.on_discovered(gen1, vec!["r0".into(), "r1".into()]);
        s.on_reload_repo("r0".into());
        assert!(
            !s.accept_repo_result(gen1, "r0"),
            "older collect result must not overwrite a newer focused reload"
        );
        assert!(s.accept_repo_result(gen1, "r1"));
        assert!(s.accept_repo_result(gen1 + 1, "r0"));
    }

    #[test]
    fn stale_pane_id_is_rejected() {
        let mut s = Scheduler::new(4);
        let first = s.request_pane();
        let spawned = s.spawn_ready();
        assert!(matches!(
            spawned[0].kind,
            SpawnKind::LoadPane { req_id } if req_id == first
        ));
        let second = s.request_pane();
        assert_ne!(first, second);
        s.note_job_finished(spawned[0].id);
        assert!(!s.accept_pane_result(first));
        assert_eq!(s.latest_pane_id(), second);
        let again = s.spawn_next().unwrap();
        assert!(matches!(
            again.kind,
            SpawnKind::LoadPane { req_id } if req_id == second
        ));
    }

    #[test]
    fn stale_autoload_id_is_rejected() {
        let mut s = Scheduler::new(4);
        let first = s.request_autoload();
        let second = s.request_autoload();
        assert_ne!(first, second);
        assert!(!s.accept_autoload_result(first));
        assert!(s.accept_autoload_result(second));
    }

    #[test]
    fn stale_commit_files_id_is_rejected() {
        let mut s = Scheduler::new(4);
        let first = s.request_commit_files();
        let second = s.request_commit_files();
        assert_ne!(first, second);
        assert!(!s.accept_commit_files_result(first));
        assert!(s.accept_commit_files_result(second));
    }

    #[test]
    fn stale_commit_diff_id_is_rejected() {
        let mut s = Scheduler::new(4);
        let first = s.request_commit_diff();
        let second = s.request_commit_diff();
        assert_ne!(first, second);
        assert!(!s.accept_commit_diff_result(first));
        assert!(s.accept_commit_diff_result(second));
    }

    #[test]
    fn stale_prepare_stash_id_is_rejected() {
        let mut s = Scheduler::new(4);
        let first = s.request_prepare_stash();
        let second = s.request_prepare_stash();
        assert_ne!(first, second);
        assert!(!s.accept_prepare_stash_result(first));
        assert!(s.accept_prepare_stash_result(second));
    }

    #[test]
    fn stale_prepare_branches_id_is_rejected() {
        let mut s = Scheduler::new(4);
        let first = s.request_prepare_branches();
        let second = s.request_prepare_branches();
        assert_ne!(first, second);
        assert!(!s.accept_prepare_branches_result(first));
        assert!(s.accept_prepare_branches_result(second));
    }
}
