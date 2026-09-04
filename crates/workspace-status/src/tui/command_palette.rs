//! Named-command overlay (`Ctrl-k` / `:`).
//!
//! Filter is case-insensitive substring on title, key chips, and group.
//! Execute is close-then-dispatch through [`super::state::AppState::dispatch`].

use super::action::{Action, PaletteOpenedBy};

/// Help-column group names (MOVE / GIT / VIEW).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandGroup {
    Move,
    Git,
    View,
}

impl CommandGroup {
    /// Overlay / help column title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Move => "MOVE",
            Self::Git => "GIT",
            Self::View => "VIEW",
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
    /// MOVE / GIT / VIEW.
    pub group: CommandGroup,
    /// Dispatched after the palette closes.
    pub action: Action,
}

/// Named commands only. Pointer / overlay-internal actions stay out.
pub const PALETTE_COMMANDS: &[PaletteCommand] = &[
    PaletteCommand {
        title: "Search focused pane",
        keys: "/",
        group: CommandGroup::Move,
        action: Action::SearchStart,
    },
    PaletteCommand {
        title: "Stage",
        keys: "s",
        group: CommandGroup::Git,
        action: Action::Stage,
    },
    PaletteCommand {
        title: "Unstage",
        keys: "u",
        group: CommandGroup::Git,
        action: Action::Unstage,
    },
    PaletteCommand {
        title: "Revert",
        keys: "x",
        group: CommandGroup::Git,
        action: Action::Revert,
    },
    PaletteCommand {
        title: "Stash menu",
        keys: "S",
        group: CommandGroup::Git,
        action: Action::StashMenu,
    },
    PaletteCommand {
        title: "Fetch remotes",
        keys: "f",
        group: CommandGroup::Git,
        action: Action::Fetch,
    },
    PaletteCommand {
        title: "Pull behind",
        keys: "p",
        group: CommandGroup::Git,
        action: Action::Pull,
    },
    PaletteCommand {
        title: "Push",
        keys: "P",
        group: CommandGroup::Git,
        action: Action::Push,
    },
    PaletteCommand {
        title: "Default branch",
        keys: "d",
        group: CommandGroup::Git,
        action: Action::DefaultBranch,
    },
    PaletteCommand {
        title: "Branch picker",
        keys: "b",
        group: CommandGroup::Git,
        action: Action::Branch,
    },
    PaletteCommand {
        title: "Checkout commit refs",
        keys: "b",
        group: CommandGroup::Git,
        action: Action::GraphCheckout,
    },
    PaletteCommand {
        title: "Create branch at commit",
        keys: "c",
        group: CommandGroup::Git,
        action: Action::GraphCreateBranch,
    },
    PaletteCommand {
        title: "Merge into HEAD",
        keys: "m",
        group: CommandGroup::Git,
        action: Action::GraphMerge,
    },
    PaletteCommand {
        title: "Remove worktree",
        keys: "W",
        group: CommandGroup::Git,
        action: Action::RemoveWorktree,
    },
    PaletteCommand {
        title: "Apply stash",
        keys: "a",
        group: CommandGroup::Git,
        action: Action::GraphStashApply,
    },
    PaletteCommand {
        title: "Pop stash",
        keys: "p",
        group: CommandGroup::Git,
        action: Action::GraphStashPop,
    },
    PaletteCommand {
        title: "Drop stash",
        keys: "D",
        group: CommandGroup::Git,
        action: Action::GraphStashDrop,
    },
    PaletteCommand {
        title: "Open in editor",
        keys: "e",
        group: CommandGroup::Git,
        action: Action::Edit,
    },
    PaletteCommand {
        title: "Open in diff tool",
        keys: "E",
        group: CommandGroup::Git,
        action: Action::ExternalDiff,
    },
    PaletteCommand {
        title: "Refresh",
        keys: "r",
        group: CommandGroup::Git,
        action: Action::Refresh,
    },
    PaletteCommand {
        title: "Mark reviewed",
        keys: "space",
        group: CommandGroup::Git,
        action: Action::ToggleReviewed,
    },
    PaletteCommand {
        title: "Keymap help",
        keys: "?",
        group: CommandGroup::View,
        action: Action::ToggleHelp,
    },
    PaletteCommand {
        title: "Cycle theme",
        keys: "T",
        group: CommandGroup::View,
        action: Action::CycleTheme,
    },
    PaletteCommand {
        title: "Flat / tree",
        keys: "t",
        group: CommandGroup::View,
        action: Action::ToggleTreeMode,
    },
    PaletteCommand {
        title: "Show ignored",
        keys: ".",
        group: CommandGroup::View,
        action: Action::ToggleShowIgnored,
    },
    PaletteCommand {
        title: "Inline / split",
        keys: "i",
        group: CommandGroup::View,
        action: Action::ToggleDiffMode,
    },
    PaletteCommand {
        title: "Toggle mouse",
        keys: "m",
        group: CommandGroup::View,
        action: Action::ToggleMouse,
    },
    PaletteCommand {
        title: "Full-file context",
        keys: "Ctrl-o",
        group: CommandGroup::View,
        action: Action::ToggleFullContext,
    },
    PaletteCommand {
        title: "Graph focus branches",
        keys: "o",
        group: CommandGroup::View,
        action: Action::GraphFocusBranches,
    },
    PaletteCommand {
        title: "Clear graph focus",
        keys: "O",
        group: CommandGroup::View,
        action: Action::GraphFocusClear,
    },
    PaletteCommand {
        title: "Comment",
        keys: ";",
        group: CommandGroup::View,
        action: Action::CommentStart,
    },
    PaletteCommand {
        title: "Highlight diff lines",
        keys: "V",
        group: CommandGroup::View,
        action: Action::DiffVisualStart,
    },
    PaletteCommand {
        title: "Copy comments",
        keys: "y",
        group: CommandGroup::View,
        action: Action::ExportComments,
    },
    PaletteCommand {
        title: "Copy entity reference",
        keys: "'",
        group: CommandGroup::View,
        action: Action::CopyEntityReference,
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

/// Groups that still have a hit. Empty query keeps MOVE / GIT / VIEW.
pub fn visible_groups(query: &str) -> Vec<CommandGroup> {
    let q = query.trim();
    if q.is_empty() {
        return vec![CommandGroup::Move, CommandGroup::Git, CommandGroup::View];
    }
    let mut out = Vec::new();
    for group in [CommandGroup::Move, CommandGroup::Git, CommandGroup::View] {
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
    /// Empty filter, first row highlighted.
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
                for group in [CommandGroup::Move, CommandGroup::Git, CommandGroup::View] {
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
    /// MOVE / GIT / VIEW heading.
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
                "Search focused pane",
                "/",
                CommandGroup::Move,
                Action::SearchStart,
            ),
            ("Stage", "s", CommandGroup::Git, Action::Stage),
            ("Unstage", "u", CommandGroup::Git, Action::Unstage),
            ("Revert", "x", CommandGroup::Git, Action::Revert),
            ("Stash menu", "S", CommandGroup::Git, Action::StashMenu),
            ("Fetch remotes", "f", CommandGroup::Git, Action::Fetch),
            ("Pull behind", "p", CommandGroup::Git, Action::Pull),
            ("Push", "P", CommandGroup::Git, Action::Push),
            (
                "Default branch",
                "d",
                CommandGroup::Git,
                Action::DefaultBranch,
            ),
            ("Branch picker", "b", CommandGroup::Git, Action::Branch),
            (
                "Checkout commit refs",
                "b",
                CommandGroup::Git,
                Action::GraphCheckout,
            ),
            (
                "Create branch at commit",
                "c",
                CommandGroup::Git,
                Action::GraphCreateBranch,
            ),
            (
                "Merge into HEAD",
                "m",
                CommandGroup::Git,
                Action::GraphMerge,
            ),
            (
                "Remove worktree",
                "W",
                CommandGroup::Git,
                Action::RemoveWorktree,
            ),
            (
                "Apply stash",
                "a",
                CommandGroup::Git,
                Action::GraphStashApply,
            ),
            ("Pop stash", "p", CommandGroup::Git, Action::GraphStashPop),
            ("Drop stash", "D", CommandGroup::Git, Action::GraphStashDrop),
            ("Open in editor", "e", CommandGroup::Git, Action::Edit),
            (
                "Open in diff tool",
                "E",
                CommandGroup::Git,
                Action::ExternalDiff,
            ),
            ("Refresh", "r", CommandGroup::Git, Action::Refresh),
            (
                "Mark reviewed",
                "space",
                CommandGroup::Git,
                Action::ToggleReviewed,
            ),
            ("Keymap help", "?", CommandGroup::View, Action::ToggleHelp),
            ("Cycle theme", "T", CommandGroup::View, Action::CycleTheme),
            (
                "Flat / tree",
                "t",
                CommandGroup::View,
                Action::ToggleTreeMode,
            ),
            (
                "Show ignored",
                ".",
                CommandGroup::View,
                Action::ToggleShowIgnored,
            ),
            (
                "Inline / split",
                "i",
                CommandGroup::View,
                Action::ToggleDiffMode,
            ),
            ("Toggle mouse", "m", CommandGroup::View, Action::ToggleMouse),
            (
                "Full-file context",
                "Ctrl-o",
                CommandGroup::View,
                Action::ToggleFullContext,
            ),
            (
                "Graph focus branches",
                "o",
                CommandGroup::View,
                Action::GraphFocusBranches,
            ),
            (
                "Clear graph focus",
                "O",
                CommandGroup::View,
                Action::GraphFocusClear,
            ),
            ("Comment", ";", CommandGroup::View, Action::CommentStart),
            (
                "Highlight diff lines",
                "V",
                CommandGroup::View,
                Action::DiffVisualStart,
            ),
            (
                "Copy comments",
                "y",
                CommandGroup::View,
                Action::ExportComments,
            ),
            (
                "Copy entity reference",
                "'",
                CommandGroup::View,
                Action::CopyEntityReference,
            ),
        ];
        assert_eq!(PALETTE_COMMANDS.len(), wanted.len());
        for (i, (title, keys, group, action)) in wanted.iter().enumerate() {
            let command = &PALETTE_COMMANDS[i];
            assert_eq!(command.title, *title, "row {i}");
            assert_eq!(command.keys, *keys, "{title}");
            assert_eq!(command.group, *group, "{title}");
            assert_eq!(command.action, *action, "{title}");
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
    fn empty_query_keeps_groups_query_hides_empty() {
        assert_eq!(
            visible_groups(""),
            vec![CommandGroup::Move, CommandGroup::Git, CommandGroup::View]
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
