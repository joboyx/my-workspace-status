//! Viewed-mark and comment store saves, run off the loop thread.
//!
//! A save locks the store file, merges the changes this session made since
//! its last load or save into the bucket on disk, and atomic-writes the
//! file ([`save_viewed_store`], [`save_comment_store`]). That is blocking
//! file I/O, so the reducer only asks for a save ([`StoreSync::want`]).
//! The interpreter runs it as a serial worker job: one save per store file
//! at a time, so the saves of one file land in the order they started.
//! The reducer then lands the merged bucket ([`StoreSync::land`]).
//!
//! A change made while a save runs is not lost: landing applies it on top
//! of the merged bucket, and the next save sends it. A failed save keeps
//! its changes; the next change sends them again, and quit tries a failed
//! save once more. Quit waits for the running save and runs the last one
//! (`event_loop::flush_store_saves`), and reports each store it could not
//! save ([`UnsavedStore`]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use super::comments::{save_comment_store, CommentStore};
use super::persist::apply_session_delta;
use super::viewed::{save_viewed_store, workspace_store_id, ViewedStore};

/// One persisted store a save job writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    /// Viewed marks ([`super::viewed`]).
    Viewed,
    /// Comments ([`super::comments`]).
    Comments,
}

impl StoreKind {
    /// Every store kind.
    pub(crate) const ALL: [StoreKind; 2] = [StoreKind::Viewed, StoreKind::Comments];

    /// Word for the save-failure status (`viewed save failed: …`).
    pub(crate) fn label(self) -> &'static str {
        match self {
            StoreKind::Viewed => "viewed",
            StoreKind::Comments => "comment",
        }
    }

    /// What a lost save of this store loses, for the quit report.
    fn changes(self) -> &'static str {
        match self {
            StoreKind::Viewed => "reviewed-mark changes",
            StoreKind::Comments => "comment changes",
        }
    }
}

/// Why quit could not save a store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsavedReason {
    /// The save still ran, or was still asked for, when the quit grace ran
    /// out.
    TimedOut(Duration),
    /// The last save failed, the retry at quit included.
    Failed(String),
}

/// A store whose changes quit could not save. `run_tui` prints one line
/// per store to stderr once the terminal is restored ([`Self::message`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsavedStore {
    /// Store the changes belong to.
    pub kind: StoreKind,
    /// Why they are not saved.
    pub reason: UnsavedReason,
}

impl UnsavedStore {
    /// The stderr line: `comment save failed: <error>; …` (the words of the
    /// in-TUI status), or the timeout.
    pub fn message(&self) -> String {
        let (label, changes) = (self.kind.label(), self.kind.changes());
        match &self.reason {
            UnsavedReason::Failed(err) => {
                format!("{label} save failed: {err}; {changes} since the last save are lost")
            }
            UnsavedReason::TimedOut(grace) => format!(
                "{label} save did not finish within {} s at quit; {changes} since the last save may be lost",
                grace.as_secs_f64()
            ),
        }
    }
}

/// Save state of one in-memory store map against its file.
///
/// The store map itself lives on [`super::state::AppState`]; this keeps
/// what a save needs besides it.
#[derive(Clone, Debug)]
pub(crate) struct StoreSync<K, V> {
    /// The bucket as last loaded or saved. A save writes only the delta
    /// from it to the store, so entries another `ws` process saved survive.
    baseline: BTreeMap<K, V>,
    /// The store a running save sent, and the ack it shows on success.
    inflight: Option<(BTreeMap<K, V>, Option<String>)>,
    /// The store changed since the last save started.
    wanted: bool,
    /// Status the next save shows when it succeeds (`comment saved`).
    ack: Option<String>,
    /// Error of the last save when it failed. Its changes are still
    /// unsaved: the loop does not retry on its own (a disk that stays
    /// read-only would spin), but quit tries once more ([`Self::retry_failed`]).
    failed: Option<String>,
}

impl<K: Ord + Clone, V: PartialEq + Clone> StoreSync<K, V> {
    /// Sync state for a store just loaded as `loaded`.
    pub(crate) fn new(loaded: &BTreeMap<K, V>) -> Self {
        Self {
            baseline: loaded.clone(),
            inflight: None,
            wanted: false,
            ack: None,
            failed: None,
        }
    }

    /// Error of the last save when it failed and nothing saved since.
    pub(crate) fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Ask again for the save that failed, when no save is asked for or
    /// runs. Quit calls it once per store. Returns true when it asked.
    pub(crate) fn retry_failed(&mut self) -> bool {
        if self.failed.is_none() || self.pending() {
            return false;
        }
        self.wanted = true;
        true
    }

    /// Ask for a save of the current store. `ack` replaces the status the
    /// next successful save shows; `None` keeps the one already asked for.
    pub(crate) fn want(&mut self, ack: Option<&str>) {
        self.wanted = true;
        if let Some(ack) = ack {
            self.ack = Some(ack.to_string());
        }
    }

    /// A save is asked for and none runs: a worker may start one.
    pub(crate) fn ready(&self) -> bool {
        self.wanted && self.inflight.is_none()
    }

    /// A save runs on a worker.
    pub(crate) fn in_flight(&self) -> bool {
        self.inflight.is_some()
    }

    /// A save is asked for or runs.
    pub(crate) fn pending(&self) -> bool {
        self.wanted || self.inflight.is_some()
    }

    /// Start a save of `store`: the baseline and store to send. `None`
    /// unless [`Self::ready`].
    pub(crate) fn begin(
        &mut self,
        store: &BTreeMap<K, V>,
    ) -> Option<(BTreeMap<K, V>, BTreeMap<K, V>)> {
        if !self.ready() {
            return None;
        }
        self.wanted = false;
        self.inflight = Some((store.clone(), self.ack.take()));
        Some((self.baseline.clone(), store.clone()))
    }

    /// Land the running save into `store`.
    ///
    /// `Ok(merged)` (the bucket on disk after the save) becomes the new
    /// baseline, and `store` becomes `merged` plus the changes made since
    /// the save started, which the next save sends. Returns the ack to
    /// show. `Err` keeps the baseline and `store`, so the next save sends
    /// the same changes again, and records the error ([`Self::failed`]);
    /// the ack waits for the next save unless a newer one replaced it. No
    /// running save: nothing changes.
    pub(crate) fn land(
        &mut self,
        store: &mut BTreeMap<K, V>,
        result: Result<BTreeMap<K, V>, String>,
    ) -> Result<Option<String>, String> {
        let Some((sent, ack)) = self.inflight.take() else {
            return Ok(None);
        };
        let merged = match result {
            Ok(merged) => merged,
            Err(err) => {
                if self.ack.is_none() {
                    self.ack = ack;
                }
                self.failed = Some(err.clone());
                return Err(err);
            }
        };
        let mut next = merged.clone();
        apply_session_delta(&mut next, &sent, store);
        self.baseline = merged;
        self.failed = None;
        *store = next;
        Ok(ack)
    }
}

/// What one save job writes: the delta from `baseline` to `store` into
/// the file at `path`, in the bucket of workspace `cwd`.
pub(crate) enum StoreSaveJob {
    /// Viewed-mark save.
    Viewed {
        baseline: ViewedStore,
        store: ViewedStore,
        path: PathBuf,
        cwd: PathBuf,
    },
    /// Comment save.
    Comments {
        baseline: CommentStore,
        store: CommentStore,
        path: PathBuf,
        cwd: PathBuf,
    },
}

impl StoreSaveJob {
    /// Store the job writes.
    pub(crate) fn kind(&self) -> StoreKind {
        match self {
            StoreSaveJob::Viewed { .. } => StoreKind::Viewed,
            StoreSaveJob::Comments { .. } => StoreKind::Comments,
        }
    }

    /// Run the save: lock, merge with the file, atomic write. Blocking;
    /// runs on a worker (or at quit).
    pub(crate) fn run(self) -> StoreSaved {
        match self {
            StoreSaveJob::Viewed {
                baseline,
                store,
                path,
                cwd,
            } => StoreSaved::Viewed(
                save_viewed_store(&baseline, &store, &path, &workspace_store_id(&cwd))
                    .map_err(|err| err.to_string()),
            ),
            StoreSaveJob::Comments {
                baseline,
                store,
                path,
                cwd,
            } => StoreSaved::Comments(
                save_comment_store(&baseline, &store, &path, &workspace_store_id(&cwd))
                    .map_err(|err| err.to_string()),
            ),
        }
    }
}

/// Result of one save job: the merged bucket, or why the save failed.
pub(crate) enum StoreSaved {
    /// Viewed-mark save result.
    Viewed(Result<ViewedStore, String>),
    /// Comment save result.
    Comments(Result<CommentStore, String>),
}

impl StoreSaved {
    /// Store the result is for.
    pub(crate) fn kind(&self) -> StoreKind {
        match self {
            StoreSaved::Viewed(_) => StoreKind::Viewed,
            StoreSaved::Comments(_) => StoreKind::Comments,
        }
    }

    /// Failed save of `kind` (a panicked job).
    pub(crate) fn failed(kind: StoreKind, message: String) -> Self {
        match kind {
            StoreKind::Viewed => StoreSaved::Viewed(Err(message)),
            StoreKind::Comments => StoreSaved::Comments(Err(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn one_save_at_a_time_and_a_change_during_it_waits_for_the_next() {
        let mut sync = StoreSync::new(&map(&[]));
        let mut store = map(&[("a", "1")]);
        assert!(sync.begin(&store).is_none(), "nothing asked for");
        sync.want(Some("saved"));
        let (baseline, sent) = sync.begin(&store).expect("first save starts");
        assert_eq!((baseline, sent), (map(&[]), map(&[("a", "1")])));
        // A change while the save runs asks for another, but it waits.
        store.insert("b".into(), "2".into());
        sync.want(None);
        assert!(sync.in_flight() && !sync.ready());
        assert!(sync.begin(&store).is_none(), "saves of one file are serial");
        // Disk also had `x` from another process.
        let ack = sync.land(&mut store, Ok(map(&[("a", "1"), ("x", "9")])));
        assert_eq!(ack, Ok(Some("saved".into())));
        assert_eq!(store, map(&[("a", "1"), ("b", "2"), ("x", "9")]));
        let (baseline, sent) = sync.begin(&store).expect("the waiting change saves next");
        assert_eq!(baseline, map(&[("a", "1"), ("x", "9")]));
        assert_eq!(sent, store);
    }

    #[test]
    fn a_removal_during_a_save_survives_landing() {
        let mut sync = StoreSync::new(&map(&[("a", "1")]));
        let mut store = map(&[("a", "1"), ("b", "2")]);
        sync.want(None);
        sync.begin(&store).unwrap();
        store.remove("a");
        sync.want(None);
        sync.land(&mut store, Ok(map(&[("a", "1"), ("b", "2")])))
            .unwrap();
        assert_eq!(store, map(&[("b", "2")]), "landing must not bring `a` back");
        assert!(sync.ready());
    }

    #[test]
    fn a_failed_save_keeps_the_baseline_and_the_changes() {
        let mut sync = StoreSync::new(&map(&[]));
        let mut store = map(&[("a", "1")]);
        sync.want(Some("saved"));
        sync.begin(&store).unwrap();
        assert_eq!(
            sync.land(&mut store, Err("disk full".into())),
            Err("disk full".into())
        );
        assert_eq!(store, map(&[("a", "1")]));
        assert!(!sync.pending(), "no retry until the next change");
        assert_eq!(sync.failed(), Some("disk full"));
        sync.want(None);
        let (baseline, sent) = sync.begin(&store).unwrap();
        assert_eq!((baseline, sent), (map(&[]), map(&[("a", "1")])));
        assert_eq!(
            sync.land(&mut store, Ok(map(&[("a", "1")]))),
            Ok(Some("saved".into())),
            "the ack of the failed save shows when its changes land"
        );
        assert_eq!(sync.failed(), None);
    }

    #[test]
    fn a_failed_save_is_asked_again_once_by_retry_failed() {
        let mut sync = StoreSync::new(&map(&[]));
        let mut store = map(&[("a", "1")]);
        assert!(!sync.retry_failed(), "nothing failed");
        sync.want(None);
        sync.begin(&store).unwrap();
        assert!(!sync.retry_failed(), "a save runs");
        sync.land(&mut store, Err("read-only".into())).unwrap_err();
        assert!(sync.retry_failed());
        assert!(sync.ready());
        assert!(!sync.retry_failed(), "already asked for");
        let (baseline, sent) = sync.begin(&store).unwrap();
        assert_eq!((baseline, sent), (map(&[]), map(&[("a", "1")])));
        sync.land(&mut store, Ok(map(&[("a", "1")]))).unwrap();
        assert!(!sync.retry_failed() && !sync.pending());
    }

    #[test]
    fn unsaved_store_messages_name_the_store_and_the_reason() {
        let failed = UnsavedStore {
            kind: StoreKind::Comments,
            reason: UnsavedReason::Failed("Not a directory (os error 20)".into()),
        };
        assert_eq!(
            failed.message(),
            "comment save failed: Not a directory (os error 20); comment changes since the last save are lost"
        );
        let timed_out = UnsavedStore {
            kind: StoreKind::Viewed,
            reason: UnsavedReason::TimedOut(Duration::from_secs(2)),
        };
        assert_eq!(
            timed_out.message(),
            "viewed save did not finish within 2 s at quit; reviewed-mark changes since the last save may be lost"
        );
    }
}
