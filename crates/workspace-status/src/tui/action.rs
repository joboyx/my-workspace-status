//! Elm-style Action / Effect for the ratatui TUI.

use super::drill::CommitFileSource;

/// User or system input that changes TUI state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    /**
     * Double Ctrl-C quit chord.
     *
     * First press prompts; second within [`super::ctrl_c_exit::CTRL_C_EXIT_MS`]
     * quits. `q` stays [`Self::Quit`].
     */
    CtrlC,
    ToggleHelp,
    Move(i32),
    MoveToStart,
    MoveToEnd,
    PageMove(i32),
    FoldToggle,
    /**
     * Second `z` within [`super::keys::DOUBLE_TAP_MS`].
     *
     * Subtree fold. First `z` already applied [`Self::FoldToggle`].
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
    /// Horizontal pan on the focused pane (tree, graph, commit-files, or diff).
    ///
    /// Positive looks right. Rows stay clipped to the pane; this only
    /// shifts the viewport. Keyboard `h` / `l` / Shift+arrows.
    PanDiff(i32),
    /// Toggle unlimited `-U` context on the focused file diff.
    ToggleFullContext,
    Click {
        col: u16,
        row: u16,
    },
    Drag {
        col: u16,
        row: u16,
    },
    Release,
    ToggleDiffMode,
    /// Mouse wheel. Positive `delta` is down / right.
    ///
    /// Vertical (`horizontal: false`) moves a list cursor or scrolls the
    /// graph / diff viewport. Horizontal (wheel left/right, or Shift+wheel)
    /// pans the pane under the pointer without changing the focused row.
    /// Over the workspace tree that matches the right pane: scroll does not
    /// steal the cursor. When a file diff has long lines, trackpad hscroll
    /// over the left pane pans that diff. Click still selects. Keyboard
    /// `h` / `l` are unchanged.
    ScrollWheel {
        col: u16,
        row: u16,
        delta: i32,
        horizontal: bool,
    },
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
    GraphMerge,
    /// Open the graph branch-focus overlay (`o`). Graph list only.
    GraphFocusBranches,
    /// Restore `--all` history (`O`). Graph list only.
    GraphFocusClear,
    GraphFocusMove(i32),
    GraphFocusChar(char),
    GraphFocusBackspace,
    GraphFocusToggle,
    GraphFocusSubmit,
    GraphFocusCancel,
    CycleTheme,
    ToggleMouse,
    /// Terminal size changed. Crossterm `Resize` carries the new cols/rows;
    /// ioctl can still report the previous size when the event arrives.
    Resize {
        cols: u16,
        rows: u16,
    },
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
    Fetch {
        repos: Vec<String>,
    },
    Pull {
        repos: Vec<String>,
    },
    DefaultBranch {
        repos: Vec<String>,
    },
    /// Reload every checkout (`r` on the workspace row or No-updates group).
    ReloadSnapshot,
    /// Reload one checkout (`r` on a repo, checkout, file, or dir row).
    ReloadRepo {
        repo: String,
    },
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
    Push {
        repos: Vec<String>,
    },
    PrepareStashMenu {
        repo: String,
    },
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
    PrepareBranchPicker {
        repo: String,
    },
    /// List local branches for the graph focus overlay (pumped git).
    PrepareGraphFocusPicker {
        repo: String,
    },
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
    MergeIntoHead {
        repo: String,
        /// Branch name, `origin/…`, or commit SHA (tags resolve to the commit).
        rev: String,
        /// Overlay / status label for `rev`.
        label: String,
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
