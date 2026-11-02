//! Background job outages: a background job source that is failing now.
//!
//! Three background jobs can fail with no key press behind them: the PR
//! lookups and polls (`gh` / `glab`, one source per forge host), the
//! scheduled background git fetch, and the live-refresh watch poll.
//! [`Outages`] holds the sources that fail now. `AppState::note_outage`
//! starts an outage (one sticky outage toast,
//! [`super::toast::Toast::outage`]) or updates its reason,
//! `AppState::note_recovered` ends it, and `AppState::dismiss_outage_toasts`
//! (Esc) hides the toasts while the outages stay. The tab strip paints
//! [`outage_header_text`] at its right end while any outage lasts.

use std::collections::BTreeMap;

use unicode_width::UnicodeWidthStr;

/// The failure mark of an outage toast and the header ([`outage_glyph`]).
const OUTAGE_GLYPH: &str = "\u{2715}";

/// [`OUTAGE_GLYPH`] in ASCII glyph mode.
const OUTAGE_GLYPH_ASCII: &str = "x";

/// Joins the labels of several failing kinds in the header.
const HEADER_LABEL_SEPARATOR: &str = " · ";

/// The failure mark: `✕`, or `x` in ASCII glyph mode.
pub fn outage_glyph(ascii: bool) -> &'static str {
    if ascii {
        OUTAGE_GLYPH_ASCII
    } else {
        OUTAGE_GLYPH
    }
}

/// The background job a source belongs to. The header names each failing
/// kind once, so several PR hosts are one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutageKind {
    /// PR lookups and polls (`gh` / `glab`).
    PrLookup,
    /// The scheduled background git fetch (`fetchMs`).
    ScheduledFetch,
    /// The live-refresh watch poll (`watchMs`).
    LiveRefresh,
}

impl OutageKind {
    /// The header label of this kind: `PR lookup failed`, `fetch failed`,
    /// or `refresh failed`.
    pub fn label(self) -> &'static str {
        match self {
            Self::PrLookup => "PR lookup failed",
            Self::ScheduledFetch => "fetch failed",
            Self::LiveRefresh => "refresh failed",
        }
    }
}

/// One background job source that can fail. The key of an outage: a
/// source fails or works as a whole.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutageSource {
    /// PR lookups on one forge host through one tool.
    PrLookup {
        /// The forge CLI: `gh` or `glab`.
        tool: &'static str,
        /// The forge host, for example `gitlab.com`.
        host: String,
    },
    /// The scheduled background git fetch.
    ScheduledFetch,
    /// The live-refresh watch poll.
    LiveRefresh,
}

impl OutageSource {
    /// The background job this source belongs to. Its header label is
    /// [`OutageKind::label`].
    pub fn kind(&self) -> OutageKind {
        match self {
            Self::PrLookup { .. } => OutageKind::PrLookup,
            Self::ScheduledFetch => OutageKind::ScheduledFetch,
            Self::LiveRefresh => OutageKind::LiveRefresh,
        }
    }
}

/// One outage: the source fails now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outage {
    /// The latest failure text, as its toast shows it
    /// (`glab (gitlab.com): invalid_token`).
    pub reason: String,
}

/// What a failure report changed ([`Outages::fail`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutageNote {
    /// The source was not failing: a new outage starts.
    Started,
    /// The source already fails; its reason text changed.
    ReasonChanged,
    /// The source already fails with the same reason text.
    Unchanged,
}

/// The sources that fail now, each with its latest reason. Held on
/// `AppState` as `outages`.
#[derive(Clone, Debug, Default)]
pub struct Outages {
    entries: BTreeMap<OutageSource, Outage>,
}

impl Outages {
    /// Note that `source` failed with `reason`: start its outage, or update
    /// the reason of the outage that goes on.
    pub fn fail(&mut self, source: OutageSource, reason: String) -> OutageNote {
        match self.entries.get_mut(&source) {
            Some(outage) if outage.reason == reason => OutageNote::Unchanged,
            Some(outage) => {
                outage.reason = reason;
                OutageNote::ReasonChanged
            }
            None => {
                self.entries.insert(source, Outage { reason });
                OutageNote::Started
            }
        }
    }

    /// End the outage of `source`. True when it was failing.
    pub fn recover(&mut self, source: &OutageSource) -> bool {
        self.entries.remove(source).is_some()
    }

    /// The sources that fail now, in [`OutageSource`] order.
    pub fn sources(&self) -> impl Iterator<Item = &OutageSource> {
        self.entries.keys()
    }

    /// True when no source fails.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The outage of `source`, if it fails now.
    #[cfg(test)]
    pub fn get(&self, source: &OutageSource) -> Option<&Outage> {
        self.entries.get(source)
    }

    /// The failing kinds, each once, in [`OutageKind`] order.
    pub fn kinds(&self) -> Vec<OutageKind> {
        let mut kinds: Vec<OutageKind> = self.entries.keys().map(OutageSource::kind).collect();
        kinds.dedup();
        kinds
    }
}

/// The header text for the failing `kinds` that fits in `max_cols`
/// columns, or `None` when not even the mark fits or nothing fails.
///
/// In order of preference: `✕ <label>` for one kind, `✕ <label> · <label>`
/// for several, `✕ <n> jobs failed` for several, then `✕` alone (ASCII:
/// `x`). One kind never reads `1 jobs failed`: it goes from its label to
/// the mark.
pub fn outage_header_text(kinds: &[OutageKind], ascii: bool, max_cols: usize) -> Option<String> {
    if kinds.is_empty() {
        return None;
    }
    let glyph = outage_glyph(ascii);
    let labels: Vec<&str> = kinds.iter().map(|kind| kind.label()).collect();
    let mut candidates = vec![format!("{glyph} {}", labels.join(HEADER_LABEL_SEPARATOR))];
    if kinds.len() > 1 {
        candidates.push(format!("{glyph} {} jobs failed", kinds.len()));
    }
    candidates.push(glyph.to_string());
    candidates
        .into_iter()
        .find(|text| UnicodeWidthStr::width(text.as_str()) <= max_cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gitlab() -> OutageSource {
        OutageSource::PrLookup {
            tool: "glab",
            host: "gitlab.com".into(),
        }
    }

    fn github() -> OutageSource {
        OutageSource::PrLookup {
            tool: "gh",
            host: "github.com".into(),
        }
    }

    #[test]
    fn fail_starts_once_then_updates_the_reason() {
        let mut outages = Outages::default();
        assert!(outages.is_empty());
        assert_eq!(
            outages.fail(gitlab(), "glab (gitlab.com): HTTP 401".into()),
            OutageNote::Started
        );
        assert_eq!(
            outages.fail(gitlab(), "glab (gitlab.com): HTTP 401".into()),
            OutageNote::Unchanged
        );
        assert_eq!(
            outages.fail(gitlab(), "glab (gitlab.com): invalid_token".into()),
            OutageNote::ReasonChanged
        );
        assert_eq!(
            outages.get(&gitlab()),
            Some(&Outage {
                reason: "glab (gitlab.com): invalid_token".into(),
            }),
            "the new reason"
        );
        assert!(outages.recover(&gitlab()));
        assert!(!outages.recover(&gitlab()), "already working");
        assert!(outages.is_empty());
        assert_eq!(
            outages.fail(gitlab(), "glab (gitlab.com): invalid_token".into()),
            OutageNote::Started,
            "a fail after a recover starts a new outage"
        );
    }

    #[test]
    fn sources_are_independent_and_pr_hosts_are_one_kind() {
        let mut outages = Outages::default();
        outages.fail(
            OutageSource::LiveRefresh,
            "git status (app): status failed".into(),
        );
        outages.fail(github(), "gh (github.com): HTTP 401".into());
        outages.fail(gitlab(), "glab (gitlab.com): invalid_token".into());
        assert_eq!(
            outages.kinds(),
            [OutageKind::PrLookup, OutageKind::LiveRefresh],
            "kind order, each kind once"
        );
        outages.recover(&github());
        assert_eq!(
            outages.kinds(),
            [OutageKind::PrLookup, OutageKind::LiveRefresh]
        );
        outages.recover(&gitlab());
        assert_eq!(outages.kinds(), [OutageKind::LiveRefresh]);
        assert!(outages.get(&OutageSource::LiveRefresh).is_some());
        assert_eq!(OutageSource::ScheduledFetch.kind().label(), "fetch failed");
        assert_eq!(github().kind().label(), "PR lookup failed");
        assert_eq!(OutageSource::LiveRefresh.kind().label(), "refresh failed");
    }

    #[test]
    fn header_text_steps_down_from_labels_to_a_count_to_the_mark() {
        use OutageKind::*;
        let text =
            |kinds: &[OutageKind], ascii: bool, cols: usize| outage_header_text(kinds, ascii, cols);
        assert_eq!(text(&[], false, 80), None);
        assert_eq!(
            text(&[PrLookup], false, 80).as_deref(),
            Some("✕ PR lookup failed")
        );
        assert_eq!(
            text(&[PrLookup], true, 80).as_deref(),
            Some("x PR lookup failed")
        );
        assert_eq!(
            text(&[PrLookup], false, 17).as_deref(),
            Some("✕"),
            "no `1 jobs`"
        );
        let two = [PrLookup, ScheduledFetch];
        assert_eq!(
            text(&two, false, 33).as_deref(),
            Some("✕ PR lookup failed · fetch failed")
        );
        assert_eq!(text(&two, false, 32).as_deref(), Some("✕ 2 jobs failed"));
        assert_eq!(text(&two, true, 15).as_deref(), Some("x 2 jobs failed"));
        assert_eq!(text(&two, false, 14).as_deref(), Some("✕"));
        assert_eq!(text(&two, false, 1).as_deref(), Some("✕"));
        assert_eq!(text(&two, false, 0), None);
    }
}
