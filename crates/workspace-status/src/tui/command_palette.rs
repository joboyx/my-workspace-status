//! Named-command overlay (`Ctrl-k` / `:`).
//!
//! Filter is case-insensitive substring on title, key chips, and group.
//! Execute is close-then-dispatch through [`super::state::AppState::dispatch`].

use super::action::{Action, PaletteOpenedBy};

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
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Unstage highlighted lines",
        keys: "u",
        group: CommandGroup::Highlight,
        action: Action::Unstage,
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Revert highlighted lines",
        keys: "x",
        group: CommandGroup::Highlight,
        action: Action::Revert,
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Exit highlight",
        keys: "Esc",
        group: CommandGroup::Highlight,
        action: Action::DiffVisualCancel,
        scope: CommandScope::Highlight,
    },
    PaletteCommand {
        title: "Search focused pane",
        keys: "/",
        group: CommandGroup::Move,
        action: Action::SearchStart,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Stage",
        keys: "s",
        group: CommandGroup::Git,
        action: Action::Stage,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Unstage",
        keys: "u",
        group: CommandGroup::Git,
        action: Action::Unstage,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Revert",
        keys: "x",
        group: CommandGroup::Git,
        action: Action::Revert,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Stash menu",
        keys: "S",
        group: CommandGroup::Git,
        action: Action::StashMenu,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Fetch remotes",
        keys: "f",
        group: CommandGroup::Git,
        action: Action::Fetch,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Pull behind",
        keys: "p",
        group: CommandGroup::Git,
        action: Action::Pull,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Push",
        keys: "P",
        group: CommandGroup::Git,
        action: Action::Push,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Default branch",
        keys: "d",
        group: CommandGroup::Git,
        action: Action::DefaultBranch,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Branch picker",
        keys: "b",
        group: CommandGroup::Git,
        action: Action::Branch,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Checkout commit refs",
        keys: "b",
        group: CommandGroup::Git,
        action: Action::GraphCheckout,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Create branch at commit",
        keys: "c",
        group: CommandGroup::Git,
        action: Action::GraphCreateBranch,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Merge into HEAD",
        keys: "m",
        group: CommandGroup::Git,
        action: Action::GraphMerge,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Remove worktree",
        keys: "W",
        group: CommandGroup::Git,
        action: Action::RemoveWorktree,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Apply stash",
        keys: "a",
        group: CommandGroup::Git,
        action: Action::GraphStashApply,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Pop stash",
        keys: "p",
        group: CommandGroup::Git,
        action: Action::GraphStashPop,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Drop stash",
        keys: "D",
        group: CommandGroup::Git,
        action: Action::GraphStashDrop,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Open in editor",
        keys: "e",
        group: CommandGroup::Git,
        action: Action::Edit,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Open in diff tool",
        keys: "E",
        group: CommandGroup::Git,
        action: Action::ExternalDiff,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Refresh",
        keys: "r",
        group: CommandGroup::Git,
        action: Action::Refresh,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Diff vs default",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CompareVsDefault,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Diff vs branch…",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CompareVsBranch,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Close tab",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CloseCompareTab,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Mark reviewed",
        keys: "space",
        group: CommandGroup::Git,
        action: Action::ToggleReviewed,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Keymap help",
        keys: "?",
        group: CommandGroup::View,
        action: Action::ToggleHelp,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Cycle theme",
        keys: "T",
        group: CommandGroup::View,
        action: Action::CycleTheme,
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Flat / tree",
        keys: "t",
        group: CommandGroup::View,
        action: Action::ToggleTreeMode,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Show ignored",
        keys: ".",
        group: CommandGroup::View,
        action: Action::ToggleShowIgnored,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Inline / split",
        keys: "i",
        group: CommandGroup::View,
        action: Action::ToggleDiffMode,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Wrap / unwrap",
        keys: "\\",
        group: CommandGroup::View,
        action: Action::ToggleDiffWrap,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Expand commit message",
        keys: "M",
        group: CommandGroup::View,
        action: Action::ToggleCommitMsgExpand,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Toggle mouse",
        keys: "m",
        group: CommandGroup::View,
        action: Action::ToggleMouse,
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Full-file context",
        keys: "Ctrl-o",
        group: CommandGroup::View,
        action: Action::ToggleFullContext,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Graph focus branches",
        keys: "o",
        group: CommandGroup::View,
        action: Action::GraphFocusBranches,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Clear graph focus",
        keys: "O",
        group: CommandGroup::View,
        action: Action::GraphFocusClear,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Comment",
        keys: ";",
        group: CommandGroup::View,
        action: Action::CommentStart,
        scope: CommandScope::Any,
    },
    PaletteCommand {
        title: "Highlight diff lines",
        keys: "V",
        group: CommandGroup::View,
        action: Action::DiffVisualStart,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Copy comments",
        keys: "y",
        group: CommandGroup::View,
        action: Action::ExportComments,
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Copy entity reference",
        keys: "'",
        group: CommandGroup::View,
        action: Action::CopyEntityReference,
        scope: CommandScope::Any,
    },
];

/// Case-insensitive substring on title, key chips, and group.
pub fn command_matches(command: &PaletteCommand, query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    command.title.to_ascii_lowercase().contains(&q)
        || command.keys.to_ascii_lowercase().contains(&q)
        || command.group.title().to_ascii_lowercase().contains(&q)
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPaletteState {
    /// Key that opened the overlay (`:` vs Ctrl-k prompt prefix).
    pub opened_by: PaletteOpenedBy,
    /// Filter query (substring).
    pub filter: String,
    /// Highlight index into [`Self::visible`].
    pub cursor: usize,
}

impl CommandPaletteState {
    /// Empty filter, cursor on row 0. `AppState` then moves it to the first
    /// enabled row.
    pub fn new(opened_by: PaletteOpenedBy) -> Self {
        Self {
            opened_by,
            filter: String::new(),
            cursor: 0,
        }
    }

    /// Commands that match the current filter, table order.
    pub fn visible(&self) -> Vec<&'static PaletteCommand> {
        filter_commands(&self.filter)
    }

    /// Highlighted command, if the filtered list is non-empty.
    pub fn selected(&self) -> Option<&'static PaletteCommand> {
        self.visible().get(self.cursor).copied()
    }

    /// Mapped [`Action`] for the highlighted command, if any.
    pub fn selected_action(&self) -> Option<&Action> {
        self.selected().map(|command| &command.action)
    }

    /// Clamp like the branch picker.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, len as i32 - 1) as usize;
    }

    /// Append a filter character and clamp the highlight.
    pub fn push_char(&mut self, c: char) {
        self.filter.push(c);
        self.clamp_cursor();
    }

    /// Delete the last filter character and clamp the highlight.
    pub fn backspace(&mut self) {
        self.filter.pop();
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
                "Diff vs default",
                "",
                CommandGroup::Git,
                Action::CompareVsDefault,
                CommandScope::NoHighlight,
            ),
            (
                "Diff vs branch…",
                "",
                CommandGroup::Git,
                Action::CompareVsBranch,
                CommandScope::NoHighlight,
            ),
            (
                "Close tab",
                "",
                CommandGroup::Git,
                Action::CloseCompareTab,
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
                "Wrap / unwrap",
                "\\",
                CommandGroup::View,
                Action::ToggleDiffWrap,
                CommandScope::NoHighlight,
            ),
            (
                "Expand commit message",
                "M",
                CommandGroup::View,
                Action::ToggleCommitMsgExpand,
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
        let palette = CommandPaletteState::new(PaletteOpenedBy::Colon);
        assert_eq!(
            palette.paint_rows().first(),
            Some(&PalettePaintRow::Header("HIGHLIGHT"))
        );
        assert_eq!(
            palette.selected().map(|c| c.title),
            Some("Stage highlighted lines")
        );
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
        let mut palette = CommandPaletteState::new(PaletteOpenedBy::Colon);
        palette.filter = "keymap".into();
        palette.clamp_cursor();
        palette.move_cursor(20);
        assert_eq!(palette.selected().map(|c| c.title), Some("Keymap help"));
        palette.move_cursor(-20);
        assert_eq!(palette.selected().map(|c| c.title), Some("Keymap help"));
    }
}
