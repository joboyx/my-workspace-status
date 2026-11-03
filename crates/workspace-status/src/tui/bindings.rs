//! Key bindings: each key chip and the [`Action`] it runs, written once.
//!
//! A [`KeyBinding`] is the chip text the palette and help show plus the
//! action the key runs. The keymap tables ([`NORMAL_KEYS`],
//! [`G_CHORD_KEYS`], [`HIGHLIGHT_KEYS`], …) hold the one-to-one bindings of
//! one input context; `keys.rs` looks a key up in them at the place where
//! its match arm used to be, so first-match order does not change. The
//! command palette takes each row's chip and action from the same consts.
//!
//! Keys whose action depends on state (Tab, `h` / `l` fold or pan) or that
//! have their own modifier rules (Ctrl-f, Ctrl-p, `:`, `F`, Ctrl-c) keep
//! their code in `keys.rs`; their consts here exist so the chip text has
//! one source.
//!
//! [`chips!`] joins several chips into one `&'static str` at compile time,
//! so a row can show `Ctrl-p F` without restating either chip.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::action::{Action, QuickOpenEntry};

/// One key and the action it runs in one input context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    /// Chip text the palette and help show; the keymap matches this key.
    ///
    /// A single character is that character, `space` is the space bar,
    /// `Tab` / `Esc` / `Enter` / `Home` / `End` / `PgUp` / `PgDn` and the
    /// arrow names are those keys, `Ctrl-x` is the Ctrl chord
    /// ([`is_ctrl_chord`]), and a two-character chord (`gt`, `zz`) matches
    /// its second key in the pending-chord context.
    pub chip: &'static str,
    /// Action the key runs.
    pub action: Action,
    /// Other keys that run the same action in the same context (`J` and
    /// `Down` for `j`). They match like [`Self::chip`] but no chip shows
    /// them.
    pub other_keys: &'static [&'static str],
}

impl KeyBinding {
    /// True when `key` is this binding's chip or one of its other keys.
    ///
    /// Plain keys compare the key code only; the caller applies the
    /// modifier guards of its context first.
    pub fn matches(&self, key: KeyEvent) -> bool {
        chip_matches(self.chip, key) || self.other_keys.iter().any(|chip| chip_matches(chip, key))
    }
}

/// The first binding in `table` that matches `key`.
pub fn find(table: &'static [KeyBinding], key: KeyEvent) -> Option<&'static KeyBinding> {
    table.iter().find(|binding| binding.matches(key))
}

const fn bind(chip: &'static str, action: Action) -> KeyBinding {
    KeyBinding {
        chip,
        action,
        other_keys: &[],
    }
}

fn chip_matches(chip: &str, key: KeyEvent) -> bool {
    if let Some(rest) = chip.strip_prefix("Ctrl-") {
        let mut chars = rest.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => is_ctrl_chord(key, c),
            _ => false,
        };
    }
    let code = match chip {
        "space" => KeyCode::Char(' '),
        "Tab" => KeyCode::Tab,
        "Esc" => KeyCode::Esc,
        "Enter" => KeyCode::Enter,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PgUp" => KeyCode::PageUp,
        "PgDn" => KeyCode::PageDown,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        _ => {
            let mut chars = chip.chars();
            match (chars.next(), chars.next(), chars.next()) {
                (Some(c), None, None) => KeyCode::Char(c),
                // A chord chip (`gt`, `zz`): the pending context reads the
                // second key.
                (Some(_), Some(c), None) => KeyCode::Char(c),
                _ => return false,
            }
        }
    };
    key.code == code
}

/// Modifiers that turn a key into a command chord.
///
/// SHIFT is not one of them: it selects the shifted binding (`P`, `O`, …).
pub(crate) const COMMAND_MODIFIERS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

/// True when `key` carries Ctrl, Alt, Super, Hyper, or Meta.
pub(crate) fn has_command_modifier(key: KeyEvent) -> bool {
    key.modifiers.intersects(COMMAND_MODIFIERS)
}

/// True for Ctrl+`c` with no Alt, Super, Hyper, or Meta (Shift is ignored).
pub(crate) fn is_ctrl_chord(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c)
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key
            .modifiers
            .intersects(COMMAND_MODIFIERS.difference(KeyModifiers::CONTROL))
}

/// Byte length of `parts` joined with one space (see [`chips!`]).
pub const fn chips_len(parts: &[&str]) -> usize {
    let mut len = 0;
    let mut i = 0;
    while i < parts.len() {
        if i > 0 {
            len += 1;
        }
        len += parts[i].len();
        i += 1;
    }
    len
}

/// `parts` joined with one space as `N` bytes, `N` = [`chips_len`].
///
/// Panics (at compile time in a const) when `N` is not that length.
pub const fn chips_bytes<const N: usize>(parts: &[&str]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut at = 0;
    let mut i = 0;
    while i < parts.len() {
        if i > 0 {
            out[at] = b' ';
            at += 1;
        }
        let bytes = parts[i].as_bytes();
        let mut j = 0;
        while j < bytes.len() {
            out[at] = bytes[j];
            at += 1;
            j += 1;
        }
        i += 1;
    }
    assert!(at == N, "chips_bytes: N is not chips_len(parts)");
    out
}

/// Several chips joined with one space, as a `&'static str` in const
/// context: `chips!(GO_TO_FILE.chip, GO_TO_FILE_F.chip)` is `"Ctrl-p F"`.
macro_rules! chips {
    ($($part:expr),+ $(,)?) => {{
        const PARTS: &[&str] = &[$($part),+];
        const BYTES: [u8; $crate::tui::bindings::chips_len(PARTS)] =
            $crate::tui::bindings::chips_bytes::<{ $crate::tui::bindings::chips_len(PARTS) }>(PARTS);
        match core::str::from_utf8(&BYTES) {
            Ok(joined) => joined,
            Err(_) => panic!("chip is not UTF-8"),
        }
    }};
}
pub(crate) use chips;

// Normal mode, plain keys.

/// `t`: flat list or tree.
pub const FLAT_OR_TREE: KeyBinding = bind("t", Action::ToggleTreeMode);
/// `q`: quit (normal mode and highlight).
pub const QUIT: KeyBinding = bind("q", Action::Quit);
/// `?`: keymap help (normal mode and highlight).
pub const KEYMAP_HELP: KeyBinding = bind("?", Action::ToggleHelp);
/// `.`: show or hide hidden repos.
pub const SHOW_HIDDEN: KeyBinding = bind(".", Action::ToggleShowHidden);
/// `f`: fetch remotes.
pub const FETCH: KeyBinding = bind("f", Action::Fetch);
/// `p`: pull behind.
pub const PULL: KeyBinding = bind("p", Action::Pull);
/// `d`: switch to the default branch.
pub const DEFAULT_BRANCH: KeyBinding = bind("d", Action::DefaultBranch);
/// `r`: refresh now.
pub const REFRESH: KeyBinding = bind("r", Action::Refresh);
/// Space: mark the file reviewed.
pub const MARK_REVIEWED: KeyBinding = bind("space", Action::ToggleReviewed);
/// `z`: fold or unfold the row.
pub const FOLD_ROW: KeyBinding = bind("z", Action::FoldToggle);
/// `Z`: fold the finished PR groups.
pub const FOLD_FINISHED_PR_GROUPS: KeyBinding = bind("Z", Action::ToggleFinishedPrGroups);
/// `/`: search the focused pane.
pub const SEARCH_PANE: KeyBinding = bind("/", Action::SearchStart);
/// `s`: stage (normal mode and highlight).
pub const STAGE: KeyBinding = bind("s", Action::Stage);
/// `u`: unstage (normal mode and highlight).
pub const UNSTAGE: KeyBinding = bind("u", Action::Unstage);
/// `x`: revert (normal mode and highlight).
pub const REVERT: KeyBinding = bind("x", Action::Revert);
/// `e`: open in the editor.
pub const OPEN_IN_EDITOR: KeyBinding = bind("e", Action::Edit);
/// `E`: open in the diff tool.
pub const OPEN_IN_DIFF_TOOL: KeyBinding = bind("E", Action::ExternalDiff);
/// `P`: push.
pub const PUSH: KeyBinding = bind("P", Action::Push);
/// `S`: stash menu.
pub const STASH_MENU: KeyBinding = bind("S", Action::StashMenu);
/// `b`: branch picker.
pub const BRANCH_PICKER: KeyBinding = bind("b", Action::Branch);
/// `o`: graph focus on branches.
pub const GRAPH_FOCUS_BRANCHES: KeyBinding = bind("o", Action::GraphFocusBranches);
/// `O`: clear the graph focus.
pub const CLEAR_GRAPH_FOCUS: KeyBinding = bind("O", Action::GraphFocusClear);
/// `W`: remove the linked worktree.
pub const REMOVE_WORKTREE: KeyBinding = bind("W", Action::RemoveWorktree);
/// `i`: inline or split diff.
pub const INLINE_OR_SPLIT: KeyBinding = bind("i", Action::ToggleDiffMode);
/// `<`: narrow the tree pane.
pub const NARROW_TREE: KeyBinding = bind("<", Action::ResizeTree(-1));
/// `>`: widen the tree pane.
pub const WIDEN_TREE: KeyBinding = bind(">", Action::ResizeTree(1));
/// `\`: wrap or unwrap the diff.
pub const WRAP_DIFF: KeyBinding = bind("\\", Action::ToggleDiffWrap);
/// `M`: collapse or expand the commit message.
pub const EXPAND_COMMIT_MESSAGE: KeyBinding = bind("M", Action::ToggleCommitMsgExpand);
/// `-`: open in the Explorer tab.
pub const OPEN_IN_EXPLORER: KeyBinding = bind("-", Action::ExplorerReveal);
/// `B`: line blame on or off.
pub const LINE_BLAME: KeyBinding = bind("B", Action::ToggleLineBlame);
/// `A`: blame actions menu.
pub const BLAME_MENU: KeyBinding = bind("A", Action::BlameMenu);
/// `m`: mouse on or off (normal mode and highlight).
pub const TOGGLE_MOUSE: KeyBinding = bind("m", Action::ToggleMouse);
/// `;`: comment (normal mode and highlight).
pub const COMMENT: KeyBinding = bind(";", Action::CommentStart);
/// `V`: highlight diff lines.
pub const HIGHLIGHT_LINES: KeyBinding = bind("V", Action::DiffVisualStart);
/// `y`: copy comments as markdown.
pub const COPY_COMMENTS: KeyBinding = bind("y", Action::ExportComments);
/// `'`: copy an entity reference (normal mode and highlight).
pub const COPY_ENTITY_REFERENCE: KeyBinding = bind("'", Action::CopyEntityReference);
/// `[`: previous file tab.
pub const PREVIOUS_FILE_TAB: KeyBinding = bind("[", Action::PreviousFileTab);
/// `]`: next file tab.
pub const NEXT_FILE_TAB: KeyBinding = bind("]", Action::NextFileTab);
/// `n`: next search match. With no armed search it still dispatches so
/// the status can say why.
pub const NEXT_MATCH: KeyBinding = bind("n", Action::SearchNext);
/// `N`: previous search match.
pub const PREVIOUS_MATCH: KeyBinding = bind("N", Action::SearchPrev);
/// `G`: bottom of the focused pane (normal mode and highlight).
pub const GO_TO_BOTTOM: KeyBinding = bind("G", Action::MoveToEnd);
/// End: bottom of the focused pane (normal mode and highlight).
pub const MOVE_TO_END: KeyBinding = bind("End", Action::MoveToEnd);
/// `g`: arm the `g` chord (normal mode and highlight).
pub const ARM_G_CHORD: KeyBinding = bind("g", Action::ArmGChord);
/// Home: top of the focused pane (normal mode and highlight).
pub const MOVE_TO_START: KeyBinding = bind("Home", Action::MoveToStart);
/// PgUp: page up (normal mode and highlight).
pub const PAGE_UP: KeyBinding = bind("PgUp", Action::PageMove(-1));
/// PgDn: page down (normal mode and highlight).
pub const PAGE_DOWN: KeyBinding = bind("PgDn", Action::PageMove(1));
/// `j`, `J`, or Down: one row down (normal mode and highlight).
pub const MOVE_DOWN: KeyBinding = KeyBinding {
    chip: "j",
    action: Action::Move(1),
    other_keys: &["J", "Down"],
};
/// `k`, `K`, or Up: one row up (normal mode and highlight).
pub const MOVE_UP: KeyBinding = KeyBinding {
    chip: "k",
    action: Action::Move(-1),
    other_keys: &["K", "Up"],
};
/// Enter: focus right or drill in.
pub const NAV_ENTER: KeyBinding = bind("Enter", Action::NavEnter);
/// Esc: back or unfocus.
pub const NAV_BACK: KeyBinding = bind("Esc", Action::NavEsc);

/// Normal-mode plain keys with one fixed action, after the modifier guard.
pub const NORMAL_KEYS: &[KeyBinding] = &[
    FLAT_OR_TREE,
    QUIT,
    KEYMAP_HELP,
    SHOW_HIDDEN,
    FETCH,
    PULL,
    DEFAULT_BRANCH,
    REFRESH,
    MARK_REVIEWED,
    FOLD_ROW,
    FOLD_FINISHED_PR_GROUPS,
    SEARCH_PANE,
    STAGE,
    UNSTAGE,
    REVERT,
    OPEN_IN_EDITOR,
    OPEN_IN_DIFF_TOOL,
    PUSH,
    STASH_MENU,
    BRANCH_PICKER,
    GRAPH_FOCUS_BRANCHES,
    CLEAR_GRAPH_FOCUS,
    REMOVE_WORKTREE,
    INLINE_OR_SPLIT,
    NARROW_TREE,
    WIDEN_TREE,
    WRAP_DIFF,
    EXPAND_COMMIT_MESSAGE,
    OPEN_IN_EXPLORER,
    LINE_BLAME,
    BLAME_MENU,
    TOGGLE_MOUSE,
    COMMENT,
    HIGHLIGHT_LINES,
    COPY_COMMENTS,
    COPY_ENTITY_REFERENCE,
    PREVIOUS_FILE_TAB,
    NEXT_FILE_TAB,
    NEXT_MATCH,
    PREVIOUS_MATCH,
    GO_TO_BOTTOM,
    MOVE_TO_END,
    ARM_G_CHORD,
    MOVE_TO_START,
    PAGE_UP,
    PAGE_DOWN,
    MOVE_DOWN,
    MOVE_UP,
    NAV_ENTER,
    NAV_BACK,
];

// Normal mode, Ctrl chords.

/// Ctrl-o: full-file context.
pub const FULL_FILE_CONTEXT: KeyBinding = bind("Ctrl-o", Action::ToggleFullContext);
/// Ctrl-u: five rows up (normal mode and highlight).
pub const HALF_PAGE_UP: KeyBinding = bind("Ctrl-u", Action::Move(-5));
/// Ctrl-d: five rows down (normal mode and highlight).
pub const HALF_PAGE_DOWN: KeyBinding = bind("Ctrl-d", Action::Move(5));
/// Ctrl-w (or Ctrl-W): close the tab.
pub const CLOSE_TAB: KeyBinding = KeyBinding {
    chip: "Ctrl-w",
    action: Action::CloseTab,
    other_keys: &["Ctrl-W"],
};

/// Normal-mode Ctrl chords; they match before the modifier guard.
pub const NORMAL_CTRL_KEYS: &[KeyBinding] =
    &[FULL_FILE_CONTEXT, HALF_PAGE_UP, HALF_PAGE_DOWN, CLOSE_TAB];

// Graph rows.

/// `a` on a graph stash row: apply the stash.
pub const GRAPH_STASH_APPLY: KeyBinding = bind("a", Action::GraphStashApply);
/// `p` on a graph stash row: pop the stash.
pub const GRAPH_STASH_POP: KeyBinding = bind("p", Action::GraphStashPop);
/// `D` on a graph stash row: drop the stash.
pub const GRAPH_STASH_DROP: KeyBinding = bind("D", Action::GraphStashDrop);

/// Keys on a focused graph stash row; they match before [`NORMAL_KEYS`].
pub const GRAPH_STASH_KEYS: &[KeyBinding] = &[GRAPH_STASH_APPLY, GRAPH_STASH_POP, GRAPH_STASH_DROP];

/// `b` on a graph commit row: check out its refs.
pub const GRAPH_CHECKOUT: KeyBinding = bind("b", Action::GraphCheckout);
/// `c` on a graph commit row: create a branch there.
pub const GRAPH_CREATE_BRANCH: KeyBinding = bind("c", Action::GraphCreateBranch);
/// `m` on a graph commit row: merge it into HEAD.
pub const GRAPH_MERGE: KeyBinding = bind("m", Action::GraphMerge);

/// Keys on a focused graph commit row; they match before [`NORMAL_KEYS`].
pub const GRAPH_COMMIT_KEYS: &[KeyBinding] = &[GRAPH_CHECKOUT, GRAPH_CREATE_BRANCH, GRAPH_MERGE];

// Chords.

/// `gg`: top of the focused pane.
pub const GO_TO_TOP: KeyBinding = bind("gg", Action::MoveToStart);
/// `gt`: next tab.
pub const NEXT_TAB: KeyBinding = bind("gt", Action::NextTab);
/// `gT`: previous tab.
pub const PREVIOUS_TAB: KeyBinding = bind("gT", Action::PreviousTab);
/// `gx`: open the pull request.
pub const OPEN_PR: KeyBinding = bind("gx", Action::OpenPullRequest);
/// `gc`: open the pull request CI page.
pub const OPEN_PR_CI: KeyBinding = bind("gc", Action::OpenPullRequestCi);
/// `gp`: PRs tab. Bound so a `p` after `g` never falls through to Pull.
pub const PRS_TAB: KeyBinding = bind("gp", Action::OpenPrsTab);
/// `gw`: view on the remote.
pub const VIEW_REMOTELY: KeyBinding = bind("gw", Action::ViewRemotely);
/// `gh`: pin the icon popover of the focused row.
pub const ICON_POPOVER: KeyBinding = bind("gh", Action::PopoverOpenFocused);

/// Second keys after `g` (pending `g` chord). `g1`–`g9` stay in `keys.rs`.
pub const G_CHORD_KEYS: &[KeyBinding] = &[
    GO_TO_TOP,
    NEXT_TAB,
    PREVIOUS_TAB,
    OPEN_PR,
    OPEN_PR_CI,
    PRS_TAB,
    VIEW_REMOTELY,
    ICON_POPOVER,
];

/// `zz`: fold or unfold the subtree (second `z` of a pending `z` chord).
pub const FOLD_SUBTREE: KeyBinding = bind("zz", Action::FoldToggleSubtree);

// Visual-line highlight.

/// Esc or `V`: leave highlight without commenting.
pub const EXIT_HIGHLIGHT: KeyBinding = KeyBinding {
    chip: "Esc",
    action: Action::DiffVisualCancel,
    other_keys: &["V"],
};
/// `h`, `H`, or Left: pan left in highlight. In normal mode `keys.rs`
/// folds or pans on these keys.
pub const PAN_LEFT: KeyBinding = KeyBinding {
    chip: "h",
    action: Action::PanDiff(-1),
    other_keys: &["H", "Left"],
};
/// `l`, `L`, or Right: pan right in highlight. In normal mode `keys.rs`
/// folds or pans on these keys.
pub const PAN_RIGHT: KeyBinding = KeyBinding {
    chip: "l",
    action: Action::PanDiff(1),
    other_keys: &["L", "Right"],
};

/// Highlight Ctrl chords; they match before the modifier guard.
pub const HIGHLIGHT_CTRL_KEYS: &[KeyBinding] = &[HALF_PAGE_UP, HALF_PAGE_DOWN];

/// Highlight plain keys with one fixed action, after the modifier guard.
pub const HIGHLIGHT_KEYS: &[KeyBinding] = &[
    KEYMAP_HELP,
    QUIT,
    TOGGLE_MOUSE,
    ARM_G_CHORD,
    GO_TO_BOTTOM,
    MOVE_TO_END,
    MOVE_TO_START,
    PAGE_UP,
    PAGE_DOWN,
    EXIT_HIGHLIGHT,
    COMMENT,
    COPY_ENTITY_REFERENCE,
    STAGE,
    UNSTAGE,
    REVERT,
    MOVE_DOWN,
    MOVE_UP,
    PAN_LEFT,
    PAN_RIGHT,
];

// Keys with state-dependent actions or their own modifier rules. `keys.rs`
// matches them in code; these consts give the chip one source.

/// Tab: the other pane. `keys.rs` runs [`Action::FocusLeft`] instead while
/// the right pane has focus.
pub const OTHER_PANE: KeyBinding = bind("Tab", Action::FocusRight);
/// Ctrl-f (Shift allowed): search in files.
pub const SEARCH_IN_FILES: KeyBinding = bind("Ctrl-f", Action::ToggleSearchFiles);
/// `:`: Quick Open commands.
pub const QUICK_OPEN_COMMANDS: KeyBinding =
    bind(":", Action::ToggleQuickOpen(QuickOpenEntry::Commands));
/// Ctrl-p (Ctrl alone): Quick Open files.
pub const GO_TO_FILE: KeyBinding = bind("Ctrl-p", Action::ToggleQuickOpen(QuickOpenEntry::Files));
/// `F`: Quick Open files.
pub const GO_TO_FILE_F: KeyBinding = bind("F", Action::ToggleQuickOpen(QuickOpenEntry::Files));
/// Ctrl-c: first press of the double Ctrl-c quit.
pub const CTRL_C_EXIT: KeyBinding = bind("Ctrl-c", Action::CtrlC);

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crossterm::event::{Event, KeyCode};

    use super::super::command_palette::{CommandPaletteState, PALETTE_COMMANDS};
    use super::super::keys::{event_to_action_ex, InputMode};
    use super::*;
    use crate::testutil::{ctrl, key};

    /// Input context a key is pressed in.
    #[derive(Clone, Copy, Debug)]
    enum Ctx {
        /// Normal mode, left pane focused, no graph row focused.
        Normal,
        /// Pending `g` chord.
        GChord,
        /// Pending `z` chord.
        ZChord,
        /// Visual-line highlight on a diff.
        Highlight,
        /// Normal mode on a focused graph stash row.
        GraphStash,
        /// Normal mode on a focused graph commit row.
        GraphCommit,
    }

    fn press(ctx: Ctx, event: &Event) -> Action {
        let normal = InputMode::Normal {
            search_active: false,
        };
        let (mode, stash, commit) = match ctx {
            Ctx::Normal => (normal, false, false),
            Ctx::GChord => (
                InputMode::GPending {
                    search_active: false,
                },
                false,
                false,
            ),
            Ctx::ZChord => (
                InputMode::ZPending {
                    search_active: false,
                },
                false,
                false,
            ),
            Ctx::Highlight => (InputMode::DiffVisual, false, false),
            Ctx::GraphStash => (normal, true, false),
            Ctx::GraphCommit => (normal, false, true),
        };
        event_to_action_ex(event, mode, false, false, stash, commit)
    }

    /// Chip text and action of the palette row titled `title`, reached by
    /// filtering on the title and moving the cursor onto that row.
    fn palette_row(title: &str) -> (&'static str, Option<Action>) {
        let mut palette = CommandPaletteState::new();
        palette.set_filter(title);
        let index = palette
            .rows()
            .iter()
            .position(|row| row.title == title)
            .unwrap_or_else(|| panic!("filter {title:?} lists no row with that title"));
        palette.move_cursor(index as i32);
        let selected = palette.selected().expect(title);
        assert_eq!(selected.title, title);
        (selected.keys, palette.selected_action())
    }

    /// Every non-theme palette row runs its action from the palette, shows
    /// its chip, and that key runs the same action in its context.
    #[test]
    fn palette_rows_and_their_keys_run_the_same_action() {
        use Ctx::*;
        use KeyCode::{Char, Esc, Tab};
        let cases = [
            (
                "Stage highlighted lines",
                "s",
                Some((Highlight, key(Char('s')))),
                Action::Stage,
            ),
            (
                "Unstage highlighted lines",
                "u",
                Some((Highlight, key(Char('u')))),
                Action::Unstage,
            ),
            (
                "Revert highlighted lines",
                "x",
                Some((Highlight, key(Char('x')))),
                Action::Revert,
            ),
            (
                "Exit highlight",
                "Esc",
                Some((Highlight, key(Esc))),
                Action::DiffVisualCancel,
            ),
            (
                "Search focused pane",
                "/",
                Some((Normal, key(Char('/')))),
                Action::SearchStart,
            ),
            (
                "Next match",
                "n",
                Some((Normal, key(Char('n')))),
                Action::SearchNext,
            ),
            (
                "Previous match",
                "N",
                Some((Normal, key(Char('N')))),
                Action::SearchPrev,
            ),
            (
                "Search in files",
                "Ctrl-f",
                Some((Normal, ctrl(Char('f')))),
                Action::ToggleSearchFiles,
            ),
            (
                "Fold row",
                "z",
                Some((Normal, key(Char('z')))),
                Action::FoldToggle,
            ),
            (
                "Fold subtree",
                "zz",
                Some((ZChord, key(Char('z')))),
                Action::FoldToggleSubtree,
            ),
            (
                "Other pane",
                "Tab",
                Some((Normal, key(Tab))),
                Action::FocusRight,
            ),
            (
                "Next tab",
                "gt",
                Some((GChord, key(Char('t')))),
                Action::NextTab,
            ),
            (
                "Previous tab",
                "gT",
                Some((GChord, key(Char('T')))),
                Action::PreviousTab,
            ),
            (
                "Previous file tab",
                "[",
                Some((Normal, key(Char('[')))),
                Action::PreviousFileTab,
            ),
            (
                "Next file tab",
                "]",
                Some((Normal, key(Char(']')))),
                Action::NextFileTab,
            ),
            (
                "Open in Explorer",
                "-",
                Some((Normal, key(Char('-')))),
                Action::ExplorerReveal,
            ),
            (
                "Fold finished PR groups",
                "Z",
                Some((Normal, key(Char('Z')))),
                Action::ToggleFinishedPrGroups,
            ),
            ("Stage", "s", Some((Normal, key(Char('s')))), Action::Stage),
            (
                "Unstage",
                "u",
                Some((Normal, key(Char('u')))),
                Action::Unstage,
            ),
            (
                "Revert",
                "x",
                Some((Normal, key(Char('x')))),
                Action::Revert,
            ),
            (
                "Stash menu",
                "S",
                Some((Normal, key(Char('S')))),
                Action::StashMenu,
            ),
            (
                "Fetch remotes",
                "f",
                Some((Normal, key(Char('f')))),
                Action::Fetch,
            ),
            (
                "Pull behind",
                "p",
                Some((Normal, key(Char('p')))),
                Action::Pull,
            ),
            ("Push", "P", Some((Normal, key(Char('P')))), Action::Push),
            (
                "Default branch",
                "d",
                Some((Normal, key(Char('d')))),
                Action::DefaultBranch,
            ),
            (
                "Branch picker",
                "b",
                Some((Normal, key(Char('b')))),
                Action::Branch,
            ),
            (
                "Checkout commit refs",
                "b",
                Some((GraphCommit, key(Char('b')))),
                Action::GraphCheckout,
            ),
            (
                "Create branch at commit",
                "c",
                Some((GraphCommit, key(Char('c')))),
                Action::GraphCreateBranch,
            ),
            (
                "Merge into HEAD",
                "m",
                Some((GraphCommit, key(Char('m')))),
                Action::GraphMerge,
            ),
            (
                "Remove worktree",
                "W",
                Some((Normal, key(Char('W')))),
                Action::RemoveWorktree,
            ),
            (
                "Apply stash",
                "a",
                Some((GraphStash, key(Char('a')))),
                Action::GraphStashApply,
            ),
            (
                "Pop stash",
                "p",
                Some((GraphStash, key(Char('p')))),
                Action::GraphStashPop,
            ),
            (
                "Drop stash",
                "D",
                Some((GraphStash, key(Char('D')))),
                Action::GraphStashDrop,
            ),
            (
                "Open in editor",
                "e",
                Some((Normal, key(Char('e')))),
                Action::Edit,
            ),
            (
                "Open in diff tool",
                "E",
                Some((Normal, key(Char('E')))),
                Action::ExternalDiff,
            ),
            (
                "Refresh",
                "r",
                Some((Normal, key(Char('r')))),
                Action::Refresh,
            ),
            (
                "Open PR",
                "gx",
                Some((GChord, key(Char('x')))),
                Action::OpenPullRequest,
            ),
            (
                "Open PR CI",
                "gc",
                Some((GChord, key(Char('c')))),
                Action::OpenPullRequestCi,
            ),
            ("Copy PR URL", "", None, Action::CopyPullRequestUrl),
            ("Refresh this PR", "", None, Action::RefreshPullRequest),
            ("Refresh PRs", "", None, Action::RefreshPullRequests),
            (
                "View remotely",
                "gw",
                Some((GChord, key(Char('w')))),
                Action::ViewRemotely,
            ),
            (
                "Blame: open commit changes",
                "",
                None,
                Action::BlameCommitVsParent,
            ),
            (
                "Blame: open previous line change",
                "",
                None,
                Action::BlamePreviousChange,
            ),
            (
                "Blame: diff commit to working tree",
                "",
                None,
                Action::BlameCommitVsWorktree,
            ),
            (
                "Blame: show commit in graph",
                "",
                None,
                Action::BlameRevealGraph,
            ),
            (
                "Diff vs default in new tab",
                "",
                None,
                Action::CompareVsDefault,
            ),
            (
                "Diff vs branch in new tab…",
                "",
                None,
                Action::CompareVsBranch,
            ),
            (
                "Diff vs commit in new tab…",
                "",
                None,
                Action::CompareVsCommit,
            ),
            (
                "Diff commit vs parent in new tab",
                "",
                None,
                Action::CompareCommitVsParent,
            ),
            (
                "Close tab",
                "Ctrl-w",
                Some((Normal, ctrl(Char('w')))),
                Action::CloseTab,
            ),
            (
                "PRs tab",
                "gp",
                Some((GChord, key(Char('p')))),
                Action::OpenPrsTab,
            ),
            (
                "Mark reviewed",
                "space",
                Some((Normal, key(Char(' ')))),
                Action::ToggleReviewed,
            ),
            (
                "Go to file",
                "Ctrl-p F",
                Some((Normal, ctrl(Char('p')))),
                Action::ToggleQuickOpen(QuickOpenEntry::Files),
            ),
            (
                "Keymap help",
                "?",
                Some((Normal, key(Char('?')))),
                Action::ToggleHelp,
            ),
            (
                "Icon popover",
                "gh",
                Some((GChord, key(Char('h')))),
                Action::PopoverOpenFocused,
            ),
            (
                "Flat / tree",
                "t",
                Some((Normal, key(Char('t')))),
                Action::ToggleTreeMode,
            ),
            (
                "Show hidden",
                ".",
                Some((Normal, key(Char('.')))),
                Action::ToggleShowHidden,
            ),
            (
                "Inline / split",
                "i",
                Some((Normal, key(Char('i')))),
                Action::ToggleDiffMode,
            ),
            (
                "Narrow tree",
                "<",
                Some((Normal, key(Char('<')))),
                Action::ResizeTree(-1),
            ),
            (
                "Widen tree",
                ">",
                Some((Normal, key(Char('>')))),
                Action::ResizeTree(1),
            ),
            (
                "Wrap / unwrap",
                "\\",
                Some((Normal, key(Char('\\')))),
                Action::ToggleDiffWrap,
            ),
            (
                "Collapse / expand commit message",
                "M",
                Some((Normal, key(Char('M')))),
                Action::ToggleCommitMsgExpand,
            ),
            (
                "Shorter commit message",
                "",
                None,
                Action::ResizeCommitMsg(-1),
            ),
            (
                "Taller commit message",
                "",
                None,
                Action::ResizeCommitMsg(1),
            ),
            (
                "Toggle line blame",
                "B",
                Some((Normal, key(Char('B')))),
                Action::ToggleLineBlame,
            ),
            (
                "Toggle mouse",
                "m",
                Some((Normal, key(Char('m')))),
                Action::ToggleMouse,
            ),
            (
                "Toggle activity sparkline",
                "",
                None,
                Action::ToggleActivitySparkline,
            ),
            (
                "Full-file context",
                "Ctrl-o",
                Some((Normal, ctrl(Char('o')))),
                Action::ToggleFullContext,
            ),
            (
                "Graph focus branches",
                "o",
                Some((Normal, key(Char('o')))),
                Action::GraphFocusBranches,
            ),
            (
                "Clear graph focus",
                "O",
                Some((Normal, key(Char('O')))),
                Action::GraphFocusClear,
            ),
            (
                "Comment",
                ";",
                Some((Normal, key(Char(';')))),
                Action::CommentStart,
            ),
            (
                "Highlight diff lines",
                "V",
                Some((Normal, key(Char('V')))),
                Action::DiffVisualStart,
            ),
            (
                "Copy comments",
                "y",
                Some((Normal, key(Char('y')))),
                Action::ExportComments,
            ),
            (
                "Copy entity reference",
                "'",
                Some((Normal, key(Char('\'')))),
                Action::CopyEntityReference,
            ),
            ("Doctor: setup checkup", "", None, Action::OpenDoctor),
            ("Quit", "q", Some((Normal, key(Char('q')))), Action::Quit),
        ];

        // The `Theme: …` rows have their own test in `command_palette`.
        let catalog: Vec<&str> = PALETTE_COMMANDS
            .iter()
            .filter(|command| !matches!(command.action, Action::SetTheme(_)))
            .map(|command| command.title)
            .collect();
        let covered: Vec<&str> = cases.iter().map(|(title, ..)| *title).collect();
        assert_eq!(
            covered, catalog,
            "one case per non-theme row, in catalog order"
        );

        for (title, chip, key_press, expected) in &cases {
            let (keys, action) = palette_row(title);
            assert_eq!(action.as_ref(), Some(expected), "{title}: palette action");
            assert_eq!(keys, *chip, "{title}: palette chip");
            if let Some((ctx, event)) = key_press {
                assert_eq!(&press(*ctx, event), expected, "{title}: {chip} in {ctx:?}");
            }
        }
    }

    /// The key event a chip names (a chord chip: its second key).
    fn chip_event(chip: &str) -> Event {
        if let Some(c) = chip.strip_prefix("Ctrl-") {
            return ctrl(KeyCode::Char(c.chars().next().expect("Ctrl- chip key")));
        }
        key(match chip {
            "space" => KeyCode::Char(' '),
            "Tab" => KeyCode::Tab,
            "Esc" => KeyCode::Esc,
            "Enter" => KeyCode::Enter,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "PgUp" => KeyCode::PageUp,
            "PgDn" => KeyCode::PageDown,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            _ => KeyCode::Char(chip.chars().last().expect("chip key")),
        })
    }

    /// Each keymap table agrees with the keymap: every chip and other key,
    /// pressed in the table's context, runs that binding's action, and no
    /// table names one key twice.
    #[test]
    fn every_table_binding_runs_its_action_in_its_context() {
        let tables: [(&str, &[KeyBinding], Ctx); 8] = [
            ("NORMAL_KEYS", NORMAL_KEYS, Ctx::Normal),
            ("NORMAL_CTRL_KEYS", NORMAL_CTRL_KEYS, Ctx::Normal),
            ("GRAPH_STASH_KEYS", GRAPH_STASH_KEYS, Ctx::GraphStash),
            ("GRAPH_COMMIT_KEYS", GRAPH_COMMIT_KEYS, Ctx::GraphCommit),
            ("G_CHORD_KEYS", G_CHORD_KEYS, Ctx::GChord),
            ("FOLD_SUBTREE", &[FOLD_SUBTREE], Ctx::ZChord),
            ("HIGHLIGHT_KEYS", HIGHLIGHT_KEYS, Ctx::Highlight),
            ("HIGHLIGHT_CTRL_KEYS", HIGHLIGHT_CTRL_KEYS, Ctx::Highlight),
        ];
        for (name, table, ctx) in tables {
            let mut seen = HashSet::new();
            for binding in table {
                for chip in std::iter::once(&binding.chip).chain(binding.other_keys) {
                    assert!(seen.insert(*chip), "{name}: {chip} is bound twice");
                    assert_eq!(
                        press(ctx, &chip_event(chip)),
                        binding.action,
                        "{name}: {chip} in {ctx:?}"
                    );
                }
            }
        }
    }
}
