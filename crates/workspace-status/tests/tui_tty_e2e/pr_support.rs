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
    github_check_url, gitlab_pipeline_url, put_github, put_github_closed, put_github_with,
    put_gitlab, put_gitlab_closed, put_gitlab_with, Badge, PrText, GITHUB_REPO, GITLAB_PROJECT,
};
use crate::common::seed::{seed_repo, unique_root};
use crate::harness::PtySession;
use crate::support::{
    flat_join_col, flat_pane_last_row, FLAT_BODY_ROW, FLAT_PAD_COLS, SLATE_SIDEBAR, WAIT,
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

/// Tokyo Night remove flash ramp (CI failed, closed), peak first. The
/// whole ramp: flash checks read every recorded frame
/// ([`PtySession::wait_recorded_frame`]), so a late start still counts.
pub const REMOVE_RAMP: [(u8, u8, u8); 4] = [
    (0x77, 0x41, 0x52),
    (0x58, 0x34, 0x43),
    (0x3d, 0x2a, 0x37),
    (0x2c, 0x22, 0x2e),
];
/// Tokyo Night add flash ramp (ready to merge, merged), peak first.
pub const ADD_RAMP: [(u8, u8, u8); 4] = [
    (0x51, 0x66, 0x43),
    (0x3f, 0x4d, 0x39),
    (0x2f, 0x38, 0x31),
    (0x25, 0x29, 0x2b),
];

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

    /// [`Self::set`] with the title and base branch of `text`.
    pub fn set_with(&self, forge: &FakeForge, badge: Badge, text: PrText<'_>) {
        match self.forge {
            Forge::GitHub => {
                put_github_with(forge, GITHUB_REPO, self.number, &self.branch, badge, text)
            }
            Forge::GitLab => put_gitlab_with(
                forge,
                GITLAB_PROJECT,
                self.number,
                &self.branch,
                badge,
                text,
            ),
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
    /// Column of the last cell inside the tree pane, where a badge glyph
    /// sits. Read from the pane border, not from any row.
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

/// True when the boxed frame on screen was drawn for the current terminal
/// size: the top right corner of the right pane is in the last column. After
/// a resize the old frame stays on the grid until the app draws again, and
/// its tree edge still reads as the old width.
pub fn frame_fits_terminal(tui: &PtySession) -> bool {
    let (cols, _) = tui.grid_size();
    tui.grid_row_text(1, cols.saturating_sub(1)..cols) == "┐"
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

/// True when the tree row that paints `name` ends in `total`, two spaces
/// after the rest of the row (the workspace row: `0 changed · all current
/// 2 PRs · 2 need you`).
pub fn total_is(tui: &PtySession, name: &str, total: &str) -> bool {
    tree_row_text(tui, name).is_some_and(|row| row.trim_end().ends_with(&format!("  {total}")))
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

/// Sample the screen for `window` and panic the moment `holds` is false.
/// Proves a state is kept over a window, which a single sample cannot.
pub fn assert_holds_for(
    tui: &PtySession,
    holds: impl Fn(&PtySession) -> bool,
    window: Duration,
    what: &str,
) {
    let start = Instant::now();
    while start.elapsed() < window {
        assert!(holds(tui), "{what} did not hold:\n{}", tui.screen());
        thread::sleep(Duration::from_millis(25));
    }
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

/// One PR of a PRs tab fixture: the checkout that shows it and what the
/// fake forge answers for it.
#[derive(Clone, Copy, Debug)]
pub struct PrSpec {
    /// Checkout directory, which is also the tree row text.
    pub dir: &'static str,
    /// Checked-out branch, which is the PR head.
    pub branch: &'static str,
    /// Forge of the `origin` remote.
    pub forge: Forge,
    /// PR number (GitHub) or merge request iid (GitLab).
    pub number: u64,
    /// State the fake forge answers with.
    pub badge: Badge,
    /// Title and base branch.
    pub text: PrText<'static>,
}

impl PrSpec {
    /// The checkout of this PR.
    pub fn checkout(&self) -> PrCheckout {
        PrCheckout::new(self.dir, self.branch, self.forge, self.number)
    }
}

/// Shorthand for a [`PrSpec`] row of a fixture table.
pub const fn spec(
    dir: &'static str,
    branch: &'static str,
    forge: Forge,
    number: u64,
    badge: Badge,
    title: &'static str,
    base: &'static str,
) -> PrSpec {
    PrSpec {
        dir,
        branch,
        forge,
        number,
        badge,
        text: PrText { title, base },
    }
}

/// Nine PRs on both forges that make every kind of PRs tab group. The
/// group keys come from the branch (`feature/ABC-130-limit`) or the commit
/// scope of the title (`feat(ABC-130): …`).
///
/// | Group | PRs | Rank |
/// | --- | --- | --- |
/// | `ABC-130` | `#40` in review (`acme/app`), `!31` CI failed (`acme/services/api`) | CI failed |
/// | `ABC-120` | `#43` ready, `#44` draft, `#45` in review stacked on `#43` | needs you (`merge #43`) |
/// | `ABC-125` | `!32` CI running | waiting |
/// | `ABC-110` | `#30` and `!20` merged | finished |
/// | `other` | `#50` in review, no key | last |
///
/// Key order would list `ABC-110` first and `ABC-130` last, and number
/// order would put `!31` before `#40` and `#44` before `#45`.
pub const PRS_BOARD: [PrSpec; 9] = [
    spec(
        "api-limit",
        "feature/ABC-130-limit",
        Forge::GitLab,
        31,
        Badge::CiFailed,
        "Limit the upload rate",
        "main",
    ),
    spec(
        "app-retry",
        "fix/retry-uploads",
        Forge::GitHub,
        40,
        Badge::InReview,
        "feat(ABC-130): Retry failed uploads",
        "main",
    ),
    spec(
        "app-export",
        "feature/ABC-120-export",
        Forge::GitHub,
        43,
        Badge::Ready,
        "Export the totals",
        "main",
    ),
    spec(
        "app-export-docs",
        "feature/ABC-120-docs",
        Forge::GitHub,
        44,
        Badge::Draft,
        "Document the export",
        "main",
    ),
    spec(
        "app-export-ui",
        "feature/ABC-120-export-ui",
        Forge::GitHub,
        45,
        Badge::InReview,
        "Show the export button",
        "feature/ABC-120-export",
    ),
    spec(
        "api-queue",
        "feature/ABC-125-queue",
        Forge::GitLab,
        32,
        Badge::CiRunning,
        "Queue the uploads",
        "main",
    ),
    spec(
        "app-cache",
        "feature/ABC-110-cache",
        Forge::GitHub,
        30,
        Badge::Merged,
        "Cache the totals",
        "main",
    ),
    spec(
        "api-cache",
        "feature/ABC-110-cache",
        Forge::GitLab,
        20,
        Badge::Merged,
        "Cache the rate limits",
        "main",
    ),
    spec(
        "app-readme",
        "update-readme",
        Forge::GitHub,
        50,
        Badge::InReview,
        "Update the README",
        "main",
    ),
];

/// Seed `specs` in a workspace named `ws` under a new temp root, with the
/// fake forge answering each PR. Returns the root, the workspace, and the
/// forge.
pub fn seed_prs(prefix: &str, specs: &[PrSpec]) -> (PathBuf, PathBuf, FakeForge) {
    let (root, workspace) = named_workspace(prefix, "ws");
    let forge = FakeForge::new(&root);
    let checkouts: Vec<PrCheckout> = specs.iter().map(PrSpec::checkout).collect();
    seed_checkouts(&workspace, &checkouts);
    for spec in specs {
        spec.checkout().set_with(&forge, spec.badge, spec.text);
    }
    (root, workspace, forge)
}

/// Body rows of the boxed pane whose top border starts `┌<title>─`, from
/// the first cell inside the side borders to the last, with trailing blanks
/// cut. `None` when that pane is not on screen.
fn pane_rows(tui: &PtySession, title: &str) -> Option<Vec<String>> {
    let (cols, rows) = tui.grid_size();
    let top = tui.grid_row_text(1, 0..cols);
    let corner = format!("┌{title}─");
    let left = top.find(&corner).map(|byte| top[..byte].chars().count())? as u16;
    let right = top
        .chars()
        .skip(usize::from(left))
        .position(|c| c == '┐')
        .map(|at| left + at as u16)?;
    let body = (2..rows)
        .take_while(|row| tui.grid_cell_char(*row, left) != Some('└'))
        .map(|row| {
            tui.grid_row_text(row, left + 1..right)
                .trim_end()
                .to_string()
        })
        .collect();
    Some(body)
}

/// One line of the PRs tab list, as painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrsLine {
    /// A group header: `folded` when its chevron is `>`, and the text after
    /// the chevron (`ABC-130 · > fix CI !31`), with the poll age of the
    /// first header cut off.
    Header {
        /// The group's rows are hidden.
        folded: bool,
        /// Key, title, and next action or `m all 2 merged`.
        text: String,
    },
    /// A PR row.
    Row {
        /// State glyph (ASCII in the PTY suite).
        glyph: char,
        /// `#40` or `!31`.
        pr_ref: String,
        /// The rest of the row: repository, title, CI, reviews, state, and
        /// stack mark, single-spaced.
        rest: String,
    },
    /// A line above the groups: a CLI error or the loading line.
    Note(String),
}

/// The list pane lines of the PRs tab and the index of the one under the
/// cursor (`▌` in its first cell). `None` when the `prs` pane is not on
/// screen.
pub fn prs_list(tui: &PtySession) -> Option<(Vec<PrsLine>, Option<usize>)> {
    let rows = pane_rows(tui, "prs")?;
    let mut lines = Vec::new();
    let mut cursor = None;
    for row in rows.iter().filter(|row| !row.is_empty()) {
        // Headers and rows start with the cursor bar column; the error,
        // loading, and empty lines start in that column with their text.
        let body = if let Some(body) = row.strip_prefix('▌') {
            cursor = Some(lines.len());
            body
        } else if let Some(body) = row.strip_prefix(' ') {
            body
        } else {
            lines.push(PrsLine::Note(row.trim().to_string()));
            continue;
        };
        let words: Vec<&str> = body.split_whitespace().collect();
        let line = match words.as_slice() {
            [chevron @ ("v" | ">"), ..] if body.starts_with(&format!("{chevron} ")) => {
                let text = body[2..].split("  ").next().unwrap_or("").trim();
                PrsLine::Header {
                    folded: *chevron == ">",
                    text: text.to_string(),
                }
            }
            [glyph, pr_ref, rest @ ..]
                if glyph.chars().count() == 1
                    && pr_ref.len() > 1
                    && pr_ref.starts_with(['#', '!'])
                    && pr_ref[1..].chars().all(|c| c.is_ascii_digit()) =>
            {
                PrsLine::Row {
                    glyph: glyph.chars().next().unwrap_or(' '),
                    pr_ref: pr_ref.to_string(),
                    rest: rest.join(" "),
                }
            }
            _ => PrsLine::Note(body.trim().to_string()),
        };
        lines.push(line);
    }
    Some((lines, cursor))
}

/// Header texts of the PRs tab list, top to bottom.
pub fn prs_headers(tui: &PtySession) -> Vec<(bool, String)> {
    prs_list(tui)
        .map(|(lines, _)| {
            lines
                .into_iter()
                .filter_map(|line| match line {
                    PrsLine::Header { folded, text } => Some((folded, text)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Refs of the PR rows of the PRs tab list, top to bottom.
pub fn prs_refs(tui: &PtySession) -> Vec<String> {
    prs_list(tui)
        .map(|(lines, _)| {
            lines
                .into_iter()
                .filter_map(|line| match line {
                    PrsLine::Row { pr_ref, .. } => Some(pr_ref),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The PRs tab list line under the cursor.
pub fn prs_cursor_line(tui: &PtySession) -> Option<PrsLine> {
    let (lines, cursor) = prs_list(tui)?;
    lines.into_iter().nth(cursor?)
}

/// True when the PRs tab cursor is on the row of `pr_ref`.
pub fn prs_cursor_on(tui: &PtySession, pr_ref: &str) -> bool {
    matches!(prs_cursor_line(tui), Some(PrsLine::Row { pr_ref: at, .. }) if at == pr_ref)
}

/// Move the PRs tab cursor down with `j`, one key at a time, until the
/// line under it passes `at`. After each key it waits for the cursor line
/// to change, so a slow frame is never read as a missed key.
pub fn prs_move_until(tui: &mut PtySession, what: &str, at: impl Fn(&PrsLine) -> bool) {
    for _ in 0..40 {
        if prs_cursor_line(tui).is_some_and(|line| at(&line)) {
            return;
        }
        let before = prs_list(tui).and_then(|(_, cursor)| cursor);
        tui.key('j');
        tui.wait_pred(
            |_| prs_list(tui).and_then(|(_, cursor)| cursor) != before,
            &format!("j moves the PRs tab cursor toward {what}"),
            WAIT,
        );
    }
    panic!("the PRs tab cursor never reached {what}:\n{}", tui.screen());
}

/// [`prs_move_until`] the row of `pr_ref`.
pub fn prs_move_to(tui: &mut PtySession, pr_ref: &str) {
    prs_move_until(
        tui,
        pr_ref,
        |line| matches!(line, PrsLine::Row { pr_ref: at, .. } if at == pr_ref),
    );
}

/// Body rows of the PRs tab list pane (`prs`) as painted, trailing blanks
/// cut: the first cell is the cursor bar column.
pub fn prs_list_rows(tui: &PtySession) -> Option<Vec<String>> {
    pane_rows(tui, "prs")
}

/// Body rows of the PRs tab detail pane (`PR`), trailing blanks cut.
pub fn prs_detail(tui: &PtySession) -> Option<Vec<String>> {
    pane_rows(tui, "PR")
}

/// The tab strip (top screen row).
pub fn tab_strip(tui: &PtySession) -> String {
    let (cols, _) = tui.grid_size();
    tui.grid_row_text(0, 0..cols).trim_end().to_string()
}
