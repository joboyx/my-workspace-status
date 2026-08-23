//! Elm-style Action / Effect for the ratatui TUI.

use super::drill::CommitFileSource;

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
    /**
     * Second `z` within [`super::keys::DOUBLE_TAP_MS`].
     *
     * Ink `fold.toggleSubtree`. First `z` already applied [`Self::FoldToggle`].
     */
    FoldToggleSubtree,
    /**
     * First `g` of a `gg` chord.
     *
     * Dispatch arms a pending timer and does not move the cursor.
     */
    ArmGChord,
    FoldClose,
    FoldOpen,
    ToggleShowIgnored,
    ToggleTreeMode,
    Fetch,
    Pull,
    DefaultBranch,
    Refresh,
    ToggleReviewed,
    FocusLeft,
    FocusRight,
    ScrollDiff(i32),
    /// Horizontal pan on a focused file diff. Positive looks right.
    PanDiff(i32),
    /// Toggle unlimited `-U` context on the focused file diff.
    ToggleFullContext,
    Click { col: u16, row: u16 },
    Drag { col: u16, row: u16 },
    Release,
    ToggleDiffMode,
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
    ConfirmYesClean,
    ConfirmNo,
    Edit,
    WatchTick,
    FetchTick,
    RemoveWorktree,
    Push,
    StashMenu,
    StashMenuChar(char),
    StashMenuEnter,
    StashMenuCancel,
    Branch,
    BranchMove(i32),
    BranchChar(char),
    BranchBackspace,
    BranchSubmit,
    BranchCancel,
    CreateBranchStart,
    CreateBranchChar(char),
    CreateBranchBackspace,
    CreateBranchSubmit,
    CreateBranchCancel,
    NavEnter,
    NavEsc,
    GraphStashApply,
    GraphStashPop,
    GraphStashDrop,
    GraphCheckout,
    GraphCreateBranch,
    EasyMotionStart,
    EasyMotionChar(char),
    EasyMotionCancel,
    CycleTheme,
    ToggleMouse,
    None,
}

/// Side effect requested after dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    /// Per-repo writes applied in first-seen order when the focused scope
    /// spans more than one repo. A single-repo write stays a plain Stage,
    /// Unstage, or Revert.
    Batch(Vec<Effect>),
    Fetch { repos: Vec<String> },
    Pull { repos: Vec<String> },
    DefaultBranch { repos: Vec<String> },
    /// Reload every checkout (`r` on the workspace row or No-updates group).
    ReloadSnapshot,
    /// Reload one checkout (`r` on a repo, checkout, file, or dir row).
    ReloadRepo { repo: String },
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
        tracked: Vec<String>,
        untracked: Vec<String>,
    },
    EditFile {
        repo: String,
        path: String,
    },
    WatchRefresh,
    Push { repos: Vec<String> },
    PrepareStashMenu { repo: String },
    StashCreate {
        repo: String,
        paths: Vec<String>,
    },
    StashApply {
        repo: String,
        stash_ref: String,
    },
    StashPop {
        repo: String,
        stash_ref: String,
    },
    StashDrop {
        repo: String,
        stash_ref: String,
    },
    PrepareBranchPicker { repo: String },
    CheckoutBranch {
        repo: String,
        /// Picker or graph selection. May be `origin/<name>`. Confirm Yes uses the local name.
        selected_name: String,
        /// After origin confirm Yes: `merge --ff-only` this already-fetched remote-tracking ref.
        fast_forward_ref: Option<String>,
    },
    CreateBranch {
        repo: String,
        name: String,
    },
    CreateBranchAt {
        repo: String,
        name: String,
        commit_id: String,
    },
    RemoveWorktree {
        primary: String,
        path: String,
        force: bool,
    },
    LoadCommitFiles {
        repo: String,
        source: CommitFileSource,
    },
    LoadCommitDiff {
        repo: String,
        source: CommitFileSource,
        path: String,
    },
}
