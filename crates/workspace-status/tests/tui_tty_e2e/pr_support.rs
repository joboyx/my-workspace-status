//! Shared fixtures and screen readers for the PR status PTY tests.
//!
//! The tests drive the real binary against the fake `gh` / `glab` of
//! `scripts/fake-forge/`. This module seeds the checkouts, moves a PR to a
//! badge state, and reads the badge, the toasts, and the flash back off the
//! vt100 cell grid. The answers per state live in `tests/common/pr_scenarios.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::common::fake_forge::{set_origin, FakeForge, GITHUB_ORIGIN, GITLAB_ORIGIN};
use crate::common::pr_scenarios::{
    github_check_url, gitlab_pipeline_url, put_github, put_github_closed, put_gitlab,
    put_gitlab_closed, Badge, GITHUB_REPO, GITLAB_PROJECT,
};
use crate::common::seed::{seed_repo, unique_root};
use crate::harness::PtySession;
use crate::support::{
    flat_join_col, flat_pane_last_row, FLAT_BODY_ROW, FLAT_PAD_COLS, SLATE_SIDEBAR,
};

/// Poll period the polling tests pass as `WS_STATUS_PR_POLL_MS`.
pub const POLL_MS: &str = "300";

/// Tokyo Night `muted`: the ref colour, and the colour of a stale badge.
pub const MUTED: (u8, u8, u8) = (0x9a, 0xa5, 0xce);
/// Tokyo Night `modified`: the `need you` part of the workspace total.
pub const MODIFIED: (u8, u8, u8) = (0xe0, 0xaf, 0x68);

/// Tokyo Night colour of the badge glyph in `badge`: red failed, green
/// ready, amber running, muted draft, blue in review, purple merged.
pub fn glyph_colour(badge: Badge) -> (u8, u8, u8) {
    match badge {
        Badge::CiFailed => (0xf7, 0x76, 0x8e),
        Badge::Ready => (0x9e, 0xce, 0x6a),
        Badge::CiRunning => (0xe0, 0xaf, 0x68),
        Badge::Draft => MUTED,
        Badge::InReview => (0x7a, 0xa2, 0xf7),
        Badge::Merged => (0xbb, 0x9a, 0xf7),
    }
}

/// Tokyo Night remove flash ramp (CI failed, closed), peak first.
pub const REMOVE_RAMP: [(u8, u8, u8); 3] =
    [(0x77, 0x41, 0x52), (0x58, 0x34, 0x43), (0x3d, 0x2a, 0x37)];
/// Tokyo Night add flash ramp (ready to merge, merged), peak first.
pub const ADD_RAMP: [(u8, u8, u8); 3] =
    [(0x51, 0x66, 0x43), (0x3f, 0x4d, 0x39), (0x2f, 0x38, 0x31)];

/// Forge a seeded checkout belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    /// `gh`, remote `github.com/acme/app`.
    GitHub,
    /// `glab`, remote `gitlab.com/acme/services/api`.
    GitLab,
}

impl Forge {
    /// Remote URL of the seeded `origin`.
    pub fn origin(self) -> &'static str {
        match self {
            Self::GitHub => GITHUB_ORIGIN,
            Self::GitLab => GITLAB_ORIGIN,
        }
    }
}

/// One seeded checkout and the PR the fake forge keeps for its branch.
#[derive(Clone, Debug)]
pub struct PrCheckout {
    /// Directory name, which is also the tree row text.
    pub dir: String,
    /// Checked-out branch.
    pub branch: String,
    /// Forge of the `origin` remote.
    pub forge: Forge,
    /// PR number (GitHub) or merge request iid (GitLab).
    pub number: u64,
}

impl PrCheckout {
    /// Checkout `dir` on `branch` with PR `number` on `forge`.
    pub fn new(dir: &str, branch: &str, forge: Forge, number: u64) -> Self {
        Self {
            dir: dir.to_string(),
            branch: branch.to_string(),
            forge,
            number,
        }
    }

    /// `#10` on GitHub, `!20` on GitLab.
    pub fn ref_text(&self) -> String {
        match self.forge {
            Forge::GitHub => format!("#{}", self.number),
            Forge::GitLab => format!("!{}", self.number),
        }
    }

    /// Page `gc` opens when the PR is in `badge`.
    ///
    /// GitHub: the failed check, else the running check, else the first
    /// check. GitLab: the head pipeline. The fake checks are `lint` (job 1)
    /// and the test check (job 2), so a failed or running state points at
    /// job 2 and a passed state at job 1.
    pub fn ci_url(&self, badge: Badge) -> String {
        match self.forge {
            Forge::GitHub => match badge {
                Badge::CiFailed | Badge::CiRunning => github_check_url(self.number, 2),
                _ => github_check_url(self.number, 1),
            },
            Forge::GitLab => gitlab_pipeline_url(self.number),
        }
    }

    /// Make the fake forge answer with this PR in `badge`. Also the
    /// mid-run swap: the next lookup sees the new state.
    pub fn set(&self, forge: &FakeForge, badge: Badge) {
        match self.forge {
            Forge::GitHub => put_github(forge, GITHUB_REPO, self.number, &self.branch, badge),
            Forge::GitLab => put_gitlab(forge, GITLAB_PROJECT, self.number, &self.branch, badge),
        }
    }

    /// Make the fake forge answer with a closed, unmerged PR for this branch.
    pub fn set_closed(&self, forge: &FakeForge) {
        match self.forge {
            Forge::GitHub => put_github_closed(forge, GITHUB_REPO, self.number, &self.branch),
            Forge::GitLab => put_gitlab_closed(forge, GITLAB_PROJECT, self.number, &self.branch),
        }
    }

    /// Make the fake forge answer with no PR for this branch.
    pub fn set_none(&self, forge: &FakeForge) {
        use crate::common::fake_forge::Reply;
        match self.forge {
            Forge::GitHub => forge.github_pr_list(GITHUB_REPO, &self.branch, Reply::Json("[]")),
            Forge::GitLab => forge.gitlab_mr_list(GITLAB_PROJECT, &self.branch, Reply::Json("[]")),
        }
    }
}

/// Unique temp root and a workspace directory named `name` inside it.
pub fn named_workspace(prefix: &str, name: &str) -> (PathBuf, PathBuf) {
    let root = unique_root(prefix);
    let workspace = root.join(name);
    fs::create_dir_all(&workspace).unwrap();
    (root, workspace)
}

/// Seed every checkout in `workspace` with the `origin` of its forge. Only
/// `git remote` runs, so nothing touches the network.
pub fn seed_checkouts(workspace: &Path, checkouts: &[PrCheckout]) {
    for checkout in checkouts {
        seed_repo(workspace, &checkout.dir, &checkout.branch, false);
        set_origin(&workspace.join(&checkout.dir), checkout.forge.origin());
    }
}

/// Twelve checkouts, one per state on each forge: `gh-failed` (#10) ...
/// `gh-merged` (#15) and `gl-failed` (!20) ... `gl-merged` (!25), numbered in
/// [`Badge::ALL`] order. The fake forge answers each in its own state. The
/// workspace directory is `ws`, a short name that leaves the workspace row
/// room for its PR total.
pub fn state_board(prefix: &str) -> (PathBuf, PathBuf, FakeForge, Vec<(PrCheckout, Badge)>) {
    let (root, workspace) = named_workspace(prefix, "ws");
    let forge = FakeForge::new(&root);
    let mut board = Vec::new();
    for (side, forge_kind, base) in [("gh", Forge::GitHub, 10), ("gl", Forge::GitLab, 20)] {
        for (index, badge) in Badge::ALL.iter().enumerate() {
            let dir = format!("{side}-{}", badge.slug());
            let checkout = PrCheckout::new(
                &dir,
                &format!("feature/{dir}"),
                forge_kind,
                base + index as u64,
            );
            checkout.set(&forge, *badge);
            board.push((checkout, *badge));
        }
    }
    let checkouts: Vec<PrCheckout> = board.iter().map(|(c, _)| c.clone()).collect();
    seed_checkouts(&workspace, &checkouts);
    (root, workspace, forge, board)
}

/// Child env for a PR test: the fake forge env, then `extra`.
pub fn forge_env<'a>(
    forge: &'a FakeForge,
    extra: &[(&'a str, &'a str)],
) -> Vec<(&'a str, &'a str)> {
    let mut env: Vec<(&str, &str)> = forge.env().to_vec();
    env.extend_from_slice(extra);
    env
}

/// Where the tree paints its badge column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeEdge {
    /// Column of the last tree cell that holds content: the badge glyph.
    pub glyph_col: u16,
    /// First screen row of the tree body.
    pub first_row: u16,
    /// Last screen row of the tree body.
    pub last_row: u16,
}

/// The tree's last content column, in the boxed look (the cell left of the
/// `┐` of the `tree` title row) or in the flat look (the cell left of the
/// pad before the right pane). `None` until a frame with the tree is on
/// screen.
pub fn tree_edge(tui: &PtySession) -> Option<TreeEdge> {
    let (cols, rows) = tui.grid_size();
    let title = tui.grid_row_text(1, 0..cols);
    if let Some(border) = title.chars().position(|c| c == '┐') {
        return Some(TreeEdge {
            glyph_col: (border as u16).checked_sub(1)?,
            first_row: 2,
            last_row: rows.saturating_sub(4),
        });
    }
    let join = flat_join_col(tui, SLATE_SIDEBAR)?;
    Some(TreeEdge {
        glyph_col: join.checked_sub(FLAT_PAD_COLS + 1)?,
        first_row: FLAT_BODY_ROW,
        last_row: flat_pane_last_row(tui),
    })
}

/// What the tree paints at the badge column of one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeBadge {
    /// Screen row.
    pub row: u16,
    /// Column of the glyph: the last content cell of the tree pane.
    pub col: u16,
    /// The cell at the badge column.
    pub glyph: char,
    /// `#10` or `!20` when `<ref> ` sits right before the glyph.
    pub ref_text: Option<String>,
}

/// The last content cell of the tree row that paints `name`, and the ref
/// text before it. `None` when the row or the pane edge is not on screen.
/// The cell is not always a PR glyph: check [`TreeBadge::glyph`].
pub fn tree_badge(tui: &PtySession, name: &str) -> Option<TreeBadge> {
    let edge = tree_edge(tui)?;
    let col = edge.glyph_col;
    let row = (edge.first_row..=edge.last_row)
        .find(|row| tui.grid_row_text(*row, 0..col + 1).contains(name))?;
    let glyph = tui.grid_cell_char(row, col).unwrap_or(' ');
    let before = tui.grid_row_text(row, 0..col);
    let ref_text = before
        .strip_suffix(' ')
        .and_then(|rest| rest.rsplit(' ').next())
        .filter(|token| {
            token.len() > 1
                && token.starts_with(['#', '!'])
                && token[1..].chars().all(|c| c.is_ascii_digit())
        })
        .map(str::to_string);
    Some(TreeBadge {
        row,
        col,
        glyph,
        ref_text,
    })
}

/// Text of the tree row that paints `name`, from the first tree cell to the
/// badge column. `None` when the row is not on screen.
pub fn tree_row_text(tui: &PtySession, name: &str) -> Option<String> {
    let edge = tree_edge(tui)?;
    (edge.first_row..=edge.last_row)
        .map(|row| tui.grid_row_text(row, 0..edge.glyph_col + 1))
        .find(|text| text.contains(name))
}

/// Foreground and bold flag of the first cell of `needle` on the tree row
/// that paints `name`. `None` when the row or the text is not on screen.
pub fn tree_text_paint(tui: &PtySession, name: &str, needle: &str) -> Option<((u8, u8, u8), bool)> {
    let edge = tree_edge(tui)?;
    let row = (edge.first_row..=edge.last_row).find(|row| {
        tui.grid_row_text(*row, 0..edge.glyph_col + 1)
            .contains(name)
    })?;
    let text = tui.grid_row_text(row, 0..edge.glyph_col + 1);
    let col = text.find(needle).map(|byte| text[..byte].chars().count())?;
    let paint = tui.cell_paint(row, col as u16)?;
    Some((paint.fg?, paint.bold))
}

/// True when the tree row of `name` ends in `<ref> <glyph>` flush with the
/// pane border (`ref_text` `None`: the glyph alone, with no ref before it).
pub fn badge_is(tui: &PtySession, name: &str, ref_text: Option<&str>, glyph: char) -> bool {
    tree_badge(tui, name).is_some_and(|b| b.glyph == glyph && b.ref_text.as_deref() == ref_text)
}

/// Foreground and bold flag of the glyph cell of `name`'s badge.
pub fn badge_glyph_paint(tui: &PtySession, name: &str) -> Option<((u8, u8, u8), bool)> {
    let badge = tree_badge(tui, name)?;
    let paint = tui.cell_paint(badge.row, badge.col)?;
    Some((paint.fg?, paint.bold))
}

/// True when the first cell of `name` on its tree row sits on a flash
/// background from `ramp`. Unlike [`badge_flashes`] this holds for a row
/// that paints no badge.
pub fn row_flashes(tui: &PtySession, name: &str, ramp: &[(u8, u8, u8)]) -> bool {
    let Some(edge) = tree_edge(tui) else {
        return false;
    };
    (edge.first_row..=edge.last_row).any(|row| {
        let text = tui.grid_row_text(row, 0..edge.glyph_col + 1);
        text.find(name)
            .map(|byte| text[..byte].chars().count() as u16)
            .and_then(|col| tui.cell_paint(row, col))
            .and_then(|paint| paint.bg)
            .is_some_and(|bg| ramp.contains(&bg))
    })
}

/// Foreground of the first cell of the ref of `name`'s badge.
pub fn badge_ref_fg(tui: &PtySession, name: &str) -> Option<(u8, u8, u8)> {
    let badge = tree_badge(tui, name)?;
    let ref_text = badge.ref_text?;
    let at = badge.col.checked_sub(1 + ref_text.chars().count() as u16)?;
    tui.cell_paint(badge.row, at)?.fg
}

/// True when the glyph cell of `name`'s badge sits on a flash background
/// from `ramp`.
pub fn badge_flashes(tui: &PtySession, name: &str, ramp: &[(u8, u8, u8)]) -> bool {
    tree_badge(tui, name)
        .and_then(|b| tui.cell_paint(b.row, b.col))
        .and_then(|paint| paint.bg)
        .is_some_and(|bg| ramp.contains(&bg))
}

/// Text of the three toast rows (newest last), joined by newlines.
///
/// They are the three pane rows above the pane's bottom border, which sits
/// above the breadcrumb and status rows. PR toasts and acknowledgements
/// share this stack.
pub fn toast_text(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(3);
    lines[end.saturating_sub(3)..end].join("\n")
}

/// Count of `needle` hits in the toast rows of `screen`.
pub fn toast_count(screen: &str, needle: &str) -> usize {
    toast_text(screen).matches(needle).count()
}

/// Screen row of the tree row `name` and the column of its badge glyph, for
/// a mouse click that pins the PR popover.
pub fn click_badge(tui: &mut PtySession, name: &str) {
    let badge = tree_badge(tui, name).unwrap_or_else(|| panic!("no tree row `{name}`"));
    tui.sgr_click(badge.col, badge.row);
}

/// `calls.log` lines of `forge` that are a list lookup (`gh pr list`, the
/// GitLab merge request list), not a detail or approvals call.
pub fn list_calls(forge: &FakeForge) -> Vec<String> {
    forge
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("gh pr list ") || call.contains("/merge_requests?"))
        .collect()
}

/// Words only a PR alert toast carries. The graph chip and the popover use
/// some of them too, but never in the toast rows.
pub const ALERT_WORDS: [&str; 5] = [
    "CI failed",
    "ready to merge",
    "merged",
    "closed",
    "more PR changes",
];

/// True when the toast rows of `screen` hold a PR alert.
pub fn has_alert(screen: &str) -> bool {
    let toasts = toast_text(screen);
    ALERT_WORDS.iter().any(|word| toasts.contains(word))
}

/// Watch the screen until `ready` holds, then for `quiet` longer, and panic
/// the moment a PR alert shows. A missing alert is only provable over a
/// window, so this samples every 25 ms and never sleeps blind.
pub fn watch_quiet(
    tui: &PtySession,
    ready: impl Fn(&PtySession) -> bool,
    quiet: Duration,
    what: &str,
) {
    let start = Instant::now();
    let mut ready_at: Option<Instant> = None;
    loop {
        let screen = tui.screen();
        assert!(!has_alert(&screen), "a PR alert showed {what}:\n{screen}");
        if ready_at.is_none() && ready(tui) {
            ready_at = Some(Instant::now());
        }
        if let Some(at) = ready_at {
            if at.elapsed() >= quiet {
                return;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(20) + quiet,
            "timeout waiting for the quiet window {what}:\n{screen}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}
