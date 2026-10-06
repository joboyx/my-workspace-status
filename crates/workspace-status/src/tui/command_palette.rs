//! The commands mode of Quick Open (`:`, or `>` typed first in files mode).
//!
//! Filter is case-insensitive substring on title, key chips, group, and
//! aliases (`exit` finds Quit, `compare` the Diff … in new tab rows).
//! Execute is close-then-dispatch through [`super::state::AppState::dispatch`].

use super::action::Action;

/// Palette group names (HIGHLIGHT, then the help columns MOVE / GIT / VIEW).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandGroup {
    Highlight,
    Move,
    Git,
    View,
}

/// Groups in paint order.
const GROUP_ORDER: [CommandGroup; 4] = [
    CommandGroup::Highlight,
    CommandGroup::Move,
    CommandGroup::Git,
    CommandGroup::View,
];

impl CommandGroup {
    /// Overlay / help column title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Highlight => "HIGHLIGHT",
            Self::Move => "MOVE",
            Self::Git => "GIT",
            Self::View => "VIEW",
        }
    }
}

/// When a palette row can run, relative to visual-line diff highlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandScope {
    /// Runs with or without a highlighted range.
    Any,
    /// Runs only while a diff range is highlighted.
    Highlight,
    /// Runs only while no diff range is highlighted.
    NoHighlight,
}

impl CommandScope {
    /// True when a row with this scope may run in the current highlight mode.
    pub fn fits(self, highlighted: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Highlight => highlighted,
            Self::NoHighlight => !highlighted,
        }
    }
}

/// One named command in the palette catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteCommand {
    /// User-facing title (filter target).
    pub title: &'static str,
    /// Key chips shown in the row (filter target).
    pub keys: &'static str,
    /// HIGHLIGHT / MOVE / GIT / VIEW.
    pub group: CommandGroup,
    /// Dispatched after the palette closes.
    pub action: Action,
    /// Other words that find this row (filter target), e.g. `exit` for Quit.
    pub aliases: &'static [&'static str],
    /// Highlight mode in which the row is enabled.
    pub scope: CommandScope,
}

/// Named commands only. Pointer / overlay-internal actions stay out.
pub const PALETTE_COMMANDS: &[PaletteCommand] = &[
    PaletteCommand {
        title: "Stage highlighted lines",
        keys: "s",
        group: CommandGroup::Highlight,
        action: Action::Stage,
        aliases: &["add"],
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Unstage highlighted lines",
        keys: "u",
        group: CommandGroup::Highlight,
        action: Action::Unstage,
        aliases: &["reset"],
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Revert highlighted lines",
        keys: "x",
        group: CommandGroup::Highlight,
        action: Action::Revert,
        aliases: &["discard", "restore"],
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Exit highlight",
        keys: "Esc",
        group: CommandGroup::Highlight,
        action: Action::DiffVisualCancel,
        aliases: &["cancel"],
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Search focused pane",
        keys: "/",
        group: CommandGroup::Move,
        action: Action::SearchStart,
        aliases: &["find", "grep"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Next match",
        keys: "n",
        group: CommandGroup::Move,
        action: Action::SearchNext,
        aliases: &["find next", "search"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Previous match",
        keys: "N",
        group: CommandGroup::Move,
        action: Action::SearchPrev,
        aliases: &["find previous", "search"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Fold row",
        keys: "z",
        group: CommandGroup::Move,
        action: Action::FoldToggle,
        aliases: &["collapse", "expand", "unfold"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Fold subtree",
        keys: "zz",
        group: CommandGroup::Move,
        action: Action::FoldToggleSubtree,
        aliases: &["collapse", "expand", "unfold"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Other pane",
        keys: "Tab",
        group: CommandGroup::Move,
        // Palette Enter runs FocusLeft instead while the right pane has focus.
        action: Action::FocusRight,
        aliases: &["switch pane", "focus"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Next tab",
        keys: "gt",
        group: CommandGroup::Move,
        action: Action::NextTab,
        aliases: &["switch tab"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Previous tab",
        keys: "gT",
        group: CommandGroup::Move,
        action: Action::PreviousTab,
        aliases: &["switch tab"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Stage",
        keys: "s",
        group: CommandGroup::Git,
        action: Action::Stage,
        aliases: &["add"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Unstage",
        keys: "u",
        group: CommandGroup::Git,
        action: Action::Unstage,
        aliases: &["reset"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Revert",
        keys: "x",
        group: CommandGroup::Git,
        action: Action::Revert,
        aliases: &["discard", "restore"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Stash menu",
        keys: "S",
        group: CommandGroup::Git,
        action: Action::StashMenu,
        aliases: &["save", "shelve"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Fetch remotes",
        keys: "f",
        group: CommandGroup::Git,
        action: Action::Fetch,
        aliases: &["download", "update"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Pull behind",
        keys: "p",
        group: CommandGroup::Git,
        action: Action::Pull,
        aliases: &["update", "sync"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Push",
        keys: "P",
        group: CommandGroup::Git,
        action: Action::Push,
        aliases: &["upload", "publish"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Default branch",
        keys: "d",
        group: CommandGroup::Git,
        action: Action::DefaultBranch,
        aliases: &["main", "switch"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Branch picker",
        keys: "b",
        group: CommandGroup::Git,
        action: Action::Branch,
        aliases: &["checkout", "switch", "create branch", "new branch"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Checkout commit refs",
        keys: "b",
        group: CommandGroup::Git,
        action: Action::GraphCheckout,
        aliases: &["checkout", "switch"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Create branch at commit",
        keys: "c",
        group: CommandGroup::Git,
        action: Action::GraphCreateBranch,
        aliases: &["new branch"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Merge into HEAD",
        keys: "m",
        group: CommandGroup::Git,
        action: Action::GraphMerge,
        aliases: &["integrate"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Remove worktree",
        keys: "W",
        group: CommandGroup::Git,
        action: Action::RemoveWorktree,
        aliases: &["worktree", "delete"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Apply stash",
        keys: "a",
        group: CommandGroup::Git,
        action: Action::GraphStashApply,
        aliases: &["unstash"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Pop stash",
        keys: "p",
        group: CommandGroup::Git,
        action: Action::GraphStashPop,
        aliases: &["unstash"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Drop stash",
        keys: "D",
        group: CommandGroup::Git,
        action: Action::GraphStashDrop,
        aliases: &["delete"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Open in editor",
        keys: "e",
        group: CommandGroup::Git,
        action: Action::Edit,
        aliases: &["edit", "vim"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Open in diff tool",
        keys: "E",
        group: CommandGroup::Git,
        action: Action::ExternalDiff,
        aliases: &["difftool", "vimdiff"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Refresh",
        keys: "r",
        group: CommandGroup::Git,
        action: Action::Refresh,
        aliases: &["reload", "rescan"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Open PR",
        keys: "gx",
        group: CommandGroup::Git,
        action: Action::OpenPullRequest,
        aliases: &[
            "pull request",
            "merge request",
            "browser",
            "github",
            "gitlab",
        ],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Blame: open commit changes",
        keys: "",
        group: CommandGroup::Git,
        action: Action::BlameCommitVsParent,
        aliases: &["blame", "compare", "gitlens"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Blame: open previous line change",
        keys: "",
        group: CommandGroup::Git,
        action: Action::BlamePreviousChange,
        aliases: &["blame", "compare", "previous revision", "gitlens"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Blame: diff commit to working tree",
        keys: "",
        group: CommandGroup::Git,
        action: Action::BlameCommitVsWorktree,
        aliases: &["blame", "compare", "gitlens"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Blame: show commit in graph",
        keys: "",
        group: CommandGroup::Git,
        action: Action::BlameRevealGraph,
        aliases: &["blame", "reveal", "details", "gitlens"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Diff vs default in new tab",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CompareVsDefault,
        aliases: &["compare"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Diff vs branch in new tab…",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CompareVsBranch,
        aliases: &["compare"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Diff vs commit in new tab…",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CompareVsCommit,
        aliases: &["compare"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Diff commit vs parent in new tab",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CompareCommitVsParent,
        aliases: &["compare"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Close tab",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CloseTab,
        aliases: &["close compare"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Mark reviewed",
        keys: "space",
        group: CommandGroup::Git,
        action: Action::ToggleReviewed,
        aliases: &["viewed", "seen"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Keymap help",
        keys: "?",
        group: CommandGroup::View,
        action: Action::ToggleHelp,
        aliases: &["help", "keys", "shortcuts"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Cycle theme",
        keys: "T",
        group: CommandGroup::View,
        action: Action::CycleTheme,
        aliases: &["theme", "color", "colour"],
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Flat / tree",
        keys: "t",
        group: CommandGroup::View,
        action: Action::ToggleTreeMode,
        aliases: &["list"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Show ignored",
        keys: ".",
        group: CommandGroup::View,
        action: Action::ToggleShowIgnored,
        aliases: &["hidden"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Inline / split",
        keys: "i",
        group: CommandGroup::View,
        action: Action::ToggleDiffMode,
        aliases: &["side by side", "unified"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Narrow tree",
        keys: "<",
        group: CommandGroup::View,
        action: Action::ResizeTree(-1),
        aliases: &["widen diff", "pane width", "resize", "shrink"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Widen tree",
        keys: ">",
        group: CommandGroup::View,
        action: Action::ResizeTree(1),
        aliases: &["narrow diff", "pane width", "resize", "grow"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Wrap / unwrap",
        keys: "\\",
        group: CommandGroup::View,
        action: Action::ToggleDiffWrap,
        aliases: &["soft wrap"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Collapse / expand commit message",
        keys: "M",
        group: CommandGroup::View,
        action: Action::ToggleCommitMsgExpand,
        aliases: &["body"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Shorter commit message",
        keys: "-",
        group: CommandGroup::View,
        action: Action::ResizeCommitMsg(-1),
        aliases: &["message height", "msg lines", "shrink"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Taller commit message",
        keys: "+",
        group: CommandGroup::View,
        action: Action::ResizeCommitMsg(1),
        aliases: &["message height", "msg lines", "grow"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Line blame on / off",
        keys: "B",
        group: CommandGroup::View,
        action: Action::ToggleLineBlame,
        aliases: &["blame", "gitlens"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Toggle mouse",
        keys: "m",
        group: CommandGroup::View,
        action: Action::ToggleMouse,
        aliases: &["pointer", "click"],
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Full-file context",
        keys: "Ctrl-o",
        group: CommandGroup::View,
        action: Action::ToggleFullContext,
        aliases: &["expand"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Graph focus branches",
        keys: "o",
        group: CommandGroup::View,
        action: Action::GraphFocusBranches,
        aliases: &["filter graph"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Clear graph focus",
        keys: "O",
        group: CommandGroup::View,
        action: Action::GraphFocusClear,
        aliases: &["full graph", "unfocus"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Comment",
        keys: ";",
        group: CommandGroup::View,
        action: Action::CommentStart,
        aliases: &["note", "annotate"],
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Highlight diff lines",
        keys: "V",
        group: CommandGroup::View,
        action: Action::DiffVisualStart,
        aliases: &["visual", "select lines"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Copy comments",
        keys: "y",
        group: CommandGroup::View,
        action: Action::ExportComments,
        aliases: &["yank", "export"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Copy entity reference",
        keys: "'",
        group: CommandGroup::View,
        action: Action::CopyEntityReference,
        aliases: &["yank", "link"],
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Quit",
        keys: "q",
        group: CommandGroup::View,
        action: Action::Quit,
        aliases: &["exit", "close app"],
        scope: CommandScope::Any,
    },
];

/// The catalog row that runs `action`, if any.
///
/// Icon popovers take an action line's title, key chip, and disabled
/// reason from this row, so a popover never names a key the palette does
/// not.
pub fn command_for(action: &Action) -> Option<&'static PaletteCommand> {
    PALETTE_COMMANDS
        .iter()
        .find(|command| &command.action == action)
}

/// Case-insensitive substring on title, key chips, group, and aliases.
pub fn command_matches(command: &PaletteCommand, query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    command_matches_by_name(command, &q)
        || command
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().contains(&q))
}

/// Case-insensitive substring on title, key chips, and group (not aliases).
///
/// `query` is already trimmed and lowercase. An empty query matches.
fn command_matches_by_name(command: &PaletteCommand, q: &str) -> bool {
    q.is_empty()
        || command.title.to_ascii_lowercase().contains(q)
        || command.keys.to_ascii_lowercase().contains(q)
        || command.group.title().to_ascii_lowercase().contains(q)
}

/// Filtered catalog in table order. Empty query keeps every command.
pub fn filter_commands(query: &str) -> Vec<&'static PaletteCommand> {
    PALETTE_COMMANDS
        .iter()
        .filter(|command| command_matches(command, query))
        .collect()
}

#[cfg(test)]
/// Groups that still have a hit. Empty query keeps every group.
pub fn visible_groups(query: &str) -> Vec<CommandGroup> {
    let q = query.trim();
    if q.is_empty() {
        return GROUP_ORDER.to_vec();
    }
    let mut out = Vec::new();
    for group in GROUP_ORDER {
        if PALETTE_COMMANDS
            .iter()
            .any(|command| command.group == group && command_matches(command, query))
        {
            out.push(group);
        }
    }
    out
}

/// Interactive palette state (filter + highlight).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandPaletteState {
    /// Filter query (substring).
    pub filter: String,
    /// Highlight index into [`Self::visible`].
    pub cursor: usize,
    /// Disabled-row reason that Enter put on the status line. Closing the
    /// palette clears the status while it still shows this text.
    pub shown_reason: Option<String>,
}

impl CommandPaletteState {
    /// Empty filter, cursor on row 0. `AppState` then moves it to the first
    /// enabled row.
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            cursor: 0,
            shown_reason: None,
        }
    }

    /// Commands that match the current filter, table order.
    pub fn visible(&self) -> Vec<&'static PaletteCommand> {
        filter_commands(&self.filter)
    }

    /// Indexes into [`Self::visible`] that the cursor may land on after a
    /// filter change. `highlighted` is true while a diff range is
    /// highlighted.
    ///
    /// Rows the filter finds by title, key chips, or group, and whose scope
    /// fits the highlight mode, outrank rows it finds only by an alias:
    /// typing `pull` lands on Pull behind, not on a row that has a
    /// `pull request` alias. With no such row, every visible row may take
    /// the cursor (`exit` outside a highlight lands on Quit, not on Exit
    /// highlight).
    pub fn landing_rows(&self, highlighted: bool) -> Vec<usize> {
        let visible = self.visible();
        let q = self.filter.trim().to_ascii_lowercase();
        let named: Vec<usize> = visible
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                command_matches_by_name(command, &q) && command.scope.fits(highlighted)
            })
            .map(|(index, _)| index)
            .collect();
        if named.is_empty() {
            (0..visible.len()).collect()
        } else {
            named
        }
    }

    /// Highlighted command, if the filtered list is non-empty.
    pub fn selected(&self) -> Option<&'static PaletteCommand> {
        self.visible().get(self.cursor).copied()
    }

    /// Mapped [`Action`] for the highlighted command, if any.
    pub fn selected_action(&self) -> Option<&Action> {
        self.selected().map(|command| &command.action)
    }

    /// Move the highlight by `delta`, clamped like the branch picker.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, len as i32 - 1) as usize;
    }

    /// Replace the filter query and clamp the highlight.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
        }
    }

    /// Group headers plus commands in table order (empty groups omitted when filtering).
    pub fn paint_rows(&self) -> Vec<PalettePaintRow> {
        let visible = self.visible();
        let mut out = Vec::new();
        if visible.is_empty() {
            if self.filter.trim().is_empty() {
                for group in GROUP_ORDER {
                    out.push(PalettePaintRow::Header(group.title()));
                }
            }
            return out;
        }
        let mut last = None;
        for (index, command) in visible.iter().enumerate() {
            if last != Some(command.group) {
                out.push(PalettePaintRow::Header(command.group.title()));
                last = Some(command.group);
            }
            out.push(PalettePaintRow::Command { command, index });
        }
        out
    }
}

/// One painted palette line (group header or command).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PalettePaintRow {
    /// HIGHLIGHT / MOVE / GIT / VIEW heading.
    Header(&'static str),
    /// Catalog row. `index` is into [`CommandPaletteState::visible`].
    Command {
        command: &'static PaletteCommand,
        /// Index into [`CommandPaletteState::visible`].
        index: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(query: &str) -> Vec<&'static str> {
        filter_commands(query)
            .into_iter()
            .map(|c| c.title)
            .collect()
    }

    #[test]
    fn catalog_has_required_titles_keys_groups() {
        let wanted = [
            (
                "Stage highlighted lines",
                "s",
                CommandGroup::Highlight,
                Action::Stage,
                CommandScope::Highlight,
            ),
            (
                "Unstage highlighted lines",
                "u",
                CommandGroup::Highlight,
                Action::Unstage,
                CommandScope::Highlight,
            ),
            (
                "Revert highlighted lines",
                "x",
                CommandGroup::Highlight,
                Action::Revert,
                CommandScope::Highlight,
            ),
            (
                "Exit highlight",
                "Esc",
                CommandGroup::Highlight,
                Action::DiffVisualCancel,
                CommandScope::Highlight,
            ),
            (
                "Search focused pane",
                "/",
                CommandGroup::Move,
                Action::SearchStart,
                CommandScope::NoHighlight,
            ),
            (
                "Next match",
                "n",
                CommandGroup::Move,
                Action::SearchNext,
                CommandScope::NoHighlight,
            ),
            (
                "Previous match",
                "N",
                CommandGroup::Move,
                Action::SearchPrev,
                CommandScope::NoHighlight,
            ),
            (
                "Fold row",
                "z",
                CommandGroup::Move,
                Action::FoldToggle,
                CommandScope::NoHighlight,
            ),
            (
                "Fold subtree",
                "zz",
                CommandGroup::Move,
                Action::FoldToggleSubtree,
                CommandScope::NoHighlight,
            ),
            (
                "Other pane",
                "Tab",
                CommandGroup::Move,
                Action::FocusRight,
                CommandScope::NoHighlight,
            ),
            (
                "Next tab",
                "gt",
                CommandGroup::Move,
                Action::NextTab,
                CommandScope::NoHighlight,
            ),
            (
                "Previous tab",
                "gT",
                CommandGroup::Move,
                Action::PreviousTab,
                CommandScope::NoHighlight,
            ),
            (
                "Stage",
                "s",
                CommandGroup::Git,
                Action::Stage,
                CommandScope::NoHighlight,
            ),
            (
                "Unstage",
                "u",
                CommandGroup::Git,
                Action::Unstage,
                CommandScope::NoHighlight,
            ),
            (
                "Revert",
                "x",
                CommandGroup::Git,
                Action::Revert,
                CommandScope::NoHighlight,
            ),
            (
                "Stash menu",
                "S",
                CommandGroup::Git,
                Action::StashMenu,
                CommandScope::NoHighlight,
            ),
            (
                "Fetch remotes",
                "f",
                CommandGroup::Git,
                Action::Fetch,
                CommandScope::NoHighlight,
            ),
            (
                "Pull behind",
                "p",
                CommandGroup::Git,
                Action::Pull,
                CommandScope::NoHighlight,
            ),
            (
                "Push",
                "P",
                CommandGroup::Git,
                Action::Push,
                CommandScope::NoHighlight,
            ),
            (
                "Default branch",
                "d",
                CommandGroup::Git,
                Action::DefaultBranch,
                CommandScope::NoHighlight,
            ),
            (
                "Branch picker",
                "b",
                CommandGroup::Git,
                Action::Branch,
                CommandScope::NoHighlight,
            ),
            (
                "Checkout commit refs",
                "b",
                CommandGroup::Git,
                Action::GraphCheckout,
                CommandScope::NoHighlight,
            ),
            (
                "Create branch at commit",
                "c",
                CommandGroup::Git,
                Action::GraphCreateBranch,
                CommandScope::NoHighlight,
            ),
            (
                "Merge into HEAD",
                "m",
                CommandGroup::Git,
                Action::GraphMerge,
                CommandScope::NoHighlight,
            ),
            (
                "Remove worktree",
                "W",
                CommandGroup::Git,
                Action::RemoveWorktree,
                CommandScope::NoHighlight,
            ),
            (
                "Apply stash",
                "a",
                CommandGroup::Git,
                Action::GraphStashApply,
                CommandScope::NoHighlight,
            ),
            (
                "Pop stash",
                "p",
                CommandGroup::Git,
                Action::GraphStashPop,
                CommandScope::NoHighlight,
            ),
            (
                "Drop stash",
                "D",
                CommandGroup::Git,
                Action::GraphStashDrop,
                CommandScope::NoHighlight,
            ),
            (
                "Open in editor",
                "e",
                CommandGroup::Git,
                Action::Edit,
                CommandScope::NoHighlight,
            ),
            (
                "Open in diff tool",
                "E",
                CommandGroup::Git,
                Action::ExternalDiff,
                CommandScope::NoHighlight,
            ),
            (
                "Refresh",
                "r",
                CommandGroup::Git,
                Action::Refresh,
                CommandScope::NoHighlight,
            ),
            (
                "Open PR",
                "gx",
                CommandGroup::Git,
                Action::OpenPullRequest,
                CommandScope::NoHighlight,
            ),
            (
                "Blame: open commit changes",
                "",
                CommandGroup::Git,
                Action::BlameCommitVsParent,
                CommandScope::NoHighlight,
            ),
            (
                "Blame: open previous line change",
                "",
                CommandGroup::Git,
                Action::BlamePreviousChange,
                CommandScope::NoHighlight,
            ),
            (
                "Blame: diff commit to working tree",
                "",
                CommandGroup::Git,
                Action::BlameCommitVsWorktree,
                CommandScope::NoHighlight,
            ),
            (
                "Blame: show commit in graph",
                "",
                CommandGroup::Git,
                Action::BlameRevealGraph,
                CommandScope::NoHighlight,
            ),
            (
                "Diff vs default in new tab",
                "",
                CommandGroup::Git,
                Action::CompareVsDefault,
                CommandScope::NoHighlight,
            ),
            (
                "Diff vs branch in new tab…",
                "",
                CommandGroup::Git,
                Action::CompareVsBranch,
                CommandScope::NoHighlight,
            ),
            (
                "Diff vs commit in new tab…",
                "",
                CommandGroup::Git,
                Action::CompareVsCommit,
                CommandScope::NoHighlight,
            ),
            (
                "Diff commit vs parent in new tab",
                "",
                CommandGroup::Git,
                Action::CompareCommitVsParent,
                CommandScope::NoHighlight,
            ),
            (
                "Close tab",
                "",
                CommandGroup::Git,
                Action::CloseTab,
                CommandScope::NoHighlight,
            ),
            (
                "Mark reviewed",
                "space",
                CommandGroup::Git,
                Action::ToggleReviewed,
                CommandScope::NoHighlight,
            ),
            (
                "Keymap help",
                "?",
                CommandGroup::View,
                Action::ToggleHelp,
                CommandScope::NoHighlight,
            ),
            (
                "Cycle theme",
                "T",
                CommandGroup::View,
                Action::CycleTheme,
                CommandScope::Any,
            ),
            (
                "Flat / tree",
                "t",
                CommandGroup::View,
                Action::ToggleTreeMode,
                CommandScope::NoHighlight,
            ),
            (
                "Show ignored",
                ".",
                CommandGroup::View,
                Action::ToggleShowIgnored,
                CommandScope::NoHighlight,
            ),
            (
                "Inline / split",
                "i",
                CommandGroup::View,
                Action::ToggleDiffMode,
                CommandScope::NoHighlight,
            ),
            (
                "Narrow tree",
                "<",
                CommandGroup::View,
                Action::ResizeTree(-1),
                CommandScope::NoHighlight,
            ),
            (
                "Widen tree",
                ">",
                CommandGroup::View,
                Action::ResizeTree(1),
                CommandScope::NoHighlight,
            ),
            (
                "Wrap / unwrap",
                "\\",
                CommandGroup::View,
                Action::ToggleDiffWrap,
                CommandScope::NoHighlight,
            ),
            (
                "Collapse / expand commit message",
                "M",
                CommandGroup::View,
                Action::ToggleCommitMsgExpand,
                CommandScope::NoHighlight,
            ),
            (
                "Shorter commit message",
                "-",
                CommandGroup::View,
                Action::ResizeCommitMsg(-1),
                CommandScope::NoHighlight,
            ),
            (
                "Taller commit message",
                "+",
                CommandGroup::View,
                Action::ResizeCommitMsg(1),
                CommandScope::NoHighlight,
            ),
            (
                "Line blame on / off",
                "B",
                CommandGroup::View,
                Action::ToggleLineBlame,
                CommandScope::NoHighlight,
            ),
            (
                "Toggle mouse",
                "m",
                CommandGroup::View,
                Action::ToggleMouse,
                CommandScope::Any,
            ),
            (
                "Full-file context",
                "Ctrl-o",
                CommandGroup::View,
                Action::ToggleFullContext,
                CommandScope::NoHighlight,
            ),
            (
                "Graph focus branches",
                "o",
                CommandGroup::View,
                Action::GraphFocusBranches,
                CommandScope::NoHighlight,
            ),
            (
                "Clear graph focus",
                "O",
                CommandGroup::View,
                Action::GraphFocusClear,
                CommandScope::NoHighlight,
            ),
            (
                "Comment",
                ";",
                CommandGroup::View,
                Action::CommentStart,
                CommandScope::Any,
            ),
            (
                "Highlight diff lines",
                "V",
                CommandGroup::View,
                Action::DiffVisualStart,
                CommandScope::NoHighlight,
            ),
            (
                "Copy comments",
                "y",
                CommandGroup::View,
                Action::ExportComments,
                CommandScope::NoHighlight,
            ),
            (
                "Copy entity reference",
                "'",
                CommandGroup::View,
                Action::CopyEntityReference,
                CommandScope::Any,
            ),
            (
                "Quit",
                "q",
                CommandGroup::View,
                Action::Quit,
                CommandScope::Any,
            ),
        ];
        assert_eq!(PALETTE_COMMANDS.len(), wanted.len());
        for (i, (title, keys, group, action, scope)) in wanted.iter().enumerate() {
            let command = &PALETTE_COMMANDS[i];
            assert_eq!(command.title, *title, "row {i}");
            assert_eq!(command.keys, *keys, "{title}");
            assert_eq!(command.group, *group, "{title}");
            assert_eq!(command.action, *action, "{title}");
            assert_eq!(command.scope, *scope, "{title}");
        }
    }

    #[test]
    fn filter_is_case_insensitive_substring_on_title_keys_group() {
        let by_title = titles("keymap");
        assert_eq!(by_title, vec!["Keymap help"]);
        let by_keys = titles("ctrl-o");
        assert_eq!(by_keys, vec!["Full-file context"]);
        let mixed = titles("PULL");
        assert!(mixed.contains(&"Pull behind"));
        assert!(!mixed.contains(&"Pop stash"));
        assert!(titles("move").contains(&"Search focused pane"));
        assert!(PALETTE_COMMANDS
            .iter()
            .filter(|c| c.group == CommandGroup::Move)
            .all(|c| command_matches(c, "move")));
    }

    #[test]
    fn aliases_find_rows_their_titles_do_not_name() {
        for (query, title) in [
            ("compare", "Diff vs default in new tab"),
            ("compare", "Diff vs branch in new tab…"),
            ("compare", "Diff vs commit in new tab…"),
            ("compare", "Diff commit vs parent in new tab"),
            ("quit", "Quit"),
            ("exit", "Quit"),
            ("discard", "Revert"),
            ("restore", "Revert"),
            ("checkout", "Branch picker"),
            ("switch", "Branch picker"),
            ("theme", "Cycle theme"),
            ("color", "Cycle theme"),
            ("mouse", "Toggle mouse"),
            ("reload", "Refresh"),
            ("unstage", "Unstage"),
            ("add", "Stage"),
            ("stash", "Stash menu"),
            ("merge", "Merge into HEAD"),
            ("worktree", "Remove worktree"),
            ("help", "Keymap help"),
            ("keys", "Keymap help"),
            ("find", "Search focused pane"),
            ("wrap", "Wrap / unwrap"),
            ("split", "Inline / split"),
            ("comment", "Comment"),
            ("yank", "Copy comments"),
            ("yank", "Copy entity reference"),
            ("pull request", "Open PR"),
            ("merge request", "Open PR"),
            ("PR", "Open PR"),
            ("browser", "Open PR"),
            ("github", "Open PR"),
            ("gitlab", "Open PR"),
        ] {
            assert!(titles(query).contains(&title), "{query} -> {title}");
        }
        assert_eq!(titles("close app"), vec!["Quit"]);
    }

    /// An alias-only hit never takes the cursor from a row the query names.
    #[test]
    fn landing_rows_rank_name_matches_over_alias_only_matches() {
        let landing_in = |query: &str, highlighted: bool| {
            let mut palette = CommandPaletteState::new();
            palette.set_filter(query);
            let visible = palette.visible();
            palette
                .landing_rows(highlighted)
                .into_iter()
                .map(|index| visible[index].title)
                .collect::<Vec<_>>()
        };
        let landing = |query: &str| landing_in(query, false);
        assert!(titles("pull").contains(&"Open PR"), "alias still finds it");
        assert!(!landing("pull").contains(&"Open PR"));
        assert_eq!(landing("pull").first(), Some(&"Pull behind"));
        assert_eq!(landing("pull request"), vec!["Open PR"]);
        assert_eq!(landing("merge request"), vec!["Open PR"]);
        assert!(landing("PR").contains(&"Open PR"));
        assert_eq!(landing("gx"), vec!["Open PR"]);
        assert!(!landing("merge").contains(&"Open PR"));
        assert_eq!(
            landing("").len(),
            PALETTE_COMMANDS
                .iter()
                .filter(|command| command.scope.fits(false))
                .count()
        );
        // A named row out of scope does not count: `exit` keeps Quit (alias).
        assert!(landing("exit").contains(&"Quit"));
        assert_eq!(landing_in("exit", true), vec!["Exit highlight"]);
    }

    #[test]
    fn filter_typing_j_and_k_matches_rows_like_any_letter() {
        let mut palette = CommandPaletteState::new();
        palette.set_filter("keymap");
        assert_eq!(palette.filter, "keymap");
        assert_eq!(palette.selected().map(|c| c.title), Some("Keymap help"));
        palette.set_filter("j");
        assert!(palette.visible().is_empty(), "no row names a j");
    }

    #[test]
    fn diff_rows_open_in_a_new_tab_and_filters_find_them() {
        assert_eq!(titles("vs default"), vec!["Diff vs default in new tab"]);
        assert_eq!(titles("vs branch"), vec!["Diff vs branch in new tab…"]);
        assert_eq!(titles("vs commit"), vec!["Diff vs commit in new tab…"]);
        assert_eq!(
            titles("vs parent"),
            vec!["Diff commit vs parent in new tab"]
        );
        let diff_rows = vec![
            "Diff vs default in new tab",
            "Diff vs branch in new tab…",
            "Diff vs commit in new tab…",
            "Diff commit vs parent in new tab",
        ];
        assert_eq!(titles("new tab"), diff_rows);
        let compare = titles("compare");
        for title in &diff_rows {
            assert!(compare.contains(title), "compare -> {title}");
        }
        // The diff rows sit together, right before Close tab.
        let catalog: Vec<_> = PALETTE_COMMANDS.iter().map(|c| c.title).collect();
        let close = catalog.iter().position(|t| *t == "Close tab").unwrap();
        assert_eq!(catalog[close - 4..close], diff_rows[..]);
    }

    #[test]
    fn blame_filter_finds_the_blame_rows_and_the_toggle() {
        let blame_rows = vec![
            "Blame: open commit changes",
            "Blame: open previous line change",
            "Blame: diff commit to working tree",
            "Blame: show commit in graph",
            "Line blame on / off",
        ];
        assert_eq!(titles("blame"), blame_rows);
        assert_eq!(titles("gitlens"), blame_rows);
        assert_eq!(titles("previous revision"), vec![blame_rows[1]]);
        assert_eq!(titles("working tree"), vec![blame_rows[2]]);
        assert_eq!(titles("reveal"), vec![blame_rows[3]]);
        // The Blame rows sit right before the Diff … in new tab rows.
        let catalog: Vec<_> = PALETTE_COMMANDS.iter().map(|c| c.title).collect();
        let diff = catalog
            .iter()
            .position(|t| *t == "Diff vs default in new tab")
            .unwrap();
        assert_eq!(catalog[diff - 4..diff], blame_rows[..4]);
    }

    #[test]
    fn close_tab_query_lists_the_close_row() {
        assert_eq!(titles("close tab"), vec!["Close tab"]);
    }

    #[test]
    fn highlight_filter_finds_the_four_highlight_rows() {
        assert_eq!(
            titles("highlighted"),
            vec![
                "Stage highlighted lines",
                "Unstage highlighted lines",
                "Revert highlighted lines",
            ]
        );
        let highlight = titles("highlight");
        for title in [
            "Stage highlighted lines",
            "Unstage highlighted lines",
            "Revert highlighted lines",
            "Exit highlight",
        ] {
            assert!(highlight.contains(&title), "{title}: {highlight:?}");
        }
        assert_eq!(visible_groups("highlight")[0], CommandGroup::Highlight);
    }

    #[test]
    fn highlight_group_paints_first() {
        let palette = CommandPaletteState::new();
        let headers: Vec<&str> = palette
            .paint_rows()
            .into_iter()
            .filter_map(|row| match row {
                PalettePaintRow::Header(title) => Some(title),
                PalettePaintRow::Command { .. } => None,
            })
            .collect();
        assert_eq!(headers, vec!["HIGHLIGHT", "MOVE", "GIT", "VIEW"]);
    }

    #[test]
    fn empty_query_keeps_groups_query_hides_empty() {
        assert_eq!(
            visible_groups(""),
            vec![
                CommandGroup::Highlight,
                CommandGroup::Move,
                CommandGroup::Git,
                CommandGroup::View
            ]
        );
        assert_eq!(visible_groups("keymap"), vec![CommandGroup::View]);
        assert!(visible_groups("zzzz-no-hit").is_empty());
    }

    #[test]
    fn cursor_clamps_like_branch_picker() {
        let mut palette = CommandPaletteState::new();
        palette.filter = "keymap".into();
        palette.clamp_cursor();
        palette.move_cursor(20);
        assert_eq!(palette.selected().map(|c| c.title), Some("Keymap help"));
        palette.move_cursor(-20);
        assert_eq!(palette.selected().map(|c| c.title), Some("Keymap help"));
    }
}
