//! Icon popovers: a hover peek, a pinned popover, and their content.
//!
//! Paint records one [`super::state::IconHit`] per painted icon (a tagged
//! tree segment, a PR badge). The pointer resting on a hit for
//! [`PEEK_DWELL_MS`] opens a peek: paint only, never an input mode, so it
//! never takes a key. The pointer leaving both the icon and the peek starts
//! a [`PEEK_GRACE_MS`] grace, then the peek closes.
//!
//! A click on an icon, a click inside a peek, or `gh` pins a popover. A
//! pinned popover is [`super::keys::InputMode::Popover`]: `j` / `k` move
//! between its field and action lines, Enter or a click runs the focused
//! action, `y` copies the focused line, and a drag selects its text. Esc, a
//! click outside it, or a change of the focused row closes it.
//!
//! [`PopoverState`] keeps only what to describe (icon kind and target per
//! section). Every frame rebuilds the sections from live state
//! ([`popover_sections`]), so a watch tick under a pinned popover shows new
//! counts and a vanished icon drops its section. Builders are pure
//! functions of the [`AppState`] and one target, so a new icon kind adds one
//! builder arm here and nothing in the core.

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;

use super::action::Action;
use super::command_palette::{command_for, PaletteCommand};
use super::gates::ListFocusTarget;
use super::icons::{capture_count, spec, IconKind};
use super::state::AppState;
use super::tree::{pr_badge_kind, pr_badge_mark, SegRole, VisibleRow};

/// Pointer rest on an icon before its peek opens, in milliseconds.
pub const PEEK_DWELL_MS: u64 = 400;

/// Time a peek stays after the pointer leaves the icon and the peek, in
/// milliseconds. Moving onto the peek inside it keeps the peek open.
pub const PEEK_GRACE_MS: u64 = 250;

/// Widest popover box, border included. A narrower frame caps it at the
/// frame width less 4 columns.
pub const POPOVER_MAX_WIDTH: u16 = 64;

/// Footer of a pinned popover.
pub const POPOVER_FOOTER: &str = "y copy line · Enter run · Esc close";

/// What one painted icon describes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconTarget {
    /// An icon on the workspace tree row with this
    /// [`VisibleRow::id`](super::tree::VisibleRow::id).
    TreeRow(String),
    /// The PR badge of this checkout (snapshot `repo`), on a tree row or a
    /// graph worktree row.
    PullRequest(PathBuf),
}

/// How a popover opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverOrigin {
    /// Hover dwell. Paint only; keys pass through.
    Peek,
    /// Click or `gh`. Owns the keys until it closes.
    Pinned,
}

/// The focus a popover opened on. A pinned popover closes when the focus
/// no longer matches (another row, another pane, another tab).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverOwner {
    /// Active tab index.
    pub tab: usize,
    /// Focused list.
    pub list: ListFocusTarget,
    /// Focused tree row id, graph cursor index, or empty.
    pub row: String,
}

/// An open popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverState {
    /// Peek or pinned.
    pub origin: PopoverOrigin,
    /// One section per entry, in paint order: icon kind and target.
    pub targets: Vec<(IconKind, IconTarget)>,
    /// Screen cells of the icon the popover hangs from. `None` (from `gh`)
    /// hangs it from the first painted icon of [`Self::targets`], else
    /// from the focused row.
    pub anchor: Option<Rect>,
    /// Focused line, an index into [`flat_lines`] of the sections.
    pub focus_line: usize,
    /// Focus when the popover opened.
    pub owner: PopoverOwner,
}

impl PopoverState {
    /// True for a pinned popover.
    pub fn is_pinned(&self) -> bool {
        self.origin == PopoverOrigin::Pinned
    }
}

/// One line of a popover section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopoverLine {
    /// Plain text (the catalog meaning). Not focusable.
    Text(String),
    /// A muted label and a value. `y` copies the value.
    Field {
        /// Short lower-case label.
        label: &'static str,
        /// Value text.
        value: String,
    },
    /// An existing command: title and key chip from the palette catalog.
    /// Enter or a click runs its [`Action`].
    Action(&'static PaletteCommand),
}

impl PopoverLine {
    /// The palette row of `action` as a line. `None` when no row runs it.
    pub fn action(action: &Action) -> Option<Self> {
        command_for(action).map(Self::Action)
    }

    /// True for a line `j` / `k` can focus: a field or an action.
    pub fn focusable(&self) -> bool {
        !matches!(self, Self::Text(_))
    }

    /// Text `y` copies: the value of a field, the title of an action.
    pub fn copy_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Field { value, .. } => value.clone(),
            Self::Action(command) => command.title.to_string(),
        }
    }
}

/// One icon's part of a popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverSection {
    /// Icon described. The heading paints its glyph and catalog name.
    pub icon: IconKind,
    /// Colour role of the glyph, as the row paints it.
    pub role: SegRole,
    /// What the icon is on.
    pub target: IconTarget,
    /// The meaning first, then fields, then actions.
    pub lines: Vec<PopoverLine>,
}

/// Target of icon `kind` painted on tree row `row`.
///
/// A PR badge describes the checkout's PR; every other icon describes the
/// row.
pub fn tree_icon_target(kind: IconKind, row: &VisibleRow) -> IconTarget {
    match (kind, row.repo.as_deref()) {
        (IconKind::PrOpen | IconKind::PrApproved | IconKind::PrMerged, Some(repo)) => {
            IconTarget::PullRequest(PathBuf::from(repo))
        }
        _ => IconTarget::TreeRow(row.id.clone()),
    }
}

/// Sections of `targets` from live state, in order. A target whose icon is
/// gone (row removed, PR answer dropped, sync mark cleared) has none.
pub fn popover_sections(
    state: &AppState,
    targets: &[(IconKind, IconTarget)],
) -> Vec<PopoverSection> {
    targets
        .iter()
        .filter_map(|(kind, target)| section(state, *kind, target))
        .collect()
}

fn section(state: &AppState, kind: IconKind, target: &IconTarget) -> Option<PopoverSection> {
    match target {
        IconTarget::TreeRow(id) => {
            let row = state.rows.iter().find(|row| &row.id == id)?;
            if is_sync_kind(kind) {
                sync_section(row)
            } else {
                None
            }
        }
        IconTarget::PullRequest(repo) => pr_section(state, repo),
    }
}

fn is_sync_kind(kind: IconKind) -> bool {
    matches!(
        kind,
        IconKind::Ahead
            | IconKind::Behind
            | IconKind::Diverged
            | IconKind::NoUpstream
            | IconKind::Clean
    )
}

/// Sync mark: meaning, the counts from `sync_note`, then Pull / Push /
/// Fetch as the state calls for. The kind is the mark the row paints now.
fn sync_section(row: &VisibleRow) -> Option<PopoverSection> {
    let mark = row
        .trailing_segs
        .iter()
        .find(|seg| seg.icon.is_some_and(is_sync_kind))?;
    let kind = mark.icon?;
    let note = row.chrome.sync_note.as_str();
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    let actions: &[Action] = match kind {
        IconKind::Ahead => {
            lines.push(PopoverLine::Field {
                label: "ahead",
                value: commits(note_count(note, "ahead by "), "not pushed"),
            });
            &[Action::Push, Action::Fetch]
        }
        IconKind::Behind => {
            lines.push(PopoverLine::Field {
                label: "behind",
                value: commits(note_count(note, "behind by "), "to pull"),
            });
            &[Action::Pull, Action::Fetch]
        }
        IconKind::Diverged => {
            let counts = note
                .strip_prefix("diverged (")
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or(note);
            lines.push(PopoverLine::Field {
                label: "sync",
                value: counts.to_string(),
            });
            &[Action::Fetch, Action::Pull, Action::Push]
        }
        IconKind::NoUpstream => {
            if !note.is_empty() {
                lines.push(PopoverLine::Field {
                    label: "note",
                    value: note.to_string(),
                });
            }
            &[Action::Push, Action::Fetch]
        }
        _ => &[Action::Fetch],
    };
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role: mark.role,
        target: IconTarget::TreeRow(row.id.clone()),
        lines,
    })
}

/// The number after `prefix` in a sync note (`ahead by 3 commits`).
fn note_count(note: &str, prefix: &str) -> Option<u64> {
    capture_count(note, prefix).parse().ok()
}

/// `3 commits <tail>`, `1 commit <tail>`, or `commits <tail>` with no count.
fn commits(count: Option<u64>, tail: &str) -> String {
    match count {
        Some(1) => format!("1 commit {tail}"),
        Some(n) => format!("{n} commits {tail}"),
        None => format!("commits {tail}"),
    }
}

/// PR badge: meaning, number, URL (`y` copies it), and Open PR (`gx`).
fn pr_section(state: &AppState, repo: &Path) -> Option<PopoverSection> {
    let pr = state.pull_request_for(repo)?;
    let (_, role) = pr_badge_mark(state.ascii, pr.state);
    let kind = pr_badge_kind(pr.state);
    let mut lines = vec![
        PopoverLine::Text(spec(kind).meaning.to_string()),
        PopoverLine::Field {
            label: "number",
            value: format!("#{}", pr.number),
        },
        PopoverLine::Field {
            label: "url",
            value: pr.url.clone(),
        },
    ];
    lines.extend(PopoverLine::action(&Action::OpenPullRequest));
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::PullRequest(repo.to_path_buf()),
        lines,
    })
}

/// Every line of `sections`, in order. [`PopoverState::focus_line`]
/// indexes this list.
pub fn flat_lines(sections: &[PopoverSection]) -> Vec<&PopoverLine> {
    sections
        .iter()
        .flat_map(|section| section.lines.iter())
        .collect()
}

/// Line a new pinned popover focuses: the first action, else the first
/// field, else 0.
pub fn landing_line(lines: &[&PopoverLine]) -> usize {
    lines
        .iter()
        .position(|line| matches!(line, PopoverLine::Action(_)))
        .or_else(|| lines.iter().position(|line| line.focusable()))
        .unwrap_or(0)
}

/// The focusable line `focus` stands for: itself, else the nearest one
/// after it, else the nearest one before it. `None` when no line focuses.
///
/// Content re-derives every frame, so a stored focus can land on a line
/// that moved or went away.
pub fn focused_line(lines: &[&PopoverLine], focus: usize) -> Option<usize> {
    let focus = focus.min(lines.len().checked_sub(1)?);
    (focus..lines.len())
        .chain((0..focus).rev())
        .find(|&index| lines[index].focusable())
}

/// Focus after `delta` focusable steps from `focus`, stopping at the ends.
pub fn step_line(lines: &[&PopoverLine], focus: usize, delta: i32) -> usize {
    let Some(mut at) = focused_line(lines, focus) else {
        return focus;
    };
    for _ in 0..delta.unsigned_abs() {
        let next = if delta > 0 {
            (at + 1..lines.len()).find(|&index| lines[index].focusable())
        } else {
            (0..at).rev().find(|&index| lines[index].focusable())
        };
        match next {
            Some(index) => at = index,
            None => break,
        }
    }
    at
}

/// Box of a `width` × `height` popover that hangs from `anchor` inside
/// `bounds`.
///
/// It opens on the row below the anchor, flips above it when the rows
/// below cannot hold it, and sits at the bottom of `bounds` when neither
/// side can. Its left edge is the anchor column, moved left so the box
/// stays inside `bounds`. A box larger than `bounds` shrinks to it.
pub fn popover_rect(anchor: Rect, bounds: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(bounds.width);
    let height = height.min(bounds.height);
    let below = anchor.y.saturating_add(1);
    let y = if below >= bounds.y && below.saturating_add(height) <= bounds.bottom() {
        below
    } else if anchor.y >= bounds.y.saturating_add(height) {
        anchor.y - height
    } else {
        bounds.bottom().saturating_sub(height).max(bounds.y)
    };
    let x = anchor
        .x
        .min(bounds.right().saturating_sub(width))
        .max(bounds.x);
    Rect::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::command_palette::PALETTE_COMMANDS;

    fn action_line(title: &str) -> PopoverLine {
        PopoverLine::Action(
            PALETTE_COMMANDS
                .iter()
                .find(|command| command.title == title)
                .expect("palette row"),
        )
    }

    fn field(value: &str) -> PopoverLine {
        PopoverLine::Field {
            label: "x",
            value: value.into(),
        }
    }

    #[test]
    fn rect_opens_below_flips_above_and_clamps_right() {
        let bounds = Rect::new(0, 1, 80, 20);
        let icon = |x, y| Rect::new(x, y, 1, 1);
        assert_eq!(
            popover_rect(icon(10, 3), bounds, 30, 6),
            Rect::new(10, 4, 30, 6),
            "below the icon"
        );
        assert_eq!(
            popover_rect(icon(10, 20), bounds, 30, 6),
            Rect::new(10, 14, 30, 6),
            "bottom row flips above"
        );
        assert_eq!(
            popover_rect(icon(75, 3), bounds, 30, 6),
            Rect::new(50, 4, 30, 6),
            "right edge clamps left"
        );
        assert_eq!(
            popover_rect(icon(5, 10), bounds, 30, 30),
            Rect::new(5, 1, 30, 20),
            "taller than the bounds shrinks and sits at the top"
        );
        assert_eq!(
            popover_rect(icon(5, 4), Rect::new(0, 1, 80, 8), 20, 6),
            Rect::new(5, 3, 20, 6),
            "no room either side sits at the bottom"
        );
    }

    #[test]
    fn focus_moves_over_fields_and_actions_only() {
        let text = PopoverLine::Text("meaning".into());
        let url = field("https://example.test/pull/7");
        let pull = action_line("Pull behind");
        let fetch = action_line("Fetch remotes");
        let lines = vec![&text, &url, &pull, &fetch];
        assert_eq!(landing_line(&lines), 2, "lands on the first action");
        assert_eq!(step_line(&lines, 2, 1), 3);
        assert_eq!(step_line(&lines, 3, 1), 3, "stops at the last line");
        assert_eq!(step_line(&lines, 2, -1), 1);
        assert_eq!(step_line(&lines, 1, -1), 1, "the meaning never focuses");
        assert_eq!(focused_line(&lines, 0), Some(1));
        assert_eq!(focused_line(&lines, 9), Some(3), "past the end clamps");
        let only_text = vec![&text];
        assert_eq!(focused_line(&only_text, 0), None);
        assert_eq!(landing_line(&only_text), 0);
        assert_eq!(landing_line(&[&text, &url]), 1, "no action: first field");
    }

    #[test]
    fn lines_copy_the_value_or_title() {
        assert_eq!(field("#7").copy_text(), "#7");
        assert_eq!(action_line("Push").copy_text(), "Push");
        assert_eq!(PopoverLine::Text("m".into()).copy_text(), "m");
        assert_eq!(
            PopoverLine::action(&Action::Push),
            Some(action_line("Push"))
        );
        assert_eq!(PopoverLine::action(&Action::PopoverClose), None);
    }

    #[test]
    fn sync_counts_read_the_note() {
        assert_eq!(note_count("ahead by 3 commits", "ahead by "), Some(3));
        assert_eq!(note_count("behind by 12 commits", "behind by "), Some(12));
        assert_eq!(note_count("", "behind by "), None);
        assert_eq!(commits(Some(1), "to pull"), "1 commit to pull");
        assert_eq!(commits(Some(4), "not pushed"), "4 commits not pushed");
        assert_eq!(commits(None, "to pull"), "commits to pull");
    }
}
