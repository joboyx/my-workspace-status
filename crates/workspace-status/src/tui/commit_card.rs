//! Data behind the commit popovers (hash card, time, author).
//!
//! A commit or stash of the loaded graph of the same checkout feeds its
//! popover straight from the graph model. Any other commit (a blame line, a
//! compare header, a commit past the graph window) is looked up once on the
//! blocking pool ([`super::graph_load::load_commit_card`] through
//! `UserTag::CommitCard`) and kept in a [`CommitCardCache`]. Paint only
//! reads these; it never runs git.

use std::collections::HashMap;

use workspace_status_graph::{Commit, Stash};

use super::state::AppState;

/// Cards kept before the cache starts over.
const CACHE_CAP: usize = 256;

/// Lazy commit card answers per (checkout, full commit id).
///
/// An `Err` keeps the reason the lookup failed, so the popover says why
/// and the same commit is not looked up again.
#[derive(Clone, Debug, Default)]
pub struct CommitCardCache {
    cards: HashMap<(String, String), Result<Commit, String>>,
}

impl CommitCardCache {
    /// Cached answer for commit `sha` of checkout `repo`; `None` when not
    /// looked up yet.
    pub fn get(&self, repo: &str, sha: &str) -> Option<&Result<Commit, String>> {
        self.cards.get(&(repo.to_string(), sha.to_string()))
    }

    /// Store the answer for commit `sha` of checkout `repo`. A full cache
    /// starts over first.
    pub fn insert(&mut self, repo: String, sha: String, result: Result<Commit, String>) {
        let key = (repo, sha);
        if self.cards.len() >= CACHE_CAP && !self.cards.contains_key(&key) {
            self.cards.clear();
        }
        self.cards.insert(key, result);
    }

    /// Cached answers held now.
    #[cfg(test)]
    pub fn cached_len(&self) -> usize {
        self.cards.len()
    }
}

/// Where the data of one commit popover comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitCardSource<'a> {
    /// A commit of the loaded graph, or a looked-up card.
    Commit(&'a Commit),
    /// A stash of the loaded graph.
    Stash(&'a Stash),
    /// Not in the graph and not looked up yet: a lookup is due.
    Loading,
    /// The lookup failed for this reason.
    Failed(&'a str),
}

/// The data of commit `sha` of checkout `repo`: the loaded graph of that
/// checkout first (its commits, then its stashes), then the lookup cache.
/// A miss in both is [`CommitCardSource::Loading`].
pub fn commit_card_source<'a>(state: &'a AppState, repo: &str, sha: &str) -> CommitCardSource<'a> {
    let graph = state
        .graph
        .as_ref()
        .filter(|_| state.graph_identity.as_ref().map(|(r, _)| r.as_str()) == Some(repo));
    if let Some(model) = graph {
        if let Some(commit) = model.commits.iter().find(|commit| commit.id == sha) {
            return CommitCardSource::Commit(commit);
        }
        if let Some(stash) = model.stashes.iter().find(|stash| stash.id == sha) {
            return CommitCardSource::Stash(stash);
        }
    }
    match state.commit_cards.get(repo, sha) {
        Some(Ok(commit)) => CommitCardSource::Commit(commit),
        Some(Err(reason)) => CommitCardSource::Failed(reason),
        None => CommitCardSource::Loading,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str) -> Commit {
        Commit {
            id: id.into(),
            ..Commit::default()
        }
    }

    #[test]
    fn a_full_cache_starts_over_and_a_known_key_does_not() {
        let mut cache = CommitCardCache::default();
        for n in 0..CACHE_CAP {
            cache.insert("app".into(), format!("{n:040}"), Ok(commit("x")));
        }
        assert_eq!(cache.cached_len(), CACHE_CAP);
        cache.insert("app".into(), format!("{:040}", 0), Err("boom".into()));
        assert_eq!(cache.cached_len(), CACHE_CAP, "a known key does not reset");
        cache.insert("app".into(), "f".repeat(40), Ok(commit("y")));
        assert_eq!(
            cache.cached_len(),
            1,
            "a new key on a full cache starts over"
        );
        assert!(cache.get("app", &"f".repeat(40)).is_some());
        assert!(
            cache.get("lib", &"f".repeat(40)).is_none(),
            "keyed per repo"
        );
    }
}
