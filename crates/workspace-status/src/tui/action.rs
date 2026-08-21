//! Elm-style Action / Effect for the ratatui TUI.

/// User or system input that changes TUI state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    Move(i32),
    MoveToStart,
    MoveToEnd,
    PageMove(i32),
    FoldToggle,
    FoldClose,
    FoldOpen,
    ToggleShowIgnored,
    Fetch,
    Pull,
    DefaultBranch,
    Refresh,
    ToggleReviewed,
    FocusLeft,
    FocusRight,
    ScrollDiff(i32),
    Click { col: u16, row: u16 },
    ScrollWheel { col: u16, row: u16, delta: i32 },
    SearchStart,
    SearchChar(char),
    SearchBackspace,
    SearchSubmit,
    SearchCancel,
    SearchNext,
    SearchPrev,
    Stage,
    Unstage,
    Revert,
    ConfirmYes,
    ConfirmNo,
    Edit,
    WatchTick,
    None,
}

/// Side effect requested after dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Fetch { repos: Vec<String> },
    Pull { repos: Vec<String> },
    DefaultBranch { repos: Vec<String> },
    ReloadSnapshot,
    LoadRightPane,
    Stage {
        repo: String,
        paths: Vec<String>,
    },
    Unstage {
        repo: String,
        paths: Vec<String>,
    },
    Revert {
        repo: String,
        paths: Vec<String>,
        untracked: bool,
    },
    EditFile {
        repo: String,
        path: String,
    },
    WatchRefresh,
}
