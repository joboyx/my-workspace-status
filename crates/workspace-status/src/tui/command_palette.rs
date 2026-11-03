//! The commands mode of Quick Open (`:`, or `>` typed first in files mode).
//!
//! Filter is case-insensitive substring on title, key chips, group, and
//! aliases (`exit` finds Quit, `compare` the Diff … in new tab rows). A
//! filter of digits only (`:42`) adds a synthetic `Go to line 42` row on
//! top ([`go_to_line_target`]); it is not a catalog row.
//! Execute is close-then-dispatch through [`super::state::AppState::dispatch`].
//! Each built-in theme has a `Theme: {label}` row: focusing it previews
//! that theme, and only Enter on it applies and saves it.

use std::borrow::Cow;

use super::action::Action;
use super::bindings::{self, chips, KeyBinding};
use super::theme::ThemeId;

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

/// A `Theme: {label}` row for one built-in theme.
///
/// Focusing the row previews the theme; Enter applies and saves it
/// ([`Action::SetTheme`]). Aliases are `theme`, `color`, `colour`, then the
/// words passed in: the slug and each lowercase word of the label.
macro_rules! theme_row {
    ($id:ident, $title:literal, [$($word:literal),+ $(,)?]) => {
        PaletteCommand {
            title: $title,
            keys: "",
            group: CommandGroup::View,
            action: Action::SetTheme(ThemeId::$id),
            aliases: &["theme", "color", "colour", $($word),+],
            scope: CommandScope::Any,
        }
    };
}

/// A catalog row that takes its key chip and action from `binding`.
const fn bound(
    title: &'static str,
    binding: KeyBinding,
    group: CommandGroup,
    aliases: &'static [&'static str],
    scope: CommandScope,
) -> PaletteCommand {
    PaletteCommand {
        title,
        keys: binding.chip,
        group,
        action: binding.action,
        aliases,
        scope,
    }
}

/// Named commands only. Pointer / overlay-internal actions stay out.
pub const PALETTE_COMMANDS: &[PaletteCommand] = &[
    bound(
        "Stage highlighted lines",
        bindings::STAGE,
        CommandGroup::Highlight,
        &["add"],
        CommandScope::Highlight,
    ),
    bound(
        "Unstage highlighted lines",
        bindings::UNSTAGE,
        CommandGroup::Highlight,
        &["reset"],
        CommandScope::Highlight,
    ),
    bound(
        "Revert highlighted lines",
        bindings::REVERT,
        CommandGroup::Highlight,
        &["discard", "restore"],
        CommandScope::Highlight,
    ),
    bound(
        "Exit highlight",
        bindings::EXIT_HIGHLIGHT,
        CommandGroup::Highlight,
        &["cancel"],
        CommandScope::Highlight,
    ),
    bound(
        "Search focused pane",
        bindings::SEARCH_PANE,
        CommandGroup::Move,
        &["find", "grep"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Next match",
        bindings::NEXT_MATCH,
        CommandGroup::Move,
        &["find next", "search"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Previous match",
        bindings::PREVIOUS_MATCH,
        CommandGroup::Move,
        &["find previous", "search"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Search in files",
        bindings::SEARCH_IN_FILES,
        CommandGroup::Move,
        &["find in files", "grep", "text search"],
        CommandScope::Any,
    ),
    bound(
        "Fold row",
        bindings::FOLD_ROW,
        CommandGroup::Move,
        &["collapse", "expand", "unfold"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Fold subtree",
        bindings::FOLD_SUBTREE,
        CommandGroup::Move,
        &["collapse", "expand", "unfold"],
        CommandScope::NoHighlight,
    ),
    // Palette Enter runs FocusLeft instead while the right pane has focus.
    bound(
        "Other pane",
        bindings::OTHER_PANE,
        CommandGroup::Move,
        &["switch pane", "focus"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Next tab",
        bindings::NEXT_TAB,
        CommandGroup::Move,
        &["switch tab"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Previous tab",
        bindings::PREVIOUS_TAB,
        CommandGroup::Move,
        &["switch tab"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Previous file tab",
        bindings::PREVIOUS_FILE_TAB,
        CommandGroup::Move,
        &["explorer file", "switch file"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Next file tab",
        bindings::NEXT_FILE_TAB,
        CommandGroup::Move,
        &["explorer file", "switch file"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Open in Explorer",
        bindings::OPEN_IN_EXPLORER,
        CommandGroup::Move,
        &["browse", "file tree"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Fold finished PR groups",
        bindings::FOLD_FINISHED_PR_GROUPS,
        CommandGroup::Move,
        &["merged", "collapse"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Stage",
        bindings::STAGE,
        CommandGroup::Git,
        &["add"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Unstage",
        bindings::UNSTAGE,
        CommandGroup::Git,
        &["reset"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Revert",
        bindings::REVERT,
        CommandGroup::Git,
        &["discard", "restore"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Stash menu",
        bindings::STASH_MENU,
        CommandGroup::Git,
        &["save", "shelve"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Fetch remotes",
        bindings::FETCH,
        CommandGroup::Git,
        &["download", "update"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Pull behind",
        bindings::PULL,
        CommandGroup::Git,
        &["update", "sync"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Push",
        bindings::PUSH,
        CommandGroup::Git,
        &["upload", "publish"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Default branch",
        bindings::DEFAULT_BRANCH,
        CommandGroup::Git,
        &["main", "switch"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Branch picker",
        bindings::BRANCH_PICKER,
        CommandGroup::Git,
        &["checkout", "switch", "create branch", "new branch"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Checkout commit refs",
        bindings::GRAPH_CHECKOUT,
        CommandGroup::Git,
        &["checkout", "switch"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Create branch at commit",
        bindings::GRAPH_CREATE_BRANCH,
        CommandGroup::Git,
        &["new branch"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Merge into HEAD",
        bindings::GRAPH_MERGE,
        CommandGroup::Git,
        &["integrate"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Remove worktree",
        bindings::REMOVE_WORKTREE,
        CommandGroup::Git,
        &["worktree", "delete"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Apply stash",
        bindings::GRAPH_STASH_APPLY,
        CommandGroup::Git,
        &["unstash"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Pop stash",
        bindings::GRAPH_STASH_POP,
        CommandGroup::Git,
        &["unstash"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Drop stash",
        bindings::GRAPH_STASH_DROP,
        CommandGroup::Git,
        &["delete"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Open in editor",
        bindings::OPEN_IN_EDITOR,
        CommandGroup::Git,
        &["edit", "vim"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Open in diff tool",
        bindings::OPEN_IN_DIFF_TOOL,
        CommandGroup::Git,
        &["difftool", "vimdiff"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Refresh",
        bindings::REFRESH,
        CommandGroup::Git,
        &["reload", "rescan"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Open PR",
        bindings::OPEN_PR,
        CommandGroup::Git,
        &[
            "pull request",
            "merge request",
            "browser",
            "github",
            "gitlab",
        ],
        CommandScope::NoHighlight,
    ),
    bound(
        "Open PR CI",
        bindings::OPEN_PR_CI,
        CommandGroup::Git,
        &[
            "checks",
            "pipeline",
            "pull request",
            "merge request",
            "browser",
            "github",
            "gitlab",
        ],
        CommandScope::NoHighlight,
    ),
    // `y` and `r` run these only inside a pinned PR popover; elsewhere
    // those keys copy comments and refresh. So the rows show no chip, and
    // the popover lines paint `y` and `r` themselves.
    PaletteCommand {
        title: "Copy PR URL",
        keys: "",
        group: CommandGroup::Git,
        action: Action::CopyPullRequestUrl,
        aliases: &["pull request", "merge request", "link", "yank"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Refresh this PR",
        keys: "",
        group: CommandGroup::Git,
        action: Action::RefreshPullRequest,
        aliases: &["pull request", "merge request", "reload"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Refresh PRs",
        keys: "",
        group: CommandGroup::Git,
        action: Action::RefreshPullRequests,
        aliases: &["pull requests", "merge requests", "reload"],
        scope: CommandScope::NoHighlight,
    },
    bound(
        "View remotely",
        bindings::VIEW_REMOTELY,
        CommandGroup::Git,
        &["browser", "web", "github", "gitlab", "open on remote"],
        CommandScope::NoHighlight,
    ),
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
    bound(
        "Close tab",
        bindings::CLOSE_TAB,
        CommandGroup::Git,
        &["close compare"],
        CommandScope::NoHighlight,
    ),
    bound(
        "PRs tab",
        bindings::PRS_TAB,
        CommandGroup::Git,
        &["pull requests", "merge requests", "github", "gitlab"],
        CommandScope::Any,
    ),
    bound(
        "Mark reviewed",
        bindings::MARK_REVIEWED,
        CommandGroup::Git,
        &["viewed", "seen"],
        CommandScope::NoHighlight,
    ),
    PaletteCommand {
        title: "Go to file",
        keys: chips!(bindings::GO_TO_FILE.chip, bindings::GO_TO_FILE_F.chip),
        group: CommandGroup::View,
        action: bindings::GO_TO_FILE.action,
        aliases: &["open file", "quick open", "fuzzy find"],
        scope: CommandScope::Any,
    },
    bound(
        "Keymap help",
        bindings::KEYMAP_HELP,
        CommandGroup::View,
        &["help", "keys", "shortcuts"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Icon popover",
        bindings::ICON_POPOVER,
        CommandGroup::View,
        &["icons", "details"],
        CommandScope::NoHighlight,
    ),
    theme_row!(
        TokyoNight,
        "Theme: Tokyo Night",
        ["tokyo-night", "tokyo", "night"]
    ),
    theme_row!(Monokai, "Theme: Monokai", ["monokai"]),
    theme_row!(Dracula, "Theme: Dracula", ["dracula"]),
    theme_row!(
        GruvboxDark,
        "Theme: Gruvbox Dark",
        ["gruvbox-dark", "gruvbox", "dark"]
    ),
    theme_row!(
        CatppuccinMocha,
        "Theme: Catppuccin Mocha",
        ["catppuccin-mocha", "catppuccin", "mocha"]
    ),
    theme_row!(Slate, "Theme: Slate", ["slate"]),
    theme_row!(
        SolarizedDark,
        "Theme: Solarized Dark",
        ["solarized-dark", "solarized", "dark"]
    ),
    theme_row!(Nord, "Theme: Nord", ["nord"]),
    theme_row!(RosePine, "Theme: Rosé Pine", ["rose-pine", "rosé", "pine"]),
    theme_row!(Kanagawa, "Theme: Kanagawa", ["kanagawa"]),
    theme_row!(Everforest, "Theme: Everforest", ["everforest"]),
    theme_row!(OneDark, "Theme: One Dark", ["one-dark", "one", "dark"]),
    theme_row!(
        GithubDarkDimmed,
        "Theme: GitHub Dark Dimmed",
        ["github-dark-dimmed", "github", "dark", "dimmed"]
    ),
    theme_row!(
        TokyoNightDay,
        "Theme: Tokyo Night Day",
        ["tokyo-night-day", "tokyo", "night", "day"]
    ),
    theme_row!(
        MonokaiLight,
        "Theme: Monokai Light",
        ["monokai-light", "monokai", "light"]
    ),
    theme_row!(
        DraculaLight,
        "Theme: Dracula Light",
        ["dracula-light", "dracula", "light"]
    ),
    theme_row!(
        GruvboxLight,
        "Theme: Gruvbox Light",
        ["gruvbox-light", "gruvbox", "light"]
    ),
    theme_row!(
        CatppuccinLatte,
        "Theme: Catppuccin Latte",
        ["catppuccin-latte", "catppuccin", "latte"]
    ),
    theme_row!(
        SlateLight,
        "Theme: Slate Light",
        ["slate-light", "slate", "light"]
    ),
    theme_row!(
        SolarizedLight,
        "Theme: Solarized Light",
        ["solarized-light", "solarized", "light"]
    ),
    theme_row!(
        NordLight,
        "Theme: Nord Light",
        ["nord-light", "nord", "light"]
    ),
    theme_row!(
        RosePineDawn,
        "Theme: Rosé Pine Dawn",
        ["rose-pine-dawn", "rosé", "pine", "dawn"]
    ),
    theme_row!(
        KanagawaLotus,
        "Theme: Kanagawa Lotus",
        ["kanagawa-lotus", "kanagawa", "lotus"]
    ),
    theme_row!(
        EverforestLight,
        "Theme: Everforest Light",
        ["everforest-light", "everforest", "light"]
    ),
    theme_row!(OneLight, "Theme: One Light", ["one-light", "one", "light"]),
    theme_row!(
        GithubLight,
        "Theme: GitHub Light",
        ["github-light", "github", "light"]
    ),
    bound(
        "Flat / tree",
        bindings::FLAT_OR_TREE,
        CommandGroup::View,
        &["list"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Show hidden",
        bindings::SHOW_HIDDEN,
        CommandGroup::View,
        &["hidden"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Inline / split",
        bindings::INLINE_OR_SPLIT,
        CommandGroup::View,
        &["side by side", "unified"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Narrow tree",
        bindings::NARROW_TREE,
        CommandGroup::View,
        &["widen diff", "pane width", "resize", "shrink"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Widen tree",
        bindings::WIDEN_TREE,
        CommandGroup::View,
        &["narrow diff", "pane width", "resize", "grow"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Wrap / unwrap",
        bindings::WRAP_DIFF,
        CommandGroup::View,
        &["soft wrap"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Collapse / expand commit message",
        bindings::EXPAND_COMMIT_MESSAGE,
        CommandGroup::View,
        &["body"],
        CommandScope::NoHighlight,
    ),
    PaletteCommand {
        title: "Shorter commit message",
        keys: "",
        group: CommandGroup::View,
        action: Action::ResizeCommitMsg(-1),
        aliases: &["message height", "msg lines", "shrink"],
        scope: CommandScope::NoHighlight,
    },
    PaletteCommand {
        title: "Taller commit message",
        keys: "",
        group: CommandGroup::View,
        action: Action::ResizeCommitMsg(1),
        aliases: &["message height", "msg lines", "grow"],
        scope: CommandScope::NoHighlight,
    },
    bound(
        "Toggle line blame",
        bindings::LINE_BLAME,
        CommandGroup::View,
        &["blame", "gitlens"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Toggle mouse",
        bindings::TOGGLE_MOUSE,
        CommandGroup::View,
        &["pointer", "click"],
        CommandScope::Any,
    ),
    PaletteCommand {
        title: "Toggle activity sparkline",
        keys: "",
        group: CommandGroup::View,
        action: Action::ToggleActivitySparkline,
        aliases: &["sparkline", "activity"],
        scope: CommandScope::Any,
    },
    bound(
        "Full-file context",
        bindings::FULL_FILE_CONTEXT,
        CommandGroup::View,
        &["expand"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Graph focus branches",
        bindings::GRAPH_FOCUS_BRANCHES,
        CommandGroup::View,
        &["filter graph"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Clear graph focus",
        bindings::CLEAR_GRAPH_FOCUS,
        CommandGroup::View,
        &["full graph", "unfocus"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Comment",
        bindings::COMMENT,
        CommandGroup::View,
        &["note", "annotate"],
        CommandScope::Any,
    ),
    bound(
        "Highlight diff lines",
        bindings::HIGHLIGHT_LINES,
        CommandGroup::View,
        &["visual", "select lines"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Copy comments",
        bindings::COPY_COMMENTS,
        CommandGroup::View,
        &["yank", "export"],
        CommandScope::NoHighlight,
    ),
    bound(
        "Copy entity reference",
        bindings::COPY_ENTITY_REFERENCE,
        CommandGroup::View,
        &["yank", "link"],
        CommandScope::Any,
    ),
    PaletteCommand {
        title: "Doctor: setup checkup",
        keys: "",
        group: CommandGroup::View,
        action: Action::OpenDoctor,
        aliases: &["doctor", "checkup", "setup", "health"],
        scope: CommandScope::Any,
    },
    bound(
        "Quit",
        bindings::QUIT,
        CommandGroup::View,
        &["exit", "close app"],
        CommandScope::Any,
    ),
];

/// The catalog row that runs `action`, if any.
///
/// Icon popovers take an action line's title, key chip, and disabled
/// reason from this row, so a popover never names a key the palette does
/// not. A row that runs only on highlighted diff lines (`Stage highlighted
/// lines`) gives way to the plain row for the same action (`Stage`).
pub fn command_for(action: &Action) -> Option<&'static PaletteCommand> {
    let mut rows = PALETTE_COMMANDS
        .iter()
        .filter(|command| &command.action == action);
    let first = rows.next()?;
    if first.scope != CommandScope::Highlight {
        return Some(first);
    }
    Some(
        rows.find(|command| command.scope != CommandScope::Highlight)
            .unwrap_or(first),
    )
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

/// Title of the synthetic go-to-line row; the painted row adds the line
/// ([`go_to_line_title`]).
pub const GO_TO_LINE_TITLE: &str = "Go to line";

/// Line the synthetic go-to-line row jumps to, or `None` when `filter`
/// does not ask for one.
///
/// `filter` (the [`super::quick_open::commands_filter`] text) must be one
/// or more ASCII digits once trimmed at both ends: `42` and `042` give 42,
/// while a blank filter, `42x`, `line`, and `4 2` give `None`. A number too
/// large for `usize` saturates; the jump clamps it to the last line anyway.
pub fn go_to_line_target(filter: &str) -> Option<usize> {
    let digits = filter.trim();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(digits.parse().unwrap_or(usize::MAX))
}

/// Painted title of the go-to-line row for `line`: `Go to line 42`.
pub fn go_to_line_title(line: usize) -> String {
    format!("{GO_TO_LINE_TITLE} {line}")
}

/// The synthetic go-to-line row for `line`, shaped like a catalog row.
///
/// It is not in [`PALETTE_COMMANDS`]: commands mode adds it above the
/// catalog rows while the filter is all digits ([`go_to_line_target`]).
/// It has no key chip and runs in any highlight mode.
pub fn go_to_line_command(line: usize) -> PaletteCommand {
    PaletteCommand {
        title: GO_TO_LINE_TITLE,
        keys: "",
        group: CommandGroup::Move,
        action: Action::GoToLine(line),
        aliases: &[],
        scope: CommandScope::Any,
    }
}

/// Interactive palette state (filter + highlight).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandPaletteState {
    /// Filter query (substring).
    pub filter: String,
    /// Highlight index into [`Self::rows`]: the go-to-line row is index 0
    /// while it shows, and the [`Self::visible`] rows follow it.
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

    /// Catalog commands that match the current filter, table order.
    pub fn visible(&self) -> Vec<&'static PaletteCommand> {
        filter_commands(&self.filter)
    }

    /// Line of the synthetic go-to-line row, `Some` while the filter is all
    /// digits ([`go_to_line_target`]).
    pub fn go_to_line(&self) -> Option<usize> {
        go_to_line_target(&self.filter)
    }

    /// Cursor index of the first [`Self::visible`] row: 1 while the
    /// go-to-line row sits above the catalog rows, else 0.
    fn catalog_start(&self) -> usize {
        usize::from(self.go_to_line().is_some())
    }

    /// Every row in cursor order: the go-to-line row while it shows, then
    /// [`Self::visible`].
    pub fn rows(&self) -> Vec<Cow<'static, PaletteCommand>> {
        self.go_to_line()
            .map(|line| Cow::Owned(go_to_line_command(line)))
            .into_iter()
            .chain(self.visible().into_iter().map(Cow::Borrowed))
            .collect()
    }

    /// Indexes into [`Self::rows`] that the cursor may land on after a
    /// filter change. `highlighted` is true while a diff range is
    /// highlighted.
    ///
    /// The go-to-line row, while it shows, is the only landing row, enabled
    /// or not: Enter on `:42` runs it or says why it cannot.
    /// Otherwise rows the filter finds by title, key chips, or group, and
    /// whose scope fits the highlight mode, outrank rows it finds only by
    /// an alias: typing `pull` lands on Pull behind, not on a row that has
    /// a `pull request` alias. With no such row, every visible row may
    /// take the cursor (`exit` outside a highlight lands on Quit, not on
    /// Exit highlight).
    pub fn landing_rows(&self, highlighted: bool) -> Vec<usize> {
        if self.go_to_line().is_some() {
            return vec![0];
        }
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

    /// Highlighted row, if [`Self::rows`] is non-empty.
    pub fn selected(&self) -> Option<Cow<'static, PaletteCommand>> {
        let start = self.catalog_start();
        match (self.go_to_line(), self.cursor.checked_sub(start)) {
            (Some(line), None) => Some(Cow::Owned(go_to_line_command(line))),
            (_, Some(index)) => self.visible().get(index).copied().map(Cow::Borrowed),
            (None, None) => None,
        }
    }

    /// Mapped [`Action`] for the highlighted row, if any.
    pub fn selected_action(&self) -> Option<Action> {
        self.selected().map(|command| command.action.clone())
    }

    /// Row count of [`Self::rows`].
    fn len(&self) -> usize {
        self.catalog_start() + self.visible().len()
    }

    /// Move the highlight by `delta`, clamped like the branch picker.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.len();
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
        let len = self.len();
        if len == 0 {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(len - 1);
        }
    }

    /// The go-to-line row while it shows, then group headers plus commands
    /// in table order (empty groups omitted when filtering).
    pub fn paint_rows(&self) -> Vec<PalettePaintRow> {
        let mut out = Vec::new();
        if let Some(line) = self.go_to_line() {
            out.push(PalettePaintRow::GoToLine { line });
        }
        let start = self.catalog_start();
        let visible = self.visible();
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
            out.push(PalettePaintRow::Command {
                command,
                index: start + index,
            });
        }
        out
    }
}

/// One painted palette line (group header, go-to-line row, or command).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PalettePaintRow {
    /// HIGHLIGHT / MOVE / GIT / VIEW heading.
    Header(&'static str),
    /// The synthetic go-to-line row ([`go_to_line_command`]), cursor index
    /// 0. It paints above every header, with no group label.
    GoToLine {
        /// Line the row jumps to, as typed (leading zeros dropped).
        line: usize,
    },
    /// Catalog row. `index` is into [`CommandPaletteState::rows`].
    Command {
        command: &'static PaletteCommand,
        /// Index into [`CommandPaletteState::rows`].
        index: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::super::theme::THEME_IDS;
    use super::*;

    fn titles(query: &str) -> Vec<&'static str> {
        filter_commands(query)
            .into_iter()
            .map(|c| c.title)
            .collect()
    }

    /// One `Theme: {label}` row per theme, in `THEME_IDS` order,
    /// after Icon popover, before Flat / tree.
    #[test]
    fn theme_rows_follow_theme_ids_after_icon_popover() {
        let start = PALETTE_COMMANDS
            .iter()
            .position(|command| command.title == "Icon popover")
            .expect("Icon popover row")
            + 1;
        let rows = &PALETTE_COMMANDS[start..start + THEME_IDS.len()];
        assert_eq!(
            PALETTE_COMMANDS[start + THEME_IDS.len()].title,
            "Flat / tree"
        );
        for (command, id) in rows.iter().zip(THEME_IDS) {
            assert_eq!(command.title, format!("Theme: {}", id.label()));
            assert_eq!(command.keys, "", "{}", command.title);
            assert_eq!(command.group, CommandGroup::View, "{}", command.title);
            assert_eq!(command.scope, CommandScope::Any, "{}", command.title);
            assert_eq!(command.action, Action::SetTheme(id), "{}", command.title);
            let words = ["theme", "color", "colour", id.as_str()]
                .into_iter()
                .map(str::to_string)
                .chain(id.label().split_whitespace().map(str::to_lowercase));
            for word in words {
                assert!(
                    command.aliases.contains(&word.as_str()),
                    "{} lacks alias {word}",
                    command.title
                );
            }
        }
        assert_eq!(
            PALETTE_COMMANDS
                .iter()
                .filter(|command| matches!(command.action, Action::SetTheme(_)))
                .count(),
            THEME_IDS.len(),
            "no theme row outside the block"
        );
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
            ("theme", "Theme: Nord"),
            ("color", "Theme: Tokyo Night"),
            ("colour", "Theme: Dracula"),
            ("tokyo-night", "Theme: Tokyo Night"),
            ("night", "Theme: Tokyo Night"),
            ("dracula", "Theme: Dracula"),
            ("github-dark-dimmed", "Theme: GitHub Dark Dimmed"),
            ("mouse", "Toggle mouse"),
            ("sparkline", "Toggle activity sparkline"),
            ("activity", "Toggle activity sparkline"),
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
            ("checks", "Open PR CI"),
            ("pipeline", "Open PR CI"),
            ("merge request", "Open PR CI"),
            ("link", "Copy PR URL"),
            ("yank", "Copy PR URL"),
            ("pull request", "Refresh this PR"),
            ("reload", "Refresh this PR"),
            ("reload", "Refresh PRs"),
            ("pull requests", "Refresh PRs"),
            ("merge requests", "Refresh PRs"),
        ] {
            assert!(titles(query).contains(&title), "{query} -> {title}");
        }
        assert_eq!(titles("close app"), vec!["Quit"]);
        assert_eq!(titles("sparkline"), vec!["Toggle activity sparkline"]);
    }

    /// The Doctor is a VIEW row with no key that runs from anywhere, and
    /// each of its aliases finds it.
    #[test]
    fn doctor_row_is_found_by_each_alias() {
        let row = command_for(&Action::OpenDoctor).expect("Doctor row");
        assert_eq!(row.title, "Doctor: setup checkup");
        assert_eq!(row.keys, "");
        assert_eq!(row.group, CommandGroup::View);
        assert_eq!(row.scope, CommandScope::Any);
        for query in ["doctor", "checkup", "setup", "health", "DOCTOR"] {
            assert!(titles(query).contains(&"Doctor: setup checkup"), "{query}");
        }
        assert_eq!(titles("doctor"), vec!["Doctor: setup checkup"]);
    }

    /// A dark theme's slug or name lands on the dark row, and the light
    /// twin's slug lands on the twin, though both rows match the shorter
    /// query.
    #[test]
    fn twin_theme_queries_land_on_the_named_polarity() {
        for (query, want) in [
            ("nord", "Theme: Nord"),
            ("nord-light", "Theme: Nord Light"),
            ("slate", "Theme: Slate"),
            ("slate-light", "Theme: Slate Light"),
            ("tokyo-night", "Theme: Tokyo Night"),
            ("tokyo-night-day", "Theme: Tokyo Night Day"),
            ("rose-pine-dawn", "Theme: Rosé Pine Dawn"),
            ("github-light", "Theme: GitHub Light"),
        ] {
            let mut palette = CommandPaletteState::new();
            palette.set_filter(query);
            let visible = palette.visible();
            let landing = palette.landing_rows(false);
            assert_eq!(
                landing.first().map(|&index| visible[index].title),
                Some(want),
                "{query}"
            );
        }
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
        assert_eq!(landing("pull"), vec!["Pull behind"], "only Pull names it");
        // Every PR row only aliases these, so all of them land.
        let pr_rows = vec![
            "Open PR",
            "Open PR CI",
            "Copy PR URL",
            "Refresh this PR",
            "Refresh PRs",
            "PRs tab",
        ];
        assert_eq!(landing("pull request"), pr_rows);
        assert_eq!(landing("merge request"), pr_rows);
        assert_eq!(landing("refresh pr"), vec!["Refresh PRs"]);
        assert!(landing("PR").contains(&"Open PR"));
        assert_eq!(landing("gx"), vec!["Open PR"]);
        assert_eq!(landing("gc"), vec!["Open PR CI"]);
        assert_eq!(landing("gp"), vec!["PRs tab"]);
        assert_eq!(landing("prs tab"), vec!["PRs tab"]);
        assert_eq!(landing("pipeline"), vec!["Open PR CI"]);
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
            "Toggle line blame",
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

    /// Toggle rows say `Toggle …`, not `… on / off`.
    #[test]
    fn no_palette_title_says_on_off() {
        for command in PALETTE_COMMANDS {
            assert!(!command.title.contains("on / off"), "{}", command.title);
        }
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
                _ => None,
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

    #[test]
    fn digits_only_filter_asks_for_go_to_line() {
        for (filter, want) in [
            ("42", Some(42)),
            ("042", Some(42)),
            ("42 ", Some(42)),
            ("  7", Some(7)),
            ("0", Some(0)),
            ("99999999999999999999999", Some(usize::MAX)),
            ("", None),
            ("   ", None),
            ("42x", None),
            ("line", None),
            ("4 2", None),
            ("-4", None),
        ] {
            assert_eq!(go_to_line_target(filter), want, "{filter:?}");
        }
        assert_eq!(go_to_line_title(42), "Go to line 42");
    }

    /// The row sits above the catalog rows and shifts their cursor index.
    #[test]
    fn go_to_line_row_is_cursor_zero_and_the_only_landing_row() {
        let mut palette = CommandPaletteState::new();
        palette.set_filter("042");
        assert_eq!(palette.go_to_line(), Some(42));
        assert_eq!(palette.landing_rows(false), vec![0]);
        assert_eq!(palette.landing_rows(true), vec![0]);
        let selected = palette.selected().expect("the go-to-line row");
        assert_eq!(selected.action, Action::GoToLine(42));
        assert_eq!(selected.title, GO_TO_LINE_TITLE);
        assert_eq!(palette.selected_action(), Some(Action::GoToLine(42)));
        assert_eq!(
            palette.paint_rows(),
            vec![PalettePaintRow::GoToLine { line: 42 }]
        );
        assert_eq!(palette.rows().len(), 1, "no catalog row has a digit");
        palette.move_cursor(5);
        assert_eq!(palette.cursor, 0, "a lone row clamps the cursor");
        // Not a catalog row: the 81-row catalog never lists it.
        assert!(PALETTE_COMMANDS
            .iter()
            .all(|command| !matches!(command.action, Action::GoToLine(_))));

        palette.set_filter("42x");
        assert_eq!(palette.go_to_line(), None);
        assert!(palette.rows().is_empty());
        assert!(palette.selected().is_none());
        palette.set_filter("");
        assert_eq!(palette.go_to_line(), None);
        assert_eq!(palette.rows().len(), PALETTE_COMMANDS.len());
        assert_eq!(
            palette.selected().map(|c| c.title),
            Some(PALETTE_COMMANDS[0].title)
        );
    }
}
