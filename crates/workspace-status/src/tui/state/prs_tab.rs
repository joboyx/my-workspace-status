//! The PRs tab: the PR of every checked-out branch, grouped by ticket key
//! ([`super::super::pr_groups`]), beside the detail of the focused one.
//!
//! The PRs live in the PR cache. This module builds the list from it on
//! each paint and key ([`AppState::prs_view`]) and keeps only the view
//! state in [`PrsTab`]: the cursor line by identity, the folds the user
//! set, and the narrow layout's list / detail toggle. One PR seen from
//! several checkouts is one row; the detail lists every checkout. Each
//! member of a checkout's stack that no visible checkout has is a
//! stack-only row too ([`with_stack_members`]). Keys act on the focused PR
//! through the paths the badge keys use
//! ([`AppState::run_pr_action_on_checkout`]); on a stack-only row they act
//! on its URL and CI link with no lookup. Git writes refuse with
//! [`PRS_TAB_READ_ONLY`].
//!
//! Group titles come from the title command on a worker
//! ([`Effect::FillPrTitles`]), once per key per cycle: the tab opening,
//! and each time the PR lookups go idle. The titles of the last fill stay
//! on screen until the next one lands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui::layout::Position;

use super::super::action::{Action, Effect};
use super::super::drill::DrillView;
use super::super::popover::IconTarget;
use super::super::pr_groups::{
    finished_start_folded, group_prs, with_stack_members, GroupHeadline, PrEntry, PrGroup, PrRow,
    StackSource, TitleCache,
};
use super::super::pr_status::PrState;
use super::super::pull_request::{
    is_web_url, no_ci_status, parse_remote, PrPage, PullRequest, NO_PR_FOR_ROW, PR_LOOKUP_RUNNING,
};
use super::super::selection::TextSelection;
use super::super::split::{SplitDrag, SplitHit};
use super::super::status::StatusMessage;
use super::super::tabs::{
    PrGroupId, PrsLineId, PrsTab, PRS_TAB_HAS_NO_SEARCH, PRS_TAB_ONLY, PRS_TAB_READ_ONLY,
    SWITCH_TO_WORKSPACE_TAB,
};
use super::super::tree::{cursor_for_viewport_top, list_viewport_start};
use super::popover::then;
use super::pull_request::PrAction;
use super::{single_or_batch, AppState, FocusPane};

/// Narrowest frame (columns) that paints the detail pane beside the list.
/// A narrower frame shows one pane, and `Tab` switches list and detail.
pub(crate) const PRS_DETAIL_MIN_TERM_COLS: u16 = 100;

/// Narrowest detail pane (columns) beside the list. It takes a third of
/// the width, and never less than this.
pub(crate) const PRS_DETAIL_MIN_COLS: u16 = 40;

/// Flash id of the PRs tab row of the PR with web URL `url`.
pub(crate) fn prs_row_flash_id(url: &str) -> String {
    format!("prs:{url}")
}

/// Identity of `group`.
pub(crate) fn pr_group_id(group: &PrGroup) -> PrGroupId {
    (group.kind, group.key.clone())
}

/// One line of the PRs tab list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrsLine {
    /// The header of group `group` (an index into [`PrsView::groups`]).
    Header {
        /// The group.
        group: usize,
        /// True when its rows are hidden.
        folded: bool,
    },
    /// Row `row` of group `group`.
    Row {
        /// The group.
        group: usize,
        /// The row in [`PrGroup::rows`].
        row: usize,
    },
}

impl PrsLine {
    /// The group the line belongs to.
    pub(crate) fn group(self) -> usize {
        match self {
            Self::Header { group, .. } | Self::Row { group, .. } => group,
        }
    }
}

/// When the listed PRs were looked up, for the first group header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrsChecked {
    /// The oldest last-success time of the listed open PRs (of every
    /// listed PR when none is open, the newest).
    pub(crate) at: Instant,
    /// A lookup failed, or a listed PR is stale: the header says
    /// `lookups failing`.
    pub(crate) failing: bool,
}

/// Everything the PRs tab paints, built from the PR cache.
#[derive(Clone, Debug, Default)]
pub(crate) struct PrsView {
    /// Groups in display order ([`group_prs`]). One row per distinct PR.
    pub(crate) groups: Vec<PrGroup>,
    /// The list: each header, then its rows unless it is folded.
    pub(crate) lines: Vec<PrsLine>,
    /// Index of the cursor line in [`Self::lines`]; `None` when empty.
    pub(crate) cursor: Option<usize>,
    /// Checkouts that show each PR, by PR URL, in snapshot order. The
    /// first one is the row's checkout. A stack-only row has no entry.
    pub(crate) checkouts: HashMap<String, Vec<PathBuf>>,
    /// Checkouts whose stack lists each stack-only PR, by PR URL
    /// ([`with_stack_members`]). The first one is the row's checkout.
    pub(crate) listed_by: HashMap<String, Vec<PathBuf>>,
    /// One line per failing forge CLI and host
    /// ([`AppState::pr_lookup_error_lines`]).
    pub(crate) errors: Vec<String>,
    /// The `checked` age of the first header; `None` before a lookup
    /// succeeded for a listed PR.
    pub(crate) checked: Option<PrsChecked>,
    /// Landed and total lookups ([`AppState::pr_lookup_progress`]).
    pub(crate) lookups: (usize, usize),
}

impl PrsView {
    /// Identity of `line`.
    pub(crate) fn line_id(&self, line: PrsLine) -> PrsLineId {
        match line {
            PrsLine::Header { group, .. } => PrsLineId::Group(pr_group_id(&self.groups[group])),
            PrsLine::Row { group, row } => {
                PrsLineId::Pr(self.groups[group].rows[row].pr.url.clone())
            }
        }
    }

    /// The row whose PR a header stands for: the PR its next action
    /// targets, else its first row.
    pub(crate) fn header_row(&self, group: usize) -> &PrRow {
        let group = &self.groups[group];
        let target = match &group.headline {
            GroupHeadline::Next(step) => group.rows.iter().find(|row| row.pr.url == step.url),
            GroupHeadline::AllMerged(_) | GroupHeadline::Bare => None,
        };
        target.unwrap_or(&group.rows[0])
    }

    /// The PR row of `line`: the row itself, or the one a header stands
    /// for ([`Self::header_row`]).
    pub(crate) fn line_row(&self, line: PrsLine) -> &PrRow {
        match line {
            PrsLine::Header { group, .. } => self.header_row(group),
            PrsLine::Row { group, row } => &self.groups[group].rows[row],
        }
    }

    /// The focused PR: the row under the cursor, or the PR its header
    /// stands for. `None` with no PR listed.
    pub(crate) fn focused(&self) -> Option<&PrRow> {
        Some(self.line_row(self.lines[self.cursor?]))
    }

    /// Every checkout that shows the PR of `row`, the row's own first.
    /// Empty for a stack-only row ([`PrRow::checked_out`] false).
    pub(crate) fn checkouts_of<'a>(&'a self, row: &'a PrRow) -> &'a [PathBuf] {
        if !row.checked_out {
            return &[];
        }
        self.checkouts
            .get(&row.pr.url)
            .map_or(std::slice::from_ref(&row.checkout), Vec::as_slice)
    }

    /// Every checkout whose stack lists the stack-only PR of `row`, the
    /// row's own first. Empty for a checked-out row.
    pub(crate) fn listed_by<'a>(&'a self, row: &'a PrRow) -> &'a [PathBuf] {
        if row.checked_out {
            return &[];
        }
        self.listed_by
            .get(&row.pr.url)
            .map_or(std::slice::from_ref(&row.checkout), Vec::as_slice)
    }
}

/// What [`AppState::pr_entries`] builds: the entries for [`group_prs`],
/// and the checkouts of each PR by URL.
struct PrsEntries {
    /// One entry per listed PR.
    entries: Vec<PrEntry>,
    /// [`PrsView::checkouts`].
    checkouts: HashMap<String, Vec<PathBuf>>,
    /// [`PrsView::listed_by`].
    listed_by: HashMap<String, Vec<PathBuf>>,
}

/// The PR the keys of the PRs tab act on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PrsTarget {
    /// The PR of this checkout.
    Checkout(PathBuf),
    /// A stack member no visible checkout has, and the checkouts whose
    /// stack lists it.
    StackOnly {
        /// The member as the stack walk found it.
        pr: PullRequest,
        /// [`PrsView::listed_by`] of its row.
        listed_by: Vec<PathBuf>,
    },
}

/// Status line when `o` asks for the checkout of a stack-only PR `pr_ref`.
pub(crate) fn not_checked_out_status(pr_ref: &str) -> String {
    format!("{pr_ref} is not checked out")
}

/// The cursor line of `view` for `tab`: the line [`PrsTab::cursor`] names;
/// for a PR whose group is folded, that group's header; else the line at
/// [`PrsTab::cursor_index`], clamped. `None` for an empty list.
fn resolve_cursor(view: &PrsView, tab: &PrsTab) -> Option<usize> {
    let last = view.lines.len().checked_sub(1)?;
    if let Some(id) = tab.cursor.as_ref() {
        if let Some(index) = view
            .lines
            .iter()
            .position(|line| view.line_id(*line) == *id)
        {
            return Some(index);
        }
        if let PrsLineId::Pr(url) = id {
            let group = view
                .groups
                .iter()
                .position(|group| group.rows.iter().any(|row| row.pr.url == *url));
            if let Some(index) = group.and_then(|group| {
                view.lines.iter().position(
                    |line| matches!(line, PrsLine::Header { group: g, .. } if *g == group),
                )
            }) {
                return Some(index);
            }
        }
    }
    Some(tab.cursor_index.min(last))
}

/// The PRs tab group titles from the title command, and its fill state.
#[derive(Clone, Debug, Default)]
pub(crate) struct PrTitles {
    /// The program that names a ticket key (`prTitleCommand`); `None`
    /// runs nothing and the headers show the key only.
    pub(crate) command: Option<PathBuf>,
    /// The titles on screen: each fill that landed, merged over the ones
    /// before ([`TitleCache::merged`]).
    shown: TitleCache,
    /// A new cycle wants a fill.
    due: bool,
    /// A fill is queued or running.
    running: bool,
    /// The ticket keys of the last fill that started, sorted.
    filled_keys: Vec<String>,
    /// When the last fill landed; `None` before the first one.
    filled_at: Option<Instant>,
    /// The next cycle fills even when the keys and the age would not ask
    /// for it (`r` on the PRs tab).
    forced: bool,
}

impl AppState {
    /// True when the PRs tab is active.
    pub fn is_prs_tab(&self) -> bool {
        self.tabs.active_prs().is_some()
    }

    /// Strip labels in paint order: the PRs tab counts the rows it lists
    /// ([`Self::pr_entries`]: distinct PRs, stack members no checkout has
    /// included).
    pub(crate) fn tab_labels(&self) -> Vec<String> {
        let listed = if self.tabs.find_prs().is_some() {
            self.pr_entries().entries.len()
        } else {
            0
        };
        self.tabs.labels(listed)
    }

    /// One [`PrEntry`] per distinct PR whose badge shows on a visible
    /// checkout, in snapshot order, then one per stack member that no
    /// visible checkout has ([`with_stack_members`]); the checkouts that
    /// show each PR, and the checkouts whose stack lists each stack-only
    /// PR (by URL). The entry takes the first checkout; its repository is
    /// the forge path of the remote (`acme/app`), and its default branch
    /// the one of the stack rule ([`Self::pr_stack_default_branch`]).
    fn pr_entries(&self) -> PrsEntries {
        let mut entries = Vec::new();
        let mut checkouts: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let mut sources = Vec::new();
        for row in self.pr_rows() {
            let path = Path::new(&row.repo);
            let Some(pr) = self.pull_request_for(path) else {
                continue;
            };
            let default_branch = self.pr_stack_default_branch(path);
            let repo = self
                .pr_cache
                .remote_for(path)
                .and_then(parse_remote)
                .map_or_else(|| row.repo.clone(), |repo| repo.path);
            if let Some(stack) = self.pr_stack(path) {
                sources.push((path, repo.clone(), default_branch, stack));
            }
            let seen = checkouts.entry(pr.url.clone()).or_default();
            seen.push(path.to_path_buf());
            if seen.len() > 1 {
                continue;
            }
            entries.push(PrEntry {
                checkout: path.to_path_buf(),
                repo,
                pr: pr.clone(),
                default_branch: default_branch.map(str::to_string),
                checked_out: true,
                stack_position: None,
            });
        }
        let sources: Vec<StackSource<'_>> = sources
            .iter()
            .map(|(checkout, repo, default_branch, stack)| StackSource {
                checkout,
                repo,
                default_branch: *default_branch,
                stack,
            })
            .collect();
        let (entries, listed_by) = with_stack_members(entries, &sources);
        PrsEntries {
            entries,
            checkouts,
            listed_by,
        }
    }

    /// The first checkout of every checked-out PR the PRs tab lists (a
    /// stack-only row has none, and gets no detail fetch).
    pub(super) fn prs_listed_checkouts(&self) -> Vec<PathBuf> {
        self.pr_entries()
            .entries
            .into_iter()
            .filter(|entry| entry.checked_out)
            .map(|entry| entry.checkout)
            .collect()
    }

    /// The view of `tab` at `now`.
    fn build_prs_view(&self, tab: &PrsTab, now: Instant) -> PrsView {
        let PrsEntries {
            entries,
            checkouts,
            listed_by,
        } = self.pr_entries();
        let groups = group_prs(entries);
        let fold_default = finished_start_folded(&groups);
        let mut lines = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            let folded = tab
                .folds
                .get(&pr_group_id(group))
                .copied()
                .unwrap_or(fold_default && group.is_finished());
            lines.push(PrsLine::Header {
                group: index,
                folded,
            });
            if !folded {
                lines.extend((0..group.rows.len()).map(|row| PrsLine::Row { group: index, row }));
            }
        }
        let mut view = PrsView {
            groups,
            lines,
            cursor: None,
            checkouts,
            listed_by,
            errors: self.pr_lookup_error_lines(),
            checked: None,
            lookups: self.pr_lookup_progress(),
        };
        view.cursor = resolve_cursor(&view, tab);
        view.checked = self.prs_checked(&view, now);
        view
    }

    /// The `checked` age of `view` at `now` ([`PrsChecked`]).
    fn prs_checked(&self, view: &PrsView, now: Instant) -> Option<PrsChecked> {
        let mut open_oldest: Option<Instant> = None;
        let mut newest: Option<Instant> = None;
        let mut stale = false;
        // A stack-only row is as fresh as the checkout whose stack lists
        // it, which is listed too.
        let rows = view.groups.iter().flat_map(|group| &group.rows);
        for row in rows.filter(|row| row.checked_out) {
            let Some(at) = self.pr_checked_at(&row.checkout) else {
                continue;
            };
            newest = Some(newest.map_or(at, |newest| newest.max(at)));
            if row.pr.state != PrState::Merged {
                open_oldest = Some(open_oldest.map_or(at, |oldest| oldest.min(at)));
            }
            stale |= self.pr_is_stale(&row.checkout, now);
        }
        Some(PrsChecked {
            at: open_oldest.or(newest)?,
            failing: stale || !view.errors.is_empty(),
        })
    }

    /// The view of the active PRs tab; `None` off it.
    pub(crate) fn prs_view(&self) -> Option<PrsView> {
        let tab = self.tabs.active_prs()?;
        Some(self.build_prs_view(tab, Instant::now()))
    }

    /// [`Self::prs_view`], and the cursor line it resolved stored in the
    /// tab, so the next build starts from it.
    pub(crate) fn sync_prs_view(&mut self) -> Option<PrsView> {
        let view = self.prs_view()?;
        if let Some(index) = view.cursor {
            let id = view.line_id(view.lines[index]);
            if let Some(tab) = self.tabs.active_prs_mut() {
                tab.cursor = Some(id);
                tab.cursor_index = index;
            }
        }
        Some(view)
    }

    /// What the keys of the active PRs tab act on: the focused PR's first
    /// checkout, or the focused stack-only PR itself.
    fn prs_focused_target(&self) -> Option<PrsTarget> {
        let view = self.prs_view()?;
        let row = view.focused()?;
        if row.checked_out {
            return view
                .checkouts_of(row)
                .first()
                .cloned()
                .map(PrsTarget::Checkout);
        }
        Some(PrsTarget::StackOnly {
            pr: row.pr.clone(),
            listed_by: view.listed_by(row).to_vec(),
        })
    }

    /// Popover owner row of the active PRs tab: its cursor line, so a
    /// pinned popover closes when the cursor moves.
    pub(super) fn prs_popover_owner_row(&self) -> Option<String> {
        let tab = self.tabs.active_prs()?;
        Some(match tab.cursor.as_ref() {
            Some(PrsLineId::Group((kind, key))) => format!("prs-group:{kind:?}:{key}"),
            Some(PrsLineId::Pr(url)) => prs_row_flash_id(url),
            None => String::new(),
        })
    }

    /// True when the frame is too narrow to paint the detail beside the
    /// list ([`PRS_DETAIL_MIN_TERM_COLS`]).
    pub(crate) fn prs_narrow(&self) -> bool {
        self.layout.term_cols < PRS_DETAIL_MIN_TERM_COLS
    }

    /// Open the PRs tab, or focus it (`gp`, palette PRs tab, a click on
    /// the workspace PR total). Asks for the detail of every listed PR;
    /// a new tab also starts a title cycle.
    pub(crate) fn open_prs_tab(&mut self) -> Effect {
        if self.is_prs_tab() {
            return Effect::None;
        }
        self.abandon_compare_picker();
        self.clear_tab_transients();
        self.park_active_session();
        let created = self.tabs.open_or_focus_prs();
        self.apply_active_session();
        let titles = if created {
            self.new_pr_title_cycle(Instant::now())
        } else {
            Effect::None
        };
        then(self.request_pr_details(), titles)
    }

    /// Why `action` refuses on the PRs tab, or `None` (off the tab too).
    ///
    /// Git writes say [`PRS_TAB_READ_ONLY`]; view keys and actions of the
    /// Workspace panes say [`SWITCH_TO_WORKSPACE_TAB`]; `/`, `n`, and `N`
    /// say [`PRS_TAB_HAS_NO_SEARCH`]. The keys the tab gives its own
    /// meaning (`o`, `y`, `r`, `z`) are not refused here.
    pub(crate) fn prs_tab_refusal(&self, action: &Action) -> Option<String> {
        if !self.is_prs_tab() {
            return None;
        }
        let reason = match action {
            Action::SearchStart | Action::SearchNext | Action::SearchPrev => PRS_TAB_HAS_NO_SEARCH,
            Action::Stage
            | Action::Unstage
            | Action::Revert
            | Action::StashMenu
            | Action::StashMenuEnter
            | Action::Fetch
            | Action::Pull
            | Action::Push
            | Action::DefaultBranch
            | Action::Branch
            | Action::BranchSubmit
            | Action::CreateBranchSubmit
            | Action::RemoveWorktree
            | Action::GraphStashApply
            | Action::GraphStashPop
            | Action::GraphStashDrop
            | Action::GraphCheckout
            | Action::GraphCreateBranch
            | Action::GraphMerge
            | Action::ConfirmYes
            | Action::ConfirmYesClean => PRS_TAB_READ_ONLY,
            Action::ToggleTreeMode
            | Action::ToggleShowHidden
            | Action::FocusThisFeature
            | Action::FocusThisRepo
            | Action::FocusClear
            | Action::FocusPromptOpen
            | Action::ToggleCommitMsgExpand
            | Action::ResizeCommitMsg(_)
            | Action::ToggleFullContext
            | Action::ToggleDiffMode
            | Action::ToggleDiffWrap
            | Action::ToggleIgnoreWhitespace
            | Action::ResizeTree(_)
            | Action::ToggleReviewed
            | Action::Edit
            | Action::ExternalDiff
            | Action::CommentStart
            | Action::DiffVisualStart
            | Action::CopyEntityReference
            | Action::ViewRemotely
            | Action::ExplorerReveal
            | Action::GraphFocusClear
            | Action::GraphFocusSubmit
            | Action::ToggleLineBlame
            | Action::BlameMenu
            | Action::BlameCommitVsParent
            | Action::BlamePreviousChange
            | Action::BlameCommitVsWorktree
            | Action::BlameRevealGraph
            | Action::CompareVsDefault
            | Action::CompareVsBranch
            | Action::CompareVsCommit
            | Action::CompareCommitVsParent
            | Action::GoToLine(_) => SWITCH_TO_WORKSPACE_TAB,
            _ => return None,
        };
        Some(reason.into())
    }

    /// Palette row reason on the PRs tab after [`Self::prs_tab_refusal`]:
    /// the rows whose key the tab gives its own meaning (Copy comments
    /// `y`, Graph focus branches `o`, Refresh `r`) are Workspace rows here,
    /// and the PR rows need a listed PR. The tab's `r` key and the Refresh
    /// PRs row look the PRs up again.
    pub(crate) fn prs_tab_palette_reason(&self, action: &Action) -> Option<String> {
        if let Some(reason) = self.prs_tab_refusal(action) {
            return Some(reason);
        }
        match action {
            Action::ExportComments | Action::GraphFocusBranches | Action::Refresh => {
                Some(SWITCH_TO_WORKSPACE_TAB.into())
            }
            Action::OpenPullRequest
            | Action::OpenPullRequestCi
            | Action::CopyPullRequestUrl
            | Action::RefreshPullRequest
                if !self.popover_pinned() && self.prs_focused_target().is_none() =>
            {
                Some(NO_PR_FOR_ROW.into())
            }
            _ => None,
        }
    }

    /// Keys the active PRs tab handles itself. `None` lets
    /// [`Self::dispatch`] fall through to the shared router (quit, help,
    /// theme, mouse, tabs, Ctrl-w, Quick Open, the popover, clicks, ticks).
    ///
    /// | Key | Action |
    /// | --- | --- |
    /// | `j` `k` arrows, `gg` `G`, page keys, wheel | move (the narrow detail: scroll it) |
    /// | Enter, `gx` | open the PR |
    /// | `gc` | open its CI page |
    /// | `y` | copy its URL |
    /// | `r` | look every PR up again |
    /// | `o` | show the PR's checkout in the Workspace tree (a stack-only PR: `<ref> is not checked out`) |
    /// | `z` | fold or unfold the group under the cursor |
    /// | `Z` | fold or unfold every finished group |
    /// | `Tab` | narrow layout: list or detail |
    /// | `gh` | pin the focused PR's popover |
    ///
    /// In a pinned popover `gx`, `gc`, and `r` act on the popover's PR, as
    /// on the Workspace tab.
    pub(super) fn dispatch_prs_tab(&mut self, action: &Action) -> Option<Effect> {
        let pinned = self.popover_pinned();
        if self.prs_detail_focused() {
            let delta = match *action {
                Action::Move(delta) => Some(delta as isize),
                Action::MoveToStart => Some(isize::MIN / 2),
                Action::MoveToEnd => Some(isize::MAX / 2),
                Action::PageMove(pages) => {
                    let page = self.layout.prs_detail.height.saturating_sub(1).max(1);
                    Some(pages as isize * page as isize)
                }
                _ => None,
            };
            if let Some(delta) = delta {
                self.scroll_prs_detail_by(delta);
                return Some(Effect::None);
            }
        }
        let effect = match *action {
            Action::Move(delta) => self.prs_move(delta as isize),
            Action::MoveToStart => self.prs_move(isize::MIN / 2),
            Action::MoveToEnd => self.prs_move(isize::MAX / 2),
            Action::PageMove(pages) => {
                let page = self.layout.prs_list_rows.height.saturating_sub(1).max(1);
                self.prs_move(pages as isize * page as isize)
            }
            Action::ScrollWheel {
                col,
                row,
                delta,
                horizontal,
            } => self.prs_wheel(col, row, delta, horizontal),
            Action::OpenPullRequest | Action::OpenPullRequestCi | Action::RefreshPullRequest
                if pinned =>
            {
                return None
            }
            Action::NavEnter | Action::OpenPullRequest => self.prs_pr_action(PrAction::Open),
            Action::OpenPullRequestCi => self.prs_pr_action(PrAction::OpenCi),
            Action::ExportComments | Action::CopyPullRequestUrl => {
                self.prs_pr_action(PrAction::CopyUrl)
            }
            Action::RefreshPullRequest => self.prs_pr_action(PrAction::Refresh),
            Action::Refresh => self.prs_refresh(),
            Action::GraphFocusBranches => self.prs_show_in_tree(),
            Action::FoldToggle | Action::FoldToggleSubtree => self.prs_toggle_fold(),
            Action::ToggleFinishedPrGroups => self.prs_toggle_finished(),
            Action::FocusLeft | Action::FocusRight => self.prs_toggle_detail(),
            Action::NavEsc | Action::BackClick
                if self.tabs.active_prs().is_some_and(|tab| tab.show_detail) =>
            {
                self.prs_toggle_detail()
            }
            Action::PopoverOpenFocused => self.prs_pin_focused(),
            Action::PanDiff(_)
            | Action::FoldOpen
            | Action::FoldClose
            | Action::NavEsc
            | Action::BackClick => Effect::None,
            _ => return None,
        };
        Some(effect)
    }

    /// Move the cursor `delta` lines, clamped to the list.
    fn prs_move(&mut self, delta: isize) -> Effect {
        let Some(view) = self.sync_prs_view() else {
            return Effect::None;
        };
        let Some(last) = view.lines.len().checked_sub(1) else {
            return Effect::None;
        };
        let current = view.cursor.unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(last);
        self.prs_set_cursor(&view, next);
        Effect::None
    }

    /// Put the cursor on line `index` of `view`.
    fn prs_set_cursor(&mut self, view: &PrsView, index: usize) {
        let id = view.line_id(view.lines[index]);
        if let Some(tab) = self.tabs.active_prs_mut() {
            tab.cursor = Some(id);
            tab.cursor_index = index;
        }
    }

    /// Scroll the list so its painted lines start at `top`: the list
    /// follows its cursor (middle-biased, [`list_viewport_start`]), so the
    /// cursor moves to the line nearest it whose viewport starts there
    /// ([`cursor_for_viewport_top`]), as the wheel moves the cursor.
    pub(super) fn scroll_prs_list_to(&mut self, top: usize) {
        let Some(view) = self.sync_prs_view() else {
            return;
        };
        let n = view.lines.len();
        if n == 0 {
            return;
        }
        let height = usize::from(self.layout.prs_list_rows.height.max(1));
        let cursor = view.cursor.unwrap_or(0);
        let next = cursor_for_viewport_top(n, cursor, top, |c| list_viewport_start(n, c, height));
        self.prs_set_cursor(&view, next);
    }

    /// True when the detail pane is the one pane shown (the narrow
    /// layout after `Tab`): the move keys then scroll it. The wide layout
    /// keeps the keys on the list.
    fn prs_detail_focused(&self) -> bool {
        self.prs_narrow() && self.tabs.active_prs().is_some_and(|tab| tab.show_detail)
    }

    /// First detail line, as the last paint or scroll left it (the paint
    /// starts another PR at the top).
    pub(crate) fn prs_detail_scroll(&self) -> usize {
        self.tabs
            .active_prs()
            .and_then(|tab| tab.detail_scroll.as_ref())
            .map_or(0, |(_, top)| *top)
    }

    /// Scroll the detail so its painted lines start at `top`, clamped to
    /// its last page (`prs_detail_vbar.scroll_max` of the last paint).
    pub(super) fn scroll_prs_detail_to(&mut self, top: usize) {
        let top = top.min(usize::from(self.layout.prs_detail_vbar.scroll_max));
        if let Some((_, scroll)) = self
            .tabs
            .active_prs_mut()
            .and_then(|tab| tab.detail_scroll.as_mut())
        {
            *scroll = top;
        }
    }

    /// First detail line the paint shows for the PR at `url`, whose
    /// largest top is `scroll_max`: the kept line clamped to it (the
    /// detail lines shrank, say a detail lookup landed), or 0 for another
    /// PR than the one kept. Kept for `url`.
    pub(crate) fn keep_prs_detail_scroll(&mut self, url: &str, scroll_max: usize) -> usize {
        let Some(tab) = self.tabs.active_prs_mut() else {
            return 0;
        };
        let top = match &tab.detail_scroll {
            Some((kept, top)) if kept == url => (*top).min(scroll_max),
            _ => 0,
        };
        tab.detail_scroll = Some((url.to_string(), top));
        top
    }

    /// Scroll the detail `delta` lines (a wheel step or a move key).
    fn scroll_prs_detail_by(&mut self, delta: isize) {
        let top = self.prs_detail_scroll().saturating_add_signed(delta);
        self.scroll_prs_detail_to(top);
    }

    /// Run `action` on the focused PR, or say `no PR for this row`.
    fn prs_pr_action(&mut self, action: PrAction) -> Effect {
        match self.prs_focused_target() {
            Some(PrsTarget::Checkout(checkout)) => {
                self.run_pr_action_on_checkout(action, &checkout)
            }
            Some(PrsTarget::StackOnly { pr, listed_by }) => {
                self.prs_stack_only_action(action, &pr, &listed_by)
            }
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                Effect::None
            }
        }
    }

    /// Run `action` on the stack member `pr` that no visible checkout has,
    /// with no lookup: Open PR and the CI page open its URL and its
    /// [`PullRequest::ci_link`] (a CI link that is not a web URL says
    /// `no CI for <ref>`), Copy URL copies its URL, and Refresh looks up
    /// again each checkout of `listed_by` (the checkouts whose stack lists
    /// it), or says `PR lookup already running`.
    fn prs_stack_only_action(
        &mut self,
        action: PrAction,
        pr: &PullRequest,
        listed_by: &[PathBuf],
    ) -> Effect {
        let pr_ref = pr.ref_text();
        match action {
            PrAction::Open => Effect::OpenKnownPullRequest {
                pr_ref,
                url: pr.url.clone(),
                page: PrPage::Pr,
            },
            PrAction::OpenCi if !is_web_url(&pr.ci_link.url) => {
                self.status = StatusMessage::warn(no_ci_status(&pr_ref));
                Effect::None
            }
            PrAction::OpenCi => Effect::OpenKnownPullRequest {
                pr_ref,
                url: pr.ci_link.url.clone(),
                page: PrPage::Ci,
            },
            PrAction::CopyUrl => Effect::CopyClipboard {
                text: pr.url.clone(),
                announce: true,
            },
            PrAction::Refresh => {
                let effects: Vec<Effect> = listed_by
                    .iter()
                    .map(|checkout| self.refresh_pull_requests(Some(checkout)))
                    .filter(|effect| *effect != Effect::None)
                    .collect();
                if effects.is_empty() {
                    self.status = StatusMessage::warn(PR_LOOKUP_RUNNING);
                }
                single_or_batch(effects)
            }
        }
    }

    /// `r`: look every PR up again now ([`Self::refresh_pull_requests`]),
    /// or say that the lookups already run. The next title cycle runs the
    /// title command again for every key.
    fn prs_refresh(&mut self) -> Effect {
        // The titles are looked up again once these lookups go idle.
        self.force_pr_titles();
        let effect = self.refresh_pull_requests(None);
        if effect == Effect::None {
            self.status = StatusMessage::warn(PR_LOOKUP_RUNNING);
        }
        effect
    }

    /// `o`: switch to the Workspace tab, leave a commit drill, and put the
    /// tree cursor on the focused PR's first checkout (its parents
    /// unfold), with the tree focused. A stack-only PR says `<ref> is not
    /// checked out` and stays on the tab.
    fn prs_show_in_tree(&mut self) -> Effect {
        let checkout = match self.prs_focused_target() {
            Some(PrsTarget::Checkout(checkout)) => checkout,
            Some(PrsTarget::StackOnly { pr, .. }) => {
                self.status = StatusMessage::warn(not_checked_out_status(&pr.ref_text()));
                return Effect::None;
            }
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                return Effect::None;
            }
        };
        let repo = checkout.to_string_lossy().into_owned();
        let mut effects = vec![self.activate_tab(0)];
        if !self.drill.is_graph() {
            self.drill = DrillView::Graph;
            effects.push(Effect::DropCommitDiff);
        }
        if self.focus_checkout_row(&repo).is_none() {
            self.status = StatusMessage::warn(format!("{repo} is not in the tree"));
        } else {
            self.focus = FocusPane::Left;
            effects.push(Effect::LoadRightPane);
        }
        effects.retain(|effect| *effect != Effect::None);
        single_or_batch(effects)
    }

    /// `z`: fold the group under the cursor (the cursor moves to its
    /// header), or unfold it.
    fn prs_toggle_fold(&mut self) -> Effect {
        let Some(view) = self.sync_prs_view() else {
            return Effect::None;
        };
        let Some(index) = view.cursor else {
            return Effect::None;
        };
        let group = view.lines[index].group();
        let folded = view
            .lines
            .iter()
            .any(|line| matches!(line, PrsLine::Header { group: g, folded: true } if *g == group));
        let id = pr_group_id(&view.groups[group]);
        if let Some(tab) = self.tabs.active_prs_mut() {
            tab.folds.insert(id.clone(), !folded);
            if !folded {
                tab.cursor = Some(PrsLineId::Group(id));
            }
        }
        Effect::None
    }

    /// `Z`: fold every finished group, or unfold them all when every one
    /// is folded. Off the PRs tab it says [`PRS_TAB_ONLY`].
    pub(super) fn prs_toggle_finished(&mut self) -> Effect {
        let Some(view) = self.sync_prs_view() else {
            self.status = StatusMessage::warn(PRS_TAB_ONLY);
            return Effect::None;
        };
        let finished: Vec<(PrGroupId, bool)> = view
            .lines
            .iter()
            .filter_map(|line| match *line {
                PrsLine::Header { group, folded } if view.groups[group].is_finished() => {
                    Some((pr_group_id(&view.groups[group]), folded))
                }
                _ => None,
            })
            .collect();
        let fold = finished.iter().any(|(_, folded)| !folded);
        if let Some(tab) = self.tabs.active_prs_mut() {
            for (id, _) in finished {
                tab.folds.insert(id, fold);
            }
        }
        Effect::None
    }

    /// `Tab`, and Esc on the detail: in the narrow layout, show the detail
    /// instead of the list, or the list again. The wide layout shows both.
    fn prs_toggle_detail(&mut self) -> Effect {
        let narrow = self.prs_narrow();
        if let Some(tab) = self.tabs.active_prs_mut() {
            tab.show_detail = narrow && !tab.show_detail;
        }
        Effect::None
    }

    /// `gh`: pin the popover of the focused PR's ref, where it painted. A
    /// stack-only PR has no popover (it reads a checkout) and says `<ref>
    /// is not checked out`, as `o` does; its detail pane shows the PR.
    fn prs_pin_focused(&mut self) -> Effect {
        let checkout = match self.prs_focused_target() {
            Some(PrsTarget::Checkout(checkout)) => checkout,
            Some(PrsTarget::StackOnly { pr, .. }) => {
                self.status = StatusMessage::warn(not_checked_out_status(&pr.ref_text()));
                return Effect::None;
            }
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                return Effect::None;
            }
        };
        let cursor_y = self.tabs.active_prs().and_then(|tab| {
            let offset = tab.cursor_index.checked_sub(tab.scroll)?;
            Some(
                self.layout
                    .prs_list_rows
                    .y
                    .saturating_add(u16::try_from(offset).ok()?),
            )
        });
        let hit = self
            .layout
            .icon_hits
            .iter()
            .find(|hit| {
                Some(hit.y) == cursor_y && hit.target == IconTarget::PullRequest(checkout.clone())
            })
            .cloned();
        match hit {
            Some(hit) => self.pin_icon(&hit),
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                Effect::None
            }
        }
    }

    /// Wheel over the list moves the cursor; over the detail it scrolls
    /// the detail (the list cursor stays); elsewhere it does nothing.
    fn prs_wheel(&mut self, col: u16, row: u16, delta: i32, horizontal: bool) -> Effect {
        if !self.mouse_enabled || horizontal {
            return Effect::None;
        }
        let at = Position::new(col, row);
        if self.layout.prs_detail_rows.contains(at) {
            self.scroll_prs_detail_by(delta as isize);
            return Effect::None;
        }
        if !self.layout.prs_list_rows.contains(at) {
            return Effect::None;
        }
        self.prs_move(delta as isize)
    }

    /// Left press on the PRs tab ([`Self::click_at`] after the tab strip):
    /// a press on the list's or the detail's vertical scrollbar grabs it
    /// or jumps ([`Self::press_vscrollbar`], no row picked); a row takes
    /// the cursor; a group header takes it and folds or unfolds (a
    /// double-click does not undo the first toggle). A press in either
    /// pane off a bar arms a text selection.
    pub(super) fn prs_click(&mut self, col: u16, row: u16) -> Effect {
        self.drag = SplitDrag::None;
        self.pad_press = None;
        // The tab has no divider: only its own bars are drag handles.
        match self.split_hit_at(col, row) {
            SplitHit::VThumb(bar) => return self.press_vscrollbar(bar, true, row),
            SplitHit::VTrack(bar) => return self.press_vscrollbar(bar, false, row),
            _ => {}
        }
        let at = Position::new(col, row);
        if self.layout.prs_detail_rows.contains(at) {
            self.text_selection = TextSelection::arm(self.layout.prs_detail, col, row);
            return Effect::None;
        }
        if !self.layout.prs_list_rows.contains(at) {
            self.text_selection = TextSelection::arm(self.layout.prs_list, col, row);
            return Effect::None;
        }
        self.text_selection = TextSelection::arm(self.layout.prs_list, col, row);
        // The shared row double-click rule, timed by the presses' arrival.
        let double = self.note_row_press(col, row);
        let Some(view) = self.sync_prs_view() else {
            return Effect::None;
        };
        let scroll = self.tabs.active_prs().map_or(0, |tab| tab.scroll);
        let index = scroll + usize::from(row - self.layout.prs_list_rows.y);
        let Some(line) = view.lines.get(index).copied() else {
            return Effect::None;
        };
        self.prs_set_cursor(&view, index);
        if matches!(line, PrsLine::Header { .. }) && !double {
            return self.prs_toggle_fold();
        }
        Effect::None
    }

    /// The group title of key `key` from the last title fill.
    pub(crate) fn pr_group_title(&self, key: &str) -> Option<&str> {
        self.pr_titles.shown.get(key)
    }

    /// Start a title cycle at `now`: the PRs tab opened, or the PR lookups
    /// went idle. Returns the fill when one may start
    /// ([`Self::pr_title_fill`]).
    pub(crate) fn new_pr_title_cycle(&mut self, now: Instant) -> Effect {
        self.pr_titles.due = true;
        self.pr_title_fill(now)
    }

    /// The title fill a waiting cycle asks for at `now`: only with a title
    /// command, the PRs tab open, and no fill queued or running, so one
    /// fill runs at a time. It runs only when the listed ticket keys
    /// changed since the last fill, the last fill landed more than one PR
    /// poll period ago ([`Self::pr_poll_interval_ms`]; never with polling
    /// off), or `r` forced it; any other cycle passes, so lookups that land
    /// one by one do not run the command for every key each time. A cycle
    /// with no ticket group clears the titles.
    fn pr_title_fill(&mut self, now: Instant) -> Effect {
        let Some(cmd) = self.pr_titles.command.clone() else {
            return Effect::None;
        };
        if !self.pr_titles.due || self.pr_titles.running || self.tabs.find_prs().is_none() {
            return Effect::None;
        }
        self.pr_titles.due = false;
        let groups: Vec<PrGroup> = group_prs(self.pr_entries().entries)
            .into_iter()
            .filter(PrGroup::wants_title)
            .collect();
        if groups.is_empty() {
            self.pr_titles.shown = TitleCache::default();
            self.pr_titles.filled_keys.clear();
            return Effect::None;
        }
        let mut keys: Vec<String> = groups.iter().map(|group| group.key.clone()).collect();
        keys.sort();
        let period = Duration::from_millis(self.pr_poll_interval_ms());
        let old = self
            .pr_titles
            .filled_at
            .is_none_or(|at| !period.is_zero() && now.saturating_duration_since(at) > period);
        if !self.pr_titles.forced && keys == self.pr_titles.filled_keys && !old {
            return Effect::None;
        }
        self.pr_titles.forced = false;
        self.pr_titles.filled_keys = keys;
        self.pr_titles.running = true;
        Effect::FillPrTitles { cmd, groups }
    }

    /// Land a title fill at `now`. A fill the quit cut short (`cancelled`)
    /// changes nothing on screen. Otherwise its titles merge over the ones
    /// on screen ([`TitleCache::merged`] over the ticket keys listed now):
    /// a key whose run gave no title keeps its title, and a key no longer
    /// listed drops. A cycle that started while it ran may ask for the
    /// next fill.
    pub(crate) fn apply_pr_titles(
        &mut self,
        titles: TitleCache,
        cancelled: bool,
        now: Instant,
    ) -> Effect {
        self.pr_titles.running = false;
        if cancelled {
            return Effect::None;
        }
        let groups = group_prs(self.pr_entries().entries);
        let keys = groups
            .iter()
            .filter(|group| group.wants_title())
            .map(|group| group.key.as_str());
        self.pr_titles.shown = self.pr_titles.shown.merged(&titles, keys);
        self.pr_titles.filled_at = Some(now);
        self.pr_title_fill(now)
    }

    /// Make the next title cycle run the command for every key (`r`).
    fn force_pr_titles(&mut self) {
        self.pr_titles.forced = true;
    }
}

/// PRs of a made-up workspace for the PRs tab tests (state and paint).
#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;

    use super::super::super::pr_status::{ChecksSummary, PrState};
    use super::super::super::pull_request::{PrLookup, PrStack, PullRequest};
    use super::super::AppState;
    use crate::snapshot::{CheckoutKind, RepoSnapshot, SyncStatus};
    use crate::testutil::{app_state_ascii, repo_snapshot};

    /// GitHub remote of `acme/app`.
    pub(crate) const GH_APP: &str = "git@github.com:acme/app.git";
    /// GitHub remote of `acme/lib`.
    pub(crate) const GH_LIB: &str = "git@github.com:acme/lib.git";
    /// GitLab remote of `acme/services/api`.
    pub(crate) const GL_API: &str = "git@gitlab.com:acme/services/api.git";

    /// A pushed primary checkout `name` on `branch`.
    pub(crate) fn checkout(name: &str, branch: &str) -> RepoSnapshot {
        RepoSnapshot {
            branch: branch.into(),
            sync_status: SyncStatus::UpToDate,
            head: "abc".into(),
            ..repo_snapshot(name)
        }
    }

    /// A linked worktree `name` of `primary` on `branch`.
    pub(crate) fn worktree(name: &str, primary: &str, branch: &str) -> RepoSnapshot {
        RepoSnapshot {
            checkout_kind: CheckoutKind::Linked,
            primary_repo: Some(primary.into()),
            ..checkout(name, branch)
        }
    }

    /// PR `number` of the repository of `remote` in `state`, with its
    /// title and head and base branches, and checks that fit the state
    /// (CI failed: one failed of six; CI running: one running; ready and
    /// merged: all passed; else none).
    pub(crate) fn pr(
        remote: &str,
        number: u64,
        state: PrState,
        title: &str,
        head: &str,
        base: &str,
    ) -> PullRequest {
        let url = if remote == GL_API {
            format!("https://gitlab.com/acme/services/api/-/merge_requests/{number}")
        } else {
            let repo = remote
                .trim_start_matches("git@github.com:")
                .trim_end_matches(".git");
            format!("https://github.com/{repo}/pull/{number}")
        };
        let checks = match state {
            PrState::CiFailed => ChecksSummary {
                pass: 5,
                fail: 1,
                pending: 0,
            },
            PrState::CiRunning => ChecksSummary {
                pass: 0,
                fail: 0,
                pending: 1,
            },
            PrState::Ready | PrState::Merged => ChecksSummary {
                pass: 6,
                fail: 0,
                pending: 0,
            },
            PrState::Draft | PrState::InReview => ChecksSummary::default(),
        };
        PullRequest {
            title: title.into(),
            head: head.into(),
            base: base.into(),
            checks,
            ..PullRequest::fixture(number, &url, state)
        }
    }

    /// State over `repos` with every lookup queued.
    pub(crate) fn app_over(repos: &[RepoSnapshot], ascii: bool) -> AppState {
        let mut app = app_state_ascii(repos, ascii);
        assert_eq!(app.due_pr_lookups().len(), repos.len());
        app
    }

    /// Land `pr` for checkout `name` on `branch` at `remote`.
    pub(crate) fn land(
        app: &mut AppState,
        name: &str,
        branch: &str,
        remote: &str,
        pr: PullRequest,
    ) {
        assert!(app.apply_pr_lookup(
            Path::new(name),
            branch,
            Some(remote.into()),
            PrLookup::Found(pr),
        ));
    }

    /// Land member `this` of `chain` for checkout `name` at `remote`, with
    /// its stack walk: the whole chain.
    pub(crate) fn land_with_stack(
        app: &mut AppState,
        name: &str,
        remote: &str,
        chain: &[PullRequest],
        this: usize,
    ) {
        let pr = &chain[this];
        assert!(app.apply_pr_badge(
            Path::new(name),
            &pr.head,
            Some(remote.into()),
            PrLookup::Found(pr.clone()),
            Ok(Some(PrStack {
                members: chain.to_vec(),
                this,
            })),
        ));
    }

    /// Two stacks whose ends no checkout has. GitHub `acme/app`: `#6`
    /// (parent, not checked out) → `#7` (`app`) → `#8` (`app-wt`) → `#9`
    /// (draft child, not checked out). GitLab `acme/services/api`: `!4`
    /// (not checked out) → `!5` (`api`) → `!6` (not checked out). No title
    /// has a ticket key, so every PR is under `other`.
    pub(crate) fn stack_workspace(ascii: bool) -> AppState {
        let repos = [
            checkout("api", "feature/rate-limit"),
            checkout("app", "feature/auth-refresh"),
            worktree("app-wt", "app", "feature/login-page"),
        ];
        let mut app = app_over(&repos, ascii);
        let github = [
            pr(
                GH_APP,
                6,
                PrState::InReview,
                "Store the tokens",
                "feature/token-store",
                "main",
            ),
            pr(
                GH_APP,
                7,
                PrState::Ready,
                "Refresh the auth token",
                "feature/auth-refresh",
                "feature/token-store",
            ),
            pr(
                GH_APP,
                8,
                PrState::CiFailed,
                "Add the login page",
                "feature/login-page",
                "feature/auth-refresh",
            ),
            pr(
                GH_APP,
                9,
                PrState::Draft,
                "Reword the login copy",
                "feature/login-copy",
                "feature/login-page",
            ),
        ];
        land_with_stack(&mut app, "app", GH_APP, &github, 1);
        land_with_stack(&mut app, "app-wt", GH_APP, &github, 2);
        let gitlab = [
            pr(
                GL_API,
                4,
                PrState::InReview,
                "Store the quotas",
                "feature/quota-store",
                "main",
            ),
            pr(
                GL_API,
                5,
                PrState::CiRunning,
                "Rate limit the API",
                "feature/rate-limit",
                "feature/quota-store",
            ),
            pr(
                GL_API,
                6,
                PrState::Draft,
                "Document the rate limit",
                "feature/rate-limit-docs",
                "feature/rate-limit",
            ),
        ];
        land_with_stack(&mut app, "api", GL_API, &gitlab, 1);
        app
    }

    /// Run `cmd` as the PR title command (`prTitleCommand`).
    pub(crate) fn set_title_command(app: &mut AppState, cmd: &Path) {
        app.pr_titles.command = Some(cmd.to_path_buf());
    }

    /// An executable script in a new temp dir that prints `Title <KEY>`;
    /// the caller removes its dir.
    #[cfg(unix)]
    pub(crate) fn title_script() -> PathBuf {
        script_with("echo \"Title $1\"")
    }

    /// An executable `/bin/sh` script with `body` in a new temp dir; the
    /// caller removes its dir.
    #[cfg(unix)]
    pub(crate) fn script_with(body: &str) -> PathBuf {
        let dir = crate::testutil::unique_dir("ws-prs-title");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("title");
        crate::testutil::write_sh_script(&path, body);
        path
    }

    /// The made-up workspace: ticket `ABC-123` with a failed PR (`app`,
    /// also checked out as `app-copy`) and a ready PR stacked on it
    /// (`app-ui`); ticket `ABC-125` with a running GitLab MR (`api`);
    /// ticket `ABC-120` merged (`lib`); and a keyless PR (`app-docs`)
    /// under `other`.
    pub(crate) fn workspace(ascii: bool) -> AppState {
        let repos = [
            checkout("api", "feature/ABC-125-limits"),
            checkout("app", "feature/ABC-123-retry"),
            checkout("app-copy", "feature/ABC-123-retry"),
            worktree("app-docs", "app", "docs/readme"),
            worktree("app-ui", "app", "feature/ABC-123-retry-ui"),
            checkout("lib", "feature/ABC-120-cache"),
        ];
        let mut app = app_over(&repos, ascii);
        let retry = pr(
            GH_APP,
            318,
            PrState::CiFailed,
            "feat(ABC-123): Retry failed uploads with backoff",
            "feature/ABC-123-retry",
            "main",
        );
        land(
            &mut app,
            "app",
            "feature/ABC-123-retry",
            GH_APP,
            retry.clone(),
        );
        land(&mut app, "app-copy", "feature/ABC-123-retry", GH_APP, retry);
        let ui = pr(
            GH_APP,
            319,
            PrState::Ready,
            "feat(ABC-123): Show the retry state",
            "feature/ABC-123-retry-ui",
            "feature/ABC-123-retry",
        );
        land(&mut app, "app-ui", "feature/ABC-123-retry-ui", GH_APP, ui);
        let docs = pr(
            GH_APP,
            320,
            PrState::InReview,
            "Update the README",
            "docs/readme",
            "main",
        );
        land(&mut app, "app-docs", "docs/readme", GH_APP, docs);
        let limits = pr(
            GL_API,
            12,
            PrState::CiRunning,
            "ABC-125: Rate limit the upload API",
            "feature/ABC-125-limits",
            "main",
        );
        land(&mut app, "api", "feature/ABC-125-limits", GL_API, limits);
        let cache = pr(
            GH_LIB,
            40,
            PrState::Merged,
            "feat(ABC-120): Cache the totals",
            "feature/ABC-120-cache",
            "main",
        );
        land(&mut app, "lib", "feature/ABC-120-cache", GH_LIB, cache);
        app
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::super::super::command_palette::{command_for, go_to_line_command};
    use super::super::super::gates::ListFocusTarget;
    use super::super::super::pr_groups::GroupKind;
    use super::super::super::pull_request::{PrLookup, PrPage};
    use super::fixtures::*;
    use super::*;

    /// The list as text: a header is its key, a row its PR ref.
    fn lines(app: &AppState) -> Vec<String> {
        let view = app.prs_view().expect("PRs tab");
        view.lines
            .iter()
            .map(|line| match *line {
                PrsLine::Header { group, .. } => view.groups[group].key.clone(),
                PrsLine::Row { group, row } => {
                    format!("  {}", view.groups[group].rows[row].pr.ref_text())
                }
            })
            .collect()
    }

    /// The cursor line as [`lines`] writes it.
    fn cursor(app: &mut AppState) -> String {
        let view = app.sync_prs_view().expect("PRs tab");
        lines(app)[view.cursor.expect("a cursor")].clone()
    }

    /// Move the cursor onto the line `want` ([`lines`]).
    fn cursor_to(app: &mut AppState, want: &str) {
        let index = lines(app)
            .iter()
            .position(|line| line == want)
            .unwrap_or_else(|| panic!("no line {want}"));
        app.dispatch(Action::MoveToStart);
        app.dispatch(Action::Move(index as i32));
        assert_eq!(cursor(app), want);
    }

    fn opened(ascii: bool) -> AppState {
        let mut app = workspace(ascii);
        app.dispatch(Action::OpenPrsTab);
        assert!(app.is_prs_tab());
        app
    }

    #[test]
    fn gp_opens_one_prs_tab_a_second_gp_focuses_it_and_ctrl_w_closes_it() {
        let mut app = workspace(false);
        let Effect::Batch(details) = app.dispatch(Action::OpenPrsTab) else {
            panic!("the first open asks for the listed PRs' details");
        };
        let mut asked: Vec<PathBuf> = details
            .iter()
            .map(|effect| match effect {
                Effect::LookupPullRequestDetail { repo, .. } => repo.clone(),
                other => panic!("not a detail fetch: {other:?}"),
            })
            .collect();
        asked.sort();
        let want: Vec<PathBuf> = ["api", "app", "app-docs", "app-ui", "lib"]
            .map(PathBuf::from)
            .to_vec();
        assert_eq!(asked, want, "one fetch per distinct PR: not app-copy");
        assert_eq!(app.tab_labels(), ["Workspace", "PRs 5"]);
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        app.dispatch(Action::JumpToTab(1));
        assert!(app.tabs.is_workspace());
        app.dispatch(Action::OpenPrsTab);
        assert!(app.is_prs_tab());
        assert_eq!(app.tabs.len(), 2, "one instance");
        assert_eq!(app.dispatch(Action::OpenPrsTab), Effect::None, "focused");
        app.dispatch(Action::CloseTab);
        assert!(app.tabs.is_workspace());
        assert_eq!(app.tabs.len(), 1);
    }

    /// `gp` runs on every tab: from an Explorer tab and from a compare tab
    /// it opens or focuses the PRs tab, and jumping back finds each tab's
    /// parked session as it was, the Workspace one included.
    #[test]
    fn gp_from_an_explorer_or_compare_tab_keeps_their_parked_sessions() {
        use super::super::explorer_tab::ExplorerTarget;
        let mut app = workspace(false);
        // Workspace session: the diff pane focused on its third row.
        app.focus = FocusPane::Right;
        app.diff_cursor = 2;

        app.open_explorer_tab("app".into(), ExplorerTarget::Root);
        assert!(app.is_explorer_tab());
        app.dispatch(Action::OpenPrsTab);
        assert!(app.is_prs_tab(), "gp from the Explorer tab");

        app.dispatch(Action::JumpToTab(2));
        assert_eq!(
            app.tabs.active_explorer().map(|tab| tab.checkout.as_str()),
            Some("app")
        );
        app.dispatch(Action::JumpToTab(1));
        app.focus = FocusPane::Right;
        app.diff_cursor = 2;
        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        assert!(app.tabs.active_compare().is_some());
        // Compare session: the diff pane focused on its fifth row.
        app.focus = FocusPane::Right;
        app.diff_cursor = 4;
        assert_eq!(app.dispatch(Action::OpenPrsTab), Effect::None, "focused");
        assert!(app.is_prs_tab(), "gp from the compare tab");
        assert_eq!(app.tabs.len(), 4, "one PRs tab");

        app.dispatch(Action::JumpToTab(4));
        let tab = app.tabs.active_compare().expect("back on the compare tab");
        assert_eq!(
            (tab.base_ref.as_str(), app.focus, app.diff_cursor),
            ("main", FocusPane::Right, 4),
            "the compare session"
        );
        app.dispatch(Action::OpenPrsTab);
        app.dispatch(Action::JumpToTab(1));
        assert!(app.tabs.is_workspace());
        assert_eq!(
            (app.focus, app.diff_cursor),
            (FocusPane::Right, 2),
            "the Workspace session"
        );
    }

    #[test]
    fn the_tab_label_counts_distinct_prs() {
        let retry = || {
            pr(
                GH_APP,
                318,
                PrState::CiFailed,
                "Retry",
                "feature/ABC-123-retry",
                "main",
            )
        };
        let mut app = app_over(
            &[
                checkout("app", "feature/ABC-123-retry"),
                checkout("app-copy", "feature/ABC-123-retry"),
            ],
            false,
        );
        app.dispatch(Action::OpenPrsTab);
        assert_eq!(app.tab_labels()[1], "PRs 0");
        land(&mut app, "app", "feature/ABC-123-retry", GH_APP, retry());
        land(
            &mut app,
            "app-copy",
            "feature/ABC-123-retry",
            GH_APP,
            retry(),
        );
        assert_eq!(
            app.tab_labels()[1],
            "PRs 1",
            "two checkouts, one PR; the tab name stays PRs"
        );
        assert_eq!(lines(&app), ["ABC-123", "  #318"], "one row");
        let view = app.prs_view().unwrap();
        let row = &view.groups[0].rows[0];
        assert_eq!(
            view.checkouts_of(row),
            [PathBuf::from("app"), PathBuf::from("app-copy")],
            "the detail lists both checkouts"
        );
    }

    #[test]
    fn groups_and_rows_list_in_order_on_both_forges() {
        let app = opened(false);
        assert_eq!(
            lines(&app),
            [
                "ABC-123", "  #318", "  #319", "ABC-125", "  !12", "ABC-120", "  #40", "other",
                "  #320"
            ],
            "CI failed first, then waiting, then finished, then other"
        );
        let view = app.prs_view().unwrap();
        let kinds: Vec<GroupKind> = view.groups.iter().map(|group| group.kind).collect();
        assert_eq!(
            kinds,
            [
                GroupKind::Ticket,
                GroupKind::Ticket,
                GroupKind::Ticket,
                GroupKind::Other
            ]
        );
        let ui = &view.groups[0].rows[1];
        assert_eq!(ui.repo, "acme/app", "the forge path names the repo");
        assert_eq!(ui.title, "Show the retry state", "key and type stripped");
        assert_eq!(
            ui.stacked_on.as_ref().map(|base| base.pr_ref.as_str()),
            Some("#318")
        );
        assert_eq!(view.groups[1].rows[0].repo, "acme/services/api");
    }

    #[test]
    fn the_configured_default_branch_is_never_a_stack_base() {
        let mut app = workspace(false);
        let mut snapshot = app.snapshot.clone();
        for row in &mut snapshot.repos {
            if row.repo == "app-ui" {
                row.default_branch_override = Some("feature/ABC-123-retry".into());
            }
        }
        app.snapshot = snapshot;
        app.dispatch(Action::OpenPrsTab);
        let view = app.prs_view().unwrap();
        assert_eq!(view.groups[0].rows[1].pr.ref_text(), "#319");
        assert_eq!(
            view.groups[0].rows[1].stacked_on, None,
            "the row's configured default branch reaches the stack rule"
        );

        // With no override, the repository's known default branch counts
        // too, as for the stack walk.
        let mut app = workspace(false);
        let mut snapshot = app.snapshot.clone();
        for row in &mut snapshot.repos {
            if row.repo == "app-ui" {
                row.default_branch = Some("feature/ABC-123-retry".into());
            }
        }
        app.snapshot = snapshot;
        app.dispatch(Action::OpenPrsTab);
        let view = app.prs_view().unwrap();
        assert_eq!(view.groups[0].rows[1].pr.ref_text(), "#319");
        assert_eq!(view.groups[0].rows[1].stacked_on, None, "known default");
    }

    /// One listed row: its ref, whether a checkout has it, and its stack
    /// position `(position, length)`.
    type StackRow = (String, bool, Option<(usize, usize)>);

    /// The list as one [`StackRow`] per row.
    fn stack_rows(app: &AppState) -> Vec<StackRow> {
        let view = app.prs_view().expect("PRs tab");
        view.groups
            .iter()
            .flat_map(|group| &group.rows)
            .map(|row| (row.pr.ref_text(), row.checked_out, row.stack_position))
            .collect()
    }

    #[test]
    fn stack_members_no_checkout_has_list_in_chain_order_on_both_forges() {
        let mut app = stack_workspace(false);
        let Effect::Batch(details) = app.dispatch(Action::OpenPrsTab) else {
            panic!("the open asks for the checked-out PRs' details");
        };
        let mut asked: Vec<PathBuf> = details
            .iter()
            .map(|effect| match effect {
                Effect::LookupPullRequestDetail { repo, .. } => repo.clone(),
                other => panic!("not a detail fetch: {other:?}"),
            })
            .collect();
        asked.sort();
        assert_eq!(
            asked,
            ["api", "app", "app-wt"].map(PathBuf::from).to_vec(),
            "no detail fetch for a stack-only row"
        );
        assert_eq!(
            lines(&app),
            ["other", "  #6", "  #7", "  #8", "  #9", "  !4", "  !5", "  !6"]
        );
        assert_eq!(
            stack_rows(&app),
            vec![
                ("#6".into(), false, Some((1, 4))),
                ("#7".into(), true, Some((2, 4))),
                ("#8".into(), true, Some((3, 4))),
                ("#9".into(), false, Some((4, 4))),
                ("!4".into(), false, Some((1, 3))),
                ("!5".into(), true, Some((2, 3))),
                ("!6".into(), false, Some((3, 3))),
            ]
        );
        assert_eq!(
            app.tab_labels(),
            ["Workspace", "PRs 7"],
            "the label counts every listed row"
        );
        assert_eq!(app.pr_totals().prs, 3, "the tree total: checked out only");

        // `#6` is listed by both checkouts' stacks; `!6` by `api` only.
        let view = app.prs_view().unwrap();
        let row = |pr_ref: &str| {
            view.groups[0]
                .rows
                .iter()
                .find(|row| row.pr.ref_text() == pr_ref)
                .unwrap()
        };
        assert_eq!(
            view.listed_by(row("#6")),
            ["app", "app-wt"].map(PathBuf::from)
        );
        assert_eq!(view.listed_by(row("!6")), [PathBuf::from("api")]);
        assert!(view.checkouts_of(row("#6")).is_empty());
        assert!(view.listed_by(row("#7")).is_empty());
    }

    #[test]
    fn keys_on_a_stack_only_row_act_on_its_own_pr() {
        let mut app = stack_workspace(false);
        app.dispatch(Action::OpenPrsTab);
        for (line, url, listed_by) in [
            (
                "  #6",
                "https://github.com/acme/app/pull/6",
                &["app", "app-wt"][..],
            ),
            (
                "  !6",
                "https://gitlab.com/acme/services/api/-/merge_requests/6",
                &["api"][..],
            ),
        ] {
            cursor_to(&mut app, line);
            let pr_ref = line.trim();
            let ci = app
                .prs_view()
                .unwrap()
                .focused()
                .unwrap()
                .pr
                .ci_link
                .url
                .clone();
            let open = Effect::OpenKnownPullRequest {
                pr_ref: pr_ref.into(),
                url: url.into(),
                page: PrPage::Pr,
            };
            assert_eq!(app.dispatch(Action::NavEnter), open, "{line}: Enter");
            assert_eq!(app.dispatch(Action::OpenPullRequest), open, "{line}: gx");
            assert_eq!(
                app.dispatch(Action::OpenPullRequestCi),
                Effect::OpenKnownPullRequest {
                    pr_ref: pr_ref.into(),
                    url: ci,
                    page: PrPage::Ci,
                },
                "{line}: gc"
            );
            assert_eq!(
                app.dispatch(Action::ExportComments),
                Effect::CopyClipboard {
                    text: url.into(),
                    announce: true,
                },
                "{line}: y"
            );

            app.status.clear();
            assert_eq!(app.dispatch(Action::GraphFocusBranches), Effect::None);
            assert_eq!(&*app.status, format!("{pr_ref} is not checked out"));
            assert!(app.is_prs_tab(), "{line}: o stays on the tab");

            // `gh` has no popover to pin (the row paints no PR icon): it
            // says what `o` says.
            app.status.clear();
            assert_eq!(app.dispatch(Action::PopoverOpenFocused), Effect::None);
            assert_eq!(
                &*app.status,
                format!("{pr_ref} is not checked out"),
                "{line}: gh"
            );
            assert!(!app.popover_pinned(), "{line}: gh pins nothing");

            // Refresh this PR looks up each checkout whose stack lists it.
            let refreshed = lookup_repos(app.dispatch(Action::RefreshPullRequest));
            assert_eq!(
                refreshed,
                listed_by
                    .iter()
                    .map(|name| PathBuf::from(*name))
                    .collect::<Vec<_>>(),
                "{line}"
            );
            app.status.clear();
            assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
            assert_eq!(&*app.status, PR_LOOKUP_RUNNING, "{line}: in flight");
        }

        // A CI link that is not a web URL says so, as on a checked-out row.
        let mut app = stack_workspace(false);
        app.dispatch(Action::OpenPrsTab);
        cursor_to(&mut app, "  #9");
        let chain: Vec<PullRequest> = app.pr_stack(Path::new("app")).unwrap().members.clone();
        let mut no_ci = chain.clone();
        no_ci[3].ci_link.url = String::new();
        land_with_stack(&mut app, "app", GH_APP, &no_ci, 1);
        land_with_stack(&mut app, "app-wt", GH_APP, &no_ci, 2);
        app.status.clear();
        assert_eq!(app.dispatch(Action::OpenPullRequestCi), Effect::None);
        assert_eq!(&*app.status, "no CI for #9");

        // Plain `r` still looks up every visible checkout, from any row.
        let mut app = stack_workspace(false);
        app.dispatch(Action::OpenPrsTab);
        cursor_to(&mut app, "  #6");
        assert_eq!(
            lookup_repos(app.dispatch(Action::Refresh)),
            ["api", "app", "app-wt"].map(PathBuf::from),
            "r on a stack-only row"
        );
    }

    /// The checkouts `effect` looks up, sorted: one
    /// `Effect::LookupPullRequests`, or a batch of them.
    fn lookup_repos(effect: Effect) -> Vec<PathBuf> {
        let mut repos: Vec<PathBuf> = match effect {
            Effect::LookupPullRequests { targets } => vec![targets],
            Effect::Batch(effects) => effects
                .into_iter()
                .map(|effect| match effect {
                    Effect::LookupPullRequests { targets } => targets,
                    other => panic!("not a lookup: {other:?}"),
                })
                .collect(),
            other => panic!("no lookup: {other:?}"),
        }
        .into_iter()
        .flatten()
        .map(|(repo, _)| repo)
        .collect();
        repos.sort();
        repos
    }

    #[test]
    fn palette_pr_rows_run_on_a_stack_only_row() {
        let mut app = stack_workspace(false);
        app.dispatch(Action::OpenPrsTab);
        cursor_to(&mut app, "  #6");
        for action in [
            Action::OpenPullRequest,
            Action::OpenPullRequestCi,
            Action::CopyPullRequestUrl,
            Action::RefreshPullRequest,
        ] {
            assert_eq!(
                app.palette_disabled_reason(command_for(&action).expect("a palette row")),
                None,
                "{action:?}"
            );
        }
    }

    /// The header double-click window is the gap between the two presses'
    /// arrival, not between the loop handling them.
    #[test]
    fn header_double_click_reads_the_press_arrival_instant() {
        let press = |app: &mut AppState, at: Instant, row: u16| {
            app.begin_input(at);
            app.dispatch(Action::Click { col: 4, row });
            app.end_input();
        };
        let setup = || {
            let mut app = opened(false);
            app.layout.prs_list_rows = ratatui::layout::Rect::new(0, 2, 60, 30);
            let header = lines(&app)
                .iter()
                .position(|line| line == "ABC-123")
                .expect("header line");
            (app, 2 + header as u16)
        };
        let folded = |app: &AppState| !lines(app).contains(&"  #318".to_string());

        // Two presses that arrived 399 ms apart, long ago: one toggle.
        let (mut app, row) = setup();
        let first = crate::testutil::instant_ago(Duration::from_secs(5));
        press(&mut app, first, row);
        assert!(folded(&app), "the first press folds the header");
        press(&mut app, first + Duration::from_millis(399), row);
        assert!(folded(&app), "a double-click does not undo the fold");

        // Two presses 401 ms apart, handled back to back: two toggles.
        let (mut app, row) = setup();
        let first = Instant::now();
        press(&mut app, first, row);
        press(&mut app, first + Duration::from_millis(401), row);
        assert!(!folded(&app), "the second single click unfolds again");
    }

    #[test]
    fn the_cursor_follows_its_line_across_folds() {
        let mut app = opened(false);
        cursor_to(&mut app, "  #319");
        assert_eq!(app.dispatch(Action::FoldToggle), Effect::None);
        assert_eq!(
            cursor(&mut app),
            "ABC-123",
            "folding moves it to the header"
        );
        assert!(!lines(&app).contains(&"  #318".to_string()));
        app.dispatch(Action::Move(1));
        assert_eq!(cursor(&mut app), "ABC-125");
        app.dispatch(Action::Move(-1));
        app.dispatch(Action::FoldToggle);
        assert_eq!(cursor(&mut app), "ABC-123", "unfolding keeps the header");
        app.dispatch(Action::Move(1));
        assert_eq!(cursor(&mut app), "  #318");
        // `zz` toggles twice: the group ends where it was.
        app.dispatch(Action::FoldToggle);
        app.dispatch(Action::FoldToggleSubtree);
        assert!(lines(&app).contains(&"  #318".to_string()));
        // `Z` folds the finished groups; the cursor on one of its rows
        // goes to its header, and comes back when it unfolds.
        cursor_to(&mut app, "  #40");
        app.dispatch(Action::ToggleFinishedPrGroups);
        assert_eq!(cursor(&mut app), "ABC-120");
        assert!(!lines(&app).contains(&"  #40".to_string()));
        assert!(lines(&app).contains(&"  #320".to_string()), "other stays");
        app.dispatch(Action::ToggleFinishedPrGroups);
        assert!(lines(&app).contains(&"  #40".to_string()), "Z unfolds all");
        // A gone line keeps the cursor's index.
        app.dispatch(Action::MoveToEnd);
        assert_eq!(cursor(&mut app), "  #320");
        land(
            &mut app,
            "app-docs",
            "docs/readme",
            GH_APP,
            pr(
                GH_APP,
                320,
                PrState::Merged,
                "Update the README",
                "docs/readme",
                "main",
            ),
        );
        assert_eq!(cursor(&mut app), "  #320", "the PR still lists");
    }

    #[test]
    fn z_upper_off_the_prs_tab_says_where_it_works() {
        let mut app = workspace(false);
        assert_eq!(app.dispatch(Action::ToggleFinishedPrGroups), Effect::None);
        assert_eq!(&*app.status, PRS_TAB_ONLY);
    }

    /// `n` PRs in `n` checkouts of `acme/app`: ticket `ABC-<i>`, merged
    /// when `i` is in `merged`.
    fn many(n: u64, merged: &[u64]) -> AppState {
        let repos: Vec<RepoSnapshotOf> = (1..=n)
            .map(|i| checkout(&format!("app{i}"), &format!("feature/ABC-{i}-x")))
            .collect();
        let mut app = app_over(&repos, false);
        for i in 1..=n {
            let state = if merged.contains(&i) {
                PrState::Merged
            } else {
                PrState::InReview
            };
            let branch = format!("feature/ABC-{i}-x");
            let remote = format!("git@github.com:acme/app{i}.git");
            land(
                &mut app,
                &format!("app{i}"),
                &branch,
                &remote,
                pr(&remote, i, state, "Change", &branch, "main"),
            );
        }
        app.dispatch(Action::OpenPrsTab);
        app
    }

    type RepoSnapshotOf = crate::snapshot::RepoSnapshot;

    #[test]
    fn finished_groups_start_folded_when_more_than_12_distinct_prs_list() {
        let app = many(12, &[1]);
        assert!(lines(&app).contains(&"  #1".to_string()), "12: unfolded");
        let mut app = many(13, &[1]);
        assert!(!lines(&app).contains(&"  #1".to_string()), "13: folded");
        assert!(lines(&app).contains(&"ABC-1".to_string()));
        cursor_to(&mut app, "ABC-1");
        app.dispatch(Action::FoldToggle);
        assert!(lines(&app).contains(&"  #1".to_string()), "z opens it");

        // Twelve PRs from thirteen checkouts count as twelve.
        let mut repos: Vec<RepoSnapshotOf> = (1..=12)
            .map(|i| checkout(&format!("app{i}"), &format!("feature/ABC-{i}-x")))
            .collect();
        repos.push(checkout("app1-copy", "feature/ABC-1-x"));
        let mut app = app_over(&repos, false);
        for (name, i) in (1..=12)
            .map(|i| (format!("app{i}"), i))
            .chain([("app1-copy".to_string(), 1)])
        {
            let branch = format!("feature/ABC-{i}-x");
            let remote = format!("git@github.com:acme/app{i}.git");
            let state = if i == 1 {
                PrState::Merged
            } else {
                PrState::InReview
            };
            land(
                &mut app,
                &name,
                &branch,
                &remote,
                pr(&remote, i, state, "Change", &branch, "main"),
            );
        }
        app.dispatch(Action::OpenPrsTab);
        assert_eq!(app.tab_labels()[1], "PRs 12");
        assert!(lines(&app).contains(&"  #1".to_string()), "distinct: 12");
    }

    #[test]
    fn o_shows_the_first_checkout_of_the_focused_pr_in_the_tree() {
        let mut app = workspace(false);
        // Fold the family of `app` so `o` has to open it.
        app.folds.insert("repo:app".into());
        app.rebuild_rows();
        assert!(
            !app.rows
                .iter()
                .any(|row| row.repo.as_deref() == Some("app-ui")),
            "the family hides its worktree rows"
        );
        app.dispatch(Action::OpenPrsTab);
        cursor_to(&mut app, "  #319");
        let effect = app.dispatch(Action::GraphFocusBranches);
        assert!(app.tabs.is_workspace(), "Workspace tab");
        assert_eq!(app.focus, FocusPane::Left, "the tree has the focus");
        let row = app.focused_row().expect("a focused row");
        assert_eq!(row.repo.as_deref(), Some("app-ui"));
        assert_eq!(effect, Effect::LoadRightPane);

        app.dispatch(Action::OpenPrsTab);
        cursor_to(&mut app, "  #318");
        app.dispatch(Action::GraphFocusBranches);
        assert_eq!(
            app.focused_row()
                .and_then(|row| row.repo.clone())
                .as_deref(),
            Some("app"),
            "the first of the PR's checkouts"
        );
        // On a header: the PR its next action targets.
        app.dispatch(Action::OpenPrsTab);
        cursor_to(&mut app, "ABC-125");
        app.dispatch(Action::GraphFocusBranches);
        assert_eq!(
            app.focused_row()
                .and_then(|row| row.repo.clone())
                .as_deref(),
            Some("api")
        );
    }

    /// Two repositories in one group can both have a `#12` in the same
    /// state. The header acts on the PR its next action names (here the one
    /// with changes requested), not on the first row with the same ref.
    #[test]
    fn a_header_acts_on_its_own_pr_when_two_repos_share_the_number() {
        const GH_MERGER: &str = "git@github.com:acme/merger.git";
        let mut app = app_over(
            &[
                checkout("app", "feature/ABC-140-a"),
                checkout("merger", "feature/ABC-140-b"),
            ],
            false,
        );
        let waiting = pr(
            GH_APP,
            12,
            PrState::InReview,
            "Tidy the app",
            "feature/ABC-140-a",
            "main",
        );
        let mut asked = pr(
            GH_MERGER,
            12,
            PrState::InReview,
            "Tidy the merger",
            "feature/ABC-140-b",
            "main",
        );
        asked.notes.changes_requested = true;
        land(&mut app, "app", "feature/ABC-140-a", GH_APP, waiting);
        land(&mut app, "merger", "feature/ABC-140-b", GH_MERGER, asked);
        app.dispatch(Action::OpenPrsTab);
        assert_eq!(lines(&app), ["ABC-140", "  #12", "  #12"]);
        cursor_to(&mut app, "ABC-140");
        assert_eq!(
            app.dispatch(Action::NavEnter),
            Effect::OpenPullRequest {
                repo: PathBuf::from("merger"),
                branch: "feature/ABC-140-b".into(),
                page: PrPage::Pr,
            }
        );
        assert_eq!(
            app.dispatch(Action::ExportComments),
            Effect::CopyClipboard {
                text: "https://github.com/acme/merger/pull/12".into(),
                announce: true,
            }
        );
    }

    #[test]
    fn tab_switches_list_and_detail_only_in_a_narrow_frame() {
        let mut app = opened(false);
        let shown = |app: &AppState| app.tabs.active_prs().unwrap().show_detail;
        app.layout.term_cols = 132;
        app.dispatch(Action::FocusRight);
        assert!(!shown(&app), "wide: both panes show");
        app.layout.term_cols = 90;
        app.dispatch(Action::FocusRight);
        assert!(shown(&app));
        app.dispatch(Action::FocusLeft);
        assert!(!shown(&app));
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::NavEsc);
        assert!(!shown(&app), "Esc goes back to the list");
    }

    #[test]
    fn keys_act_on_the_focused_pr_and_its_first_checkout() {
        let mut app = opened(false);
        cursor_to(&mut app, "  #318");
        let open = |repo: &str, branch: &str| Effect::OpenPullRequest {
            repo: PathBuf::from(repo),
            branch: branch.into(),
            page: PrPage::Pr,
        };
        let url = "https://github.com/acme/app/pull/318";
        assert_eq!(
            app.dispatch(Action::NavEnter),
            open("app", "feature/ABC-123-retry")
        );
        assert_eq!(
            app.dispatch(Action::OpenPullRequest),
            open("app", "feature/ABC-123-retry"),
            "gx"
        );
        assert_eq!(
            app.dispatch(Action::OpenPullRequestCi),
            Effect::OpenKnownPullRequest {
                pr_ref: "#318".into(),
                url: format!("{url}/checks"),
                page: PrPage::Ci,
            },
            "gc"
        );
        assert_eq!(
            app.dispatch(Action::ExportComments),
            Effect::CopyClipboard {
                text: url.into(),
                announce: true,
            },
            "y"
        );
        cursor_to(&mut app, "ABC-125");
        assert_eq!(
            app.dispatch(Action::NavEnter),
            open("api", "feature/ABC-125-limits"),
            "a header opens the PR its next action targets"
        );
        let Effect::LookupPullRequests { mut targets } = app.dispatch(Action::Refresh) else {
            panic!("r looks every PR up");
        };
        targets.sort();
        assert_eq!(targets.len(), 6, "every checkout: {targets:?}");
        assert_eq!(app.dispatch(Action::Refresh), Effect::None, "in flight");
        assert_eq!(&*app.status, PR_LOOKUP_RUNNING);
        for (action, reason) in [
            (Action::Stage, PRS_TAB_READ_ONLY),
            (Action::Pull, PRS_TAB_READ_ONLY),
            (Action::Fetch, PRS_TAB_READ_ONLY),
            (Action::SearchStart, PRS_TAB_HAS_NO_SEARCH),
            (Action::SearchNext, PRS_TAB_HAS_NO_SEARCH),
            (Action::ToggleTreeMode, SWITCH_TO_WORKSPACE_TAB),
            (Action::ExplorerReveal, SWITCH_TO_WORKSPACE_TAB),
            (Action::Edit, SWITCH_TO_WORKSPACE_TAB),
        ] {
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(&*app.status, reason, "{action:?}");
        }
        assert!(app.is_prs_tab(), "nothing left the tab");
    }

    #[test]
    fn palette_rows_on_the_prs_tab_say_why_they_cannot_run() {
        let reason = |app: &AppState, action: Action| {
            app.palette_disabled_reason(command_for(&action).expect("a palette row"))
        };
        let mut app = workspace(false);
        assert_eq!(reason(&app, Action::OpenPrsTab), None);
        assert_eq!(
            reason(&app, Action::ToggleFinishedPrGroups).as_deref(),
            Some(PRS_TAB_ONLY)
        );
        app.dispatch(Action::OpenPrsTab);
        assert_eq!(reason(&app, Action::OpenPrsTab), None);
        assert_eq!(reason(&app, Action::ToggleFinishedPrGroups), None);
        assert_eq!(reason(&app, Action::OpenPullRequest), None);
        assert_eq!(reason(&app, Action::OpenPullRequestCi), None);
        assert_eq!(reason(&app, Action::FoldToggle), None);
        assert_eq!(
            reason(&app, Action::ExportComments).as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB),
            "`y` copies the PR URL here, not the comments"
        );
        assert_eq!(
            reason(&app, Action::Stage).as_deref(),
            Some(PRS_TAB_READ_ONLY)
        );
        // Go to line would move the parked Workspace diff cursor, and the
        // Refresh row names a reload the tab does not run (its `r` key and
        // the Refresh PRs row look the PRs up again).
        assert_eq!(
            app.palette_disabled_reason(&go_to_line_command(42))
                .as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB)
        );
        app.status.clear();
        assert_eq!(app.dispatch(Action::GoToLine(42)), Effect::None);
        assert_eq!(&*app.status, SWITCH_TO_WORKSPACE_TAB);
        assert_eq!(
            reason(&app, Action::Refresh).as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB)
        );
        assert_eq!(reason(&app, Action::RefreshPullRequests), None);
        app.status.clear();
        assert_ne!(
            app.dispatch(Action::Refresh),
            Effect::None,
            "`r` still runs"
        );
        assert_ne!(&*app.status, SWITCH_TO_WORKSPACE_TAB);
        let mut empty = app_with_no_pr();
        empty.dispatch(Action::OpenPrsTab);
        assert_eq!(
            reason(&empty, Action::OpenPullRequest).as_deref(),
            Some(NO_PR_FOR_ROW)
        );
    }

    fn app_with_no_pr() -> AppState {
        let mut app = app_over(&[checkout("app", "main")], false);
        assert!(app.apply_pr_lookup(
            Path::new("app"),
            "main",
            Some(GH_APP.into()),
            PrLookup::NoPr,
        ));
        app
    }

    #[test]
    fn a_pr_change_flashes_its_prs_tab_row() {
        let mut app = opened(false);
        let url = "https://github.com/acme/app/pull/320";
        let now = Instant::now();
        assert!(app.apply_pr_lookup_at(
            Path::new("app-docs"),
            "docs/readme",
            Some(GH_APP.into()),
            PrLookup::Found(pr(
                GH_APP,
                320,
                PrState::Ready,
                "Update the README",
                "docs/readme",
                "main"
            )),
            now,
        ));
        assert!(app.alert_pr_transitions(now));
        let stamp = app.flashes.get(&prs_row_flash_id(url)).expect("flashed");
        assert_eq!(stamp.kind, super::super::FlashKind::Add);
    }

    #[test]
    fn loading_errors_and_the_checked_age_come_from_the_lookups() {
        let mut app = app_over(
            &[
                checkout("api", "feature/ABC-125-limits"),
                checkout("app", "feature/ABC-123-retry"),
            ],
            false,
        );
        app.dispatch(Action::OpenPrsTab);
        let view = app.prs_view().unwrap();
        assert_eq!(view.lookups, (0, 2), "nothing landed");
        assert_eq!(view.checked, None);
        let landed = Instant::now();
        assert!(app.apply_pr_lookup_at(
            Path::new("app"),
            "feature/ABC-123-retry",
            Some(GH_APP.into()),
            PrLookup::Found(pr(
                GH_APP,
                318,
                PrState::InReview,
                "Retry",
                "feature/ABC-123-retry",
                "main"
            )),
            landed,
        ));
        assert!(app.apply_pr_lookup(
            Path::new("api"),
            "feature/ABC-125-limits",
            Some(GL_API.into()),
            PrLookup::Failed(Some("401 Unauthorized".into())),
        ));
        let tab = app.tabs.active_prs().unwrap();
        let view = app.build_prs_view(tab, landed + Duration::from_secs(3600));
        assert_eq!(view.lookups, (2, 2));
        assert_eq!(view.errors, ["! glab (gitlab.com): 401 Unauthorized"]);
        let checked = view.checked.expect("a lookup succeeded");
        assert_eq!(checked.at, landed);
        assert!(checked.failing, "a failing lookup and a stale PR");
        assert_eq!(lines(&app), ["ABC-123", "  #318"], "other hosts still list");
    }

    /// The keys of a title fill, or `None` when `effect` holds none.
    fn fill_keys(effect: &Effect) -> Option<Vec<String>> {
        match effect {
            Effect::FillPrTitles { groups, .. } => {
                Some(groups.iter().map(|group| group.key.clone()).collect())
            }
            Effect::Batch(effects) => effects.iter().find_map(fill_keys),
            _ => None,
        }
    }

    #[test]
    fn a_title_cycle_fills_only_for_new_keys_an_old_fill_or_r() {
        let mut app = workspace(false);
        let t0 = Instant::now();
        let hour = Duration::from_secs(3600);
        assert_eq!(app.new_pr_title_cycle(t0), Effect::None, "no command");
        set_title_command(&mut app, Path::new("/opt/acme/bin/pr-title"));
        assert_eq!(app.new_pr_title_cycle(t0), Effect::None, "no PRs tab");
        let opened = app.dispatch(Action::OpenPrsTab);
        assert_eq!(
            fill_keys(&opened).as_deref(),
            Some(&["ABC-123", "ABC-125", "ABC-120"].map(String::from)[..]),
            "the first fill, ticket groups only"
        );
        assert_eq!(app.new_pr_title_cycle(t0), Effect::None, "one at a time");
        assert_eq!(
            app.apply_pr_titles(TitleCache::default(), false, t0),
            Effect::None,
            "the cycle that came while it ran has the same keys"
        );
        // Lookups that land one by one start no new fill within a poll
        // period (60 s here: a PR has CI running).
        assert_eq!(app.pr_poll_interval_ms(), 60_000);
        assert_eq!(app.new_pr_title_cycle(t0), Effect::None);
        assert_eq!(
            app.new_pr_title_cycle(t0 + Duration::from_secs(30)),
            Effect::None
        );
        // A fill older than a poll period runs again, once.
        let fill = app.new_pr_title_cycle(t0 + hour);
        assert!(fill_keys(&fill).is_some(), "old: {fill:?}");
        assert_eq!(
            app.apply_pr_titles(TitleCache::default(), false, t0 + hour),
            Effect::None
        );
        assert_eq!(app.new_pr_title_cycle(t0 + hour), Effect::None);
        // A new ticket key fills at once.
        land(
            &mut app,
            "app-docs",
            "docs/readme",
            GH_APP,
            pr(
                GH_APP,
                320,
                PrState::InReview,
                "ABC-130: Update the README",
                "docs/readme",
                "main",
            ),
        );
        let fill = app.new_pr_title_cycle(t0 + hour);
        assert_eq!(
            fill_keys(&fill).as_deref(),
            Some(&["ABC-123", "ABC-125", "ABC-130", "ABC-120"].map(String::from)[..])
        );
        app.apply_pr_titles(TitleCache::default(), false, t0 + hour);
        // `r` on the tab makes the next cycle fill.
        assert!(matches!(
            app.dispatch(Action::Refresh),
            Effect::LookupPullRequests { .. }
        ));
        assert!(fill_keys(&app.new_pr_title_cycle(t0 + hour)).is_some(), "r");
    }

    #[cfg(unix)]
    #[test]
    fn a_title_that_fails_later_stays_on_screen() {
        // Each key gets a title the first time and fails every time after.
        let cmd = script_with(
            "seen=\"$(dirname \"$0\")/seen-$1\"\n\
             [ -e \"$seen\" ] && exit 1\n\
             touch \"$seen\"\n\
             echo \"Title $1\"",
        );
        let mut app = workspace(false);
        set_title_command(&mut app, &cmd);
        let t0 = Instant::now();
        let run = |effect: &Effect| -> TitleCache {
            let Some(Effect::FillPrTitles { cmd, groups }) = (match effect {
                Effect::Batch(effects) => effects.last().cloned(),
                other => Some(other.clone()),
            }) else {
                panic!("a title fill: {effect:?}");
            };
            let mut titles = TitleCache::default();
            titles.fill(&cmd, &groups);
            titles
        };
        let opened = app.dispatch(Action::OpenPrsTab);
        let first = run(&opened);
        app.apply_pr_titles(first, false, t0);
        assert_eq!(app.pr_group_title("ABC-123"), Some("Title ABC-123"));

        let later = t0 + Duration::from_secs(3600);
        let again = app.new_pr_title_cycle(later);
        let failed = run(&again);
        assert_eq!(failed.get("ABC-123"), None, "the second run fails");
        assert_eq!(
            app.pr_group_title("ABC-123"),
            Some("Title ABC-123"),
            "the titles stay while a fill runs"
        );
        app.apply_pr_titles(failed, false, later);
        assert_eq!(
            app.pr_group_title("ABC-123"),
            Some("Title ABC-123"),
            "a failed run keeps the title"
        );
        assert_eq!(app.pr_group_title("ABC-125"), Some("Title ABC-125"));

        // A fill the quit cut short changes nothing, even when it got
        // titles before the quit: the titles and their age stay.
        let last = later + Duration::from_secs(3600);
        let next = app.new_pr_title_cycle(last);
        assert!(matches!(next, Effect::FillPrTitles { .. }));
        let renamed = script_with("echo \"Renamed $1\"");
        let Effect::FillPrTitles { groups, .. } = &next else {
            unreachable!()
        };
        let mut cut_short = TitleCache::default();
        cut_short.fill(&renamed, groups);
        assert_eq!(cut_short.get("ABC-120"), Some("Renamed ABC-120"));
        app.apply_pr_titles(cut_short, true, last);
        assert_eq!(app.pr_group_title("ABC-120"), Some("Title ABC-120"));
        assert_eq!(app.pr_group_title("ABC-123"), Some("Title ABC-123"));
        assert_eq!(app.pr_titles.filled_at, Some(later), "the age stays");
        let _ = std::fs::remove_dir_all(cmd.parent().unwrap());
        let _ = std::fs::remove_dir_all(renamed.parent().unwrap());
    }
}
