//! Crossterm events to [`Action`].

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::action::Action;

/// Window for `zz` / `gg` after the first key.
pub const DOUBLE_TAP_MS: u64 = 400;

/// Poll while a nav key may still be held, so terminal Repeat arrives at
/// key-repeat cadence instead of the idle 200ms tick.
pub const NAV_REPEAT_POLL_MS: u64 = 16;

/// How the keymap reads the next key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Normal {
        search_active: bool,
    },
    /// First `z` already toggled; second `z` in the window is subtree.
    ZPending {
        search_active: bool,
    },
    /// First `g` armed; second `g` in the window is `gg` (move to start).
    GPending {
        search_active: bool,
    },
    SearchPrompt,
    Confirm,
    Help,
    /// `?` overlay with `/` help query open (chars append; highlight only).
    HelpSearch,
    StashMenu,
    BranchPicker,
    CreateBranch,
}

/// Map one terminal event to an [`Action`].
///
/// Key release is ignored. Repeat fires only for held nav (`h`/`j`/`k`/`l`,
/// arrows, page, Ctrl-u/d) and for overlay typing. Repeat of `z` / `g` /
/// writes / quit is ignored so chords stay one-shot.
#[allow(dead_code)]
pub fn event_to_action(
    event: &Event,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
) -> Action {
    event_to_action_ex(event, mode, right_is_diff, focus_right, false, false)
}

/// Map one terminal event to an [`Action`], including graph-stash and graph-commit keys.
///
/// `hl_folds` is true when `h` / `l` / arrows should fold the workspace
/// tree. Graph, commit-file, and diff focus pass false so those keys pan.
pub fn event_to_action_ex(
    event: &Event,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
) -> Action {
    event_to_action_with(
        event,
        mode,
        right_is_diff,
        focus_right,
        graph_stash_focused,
        graph_commit_focused,
        true,
    )
}

/// [`event_to_action_ex`] with an explicit fold-vs-pan flag for `h` / `l`.
pub fn event_to_action_with(
    event: &Event,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
    hl_folds: bool,
) -> Action {
    match event {
        Event::Resize(cols, rows) => Action::Resize {
            cols: *cols,
            rows: *rows,
        },
        Event::Key(key) => key_event_to_action(
            *key,
            mode,
            right_is_diff,
            focus_right,
            graph_stash_focused,
            graph_commit_focused,
            hl_folds,
        ),
        Event::Mouse(mouse) => {
            if matches!(
                mode,
                InputMode::SearchPrompt
                    | InputMode::Confirm
                    | InputMode::Help
                    | InputMode::HelpSearch
                    | InputMode::StashMenu
                    | InputMode::BranchPicker
                    | InputMode::CreateBranch,
            ) {
                Action::None
            } else {
                mouse_to_action(*mouse)
            }
        }
        _ => Action::None,
    }
}

fn key_event_to_action(
    key: KeyEvent,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
    hl_folds: bool,
) -> Action {
    match key.kind {
        KeyEventKind::Release => Action::None,
        KeyEventKind::Press => key_to_action(
            key,
            mode,
            right_is_diff,
            focus_right,
            graph_stash_focused,
            graph_commit_focused,
            hl_folds,
        ),
        KeyEventKind::Repeat => {
            if repeat_maps_to_action(key, mode) {
                key_to_action(
                    key,
                    mode,
                    right_is_diff,
                    focus_right,
                    graph_stash_focused,
                    graph_commit_focused,
                    hl_folds,
                )
            } else {
                Action::None
            }
        }
    }
}

/// Keys that fire again while held (typical terminal key-repeat).
pub(crate) fn key_repeats_while_held(key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(key.code, KeyCode::Char('u') | KeyCode::Char('d'));
    }
    matches!(
        key.code,
        KeyCode::Char('h')
            | KeyCode::Char('j')
            | KeyCode::Char('k')
            | KeyCode::Char('l')
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
    )
}

fn repeat_maps_to_action(key: KeyEvent, mode: InputMode) -> bool {
    if key_repeats_while_held(key) {
        return true;
    }
    let typing = !key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Backspace | KeyCode::Char(_));
    match mode {
        InputMode::SearchPrompt | InputMode::HelpSearch | InputMode::CreateBranch => typing,
        InputMode::BranchPicker => match key.code {
            KeyCode::Backspace => true,
            KeyCode::Char('C') => false,
            KeyCode::Char(_) => typing,
            _ => false,
        },
        InputMode::Normal { .. }
        | InputMode::ZPending { .. }
        | InputMode::GPending { .. }
        | InputMode::Confirm
        | InputMode::Help
        | InputMode::StashMenu => false,
    }
}

/// Queued press / repeat / release of the same held nav key.
///
/// Dropped after one move so a hold cannot flush as a burst after release.
pub(crate) fn is_held_nav_backlog(held: KeyEvent, event: &Event) -> bool {
    let Event::Key(next) = event else {
        return false;
    };
    if !key_repeats_while_held(held) {
        return false;
    }
    next.code == held.code
        && next.modifiers == held.modifiers
        && matches!(
            next.kind,
            KeyEventKind::Press | KeyEventKind::Repeat | KeyEventKind::Release
        )
}

/// Press or Repeat of a nav key that should arm the short repeat poll.
pub(crate) fn held_nav_key(event: &Event) -> Option<KeyEvent> {
    match event {
        Event::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && key_repeats_while_held(*key) =>
        {
            Some(*key)
        }
        _ => None,
    }
}

fn key_to_action(
    key: KeyEvent,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
    hl_folds: bool,
) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::CtrlC;
    }
    match mode {
        InputMode::Help => match key.code {
            KeyCode::Char('/') => Action::SearchStart,
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') => Action::ToggleHelp,
            _ => Action::None,
        },
        InputMode::HelpSearch => match key.code {
            KeyCode::Esc => Action::SearchCancel,
            KeyCode::Backspace => Action::SearchBackspace,
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::SearchBackspace
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::SearchChar(c)
            }
            _ => Action::None,
        },
        InputMode::ZPending { search_active } => match key.code {
            KeyCode::Char('z') => Action::FoldToggleSubtree,
            KeyCode::Esc => Action::None,
            _ => normal_key(
                key,
                search_active,
                right_is_diff,
                focus_right,
                graph_stash_focused,
                graph_commit_focused,
                hl_folds,
            ),
        },
        InputMode::GPending { search_active } => match key.code {
            KeyCode::Char('g') => Action::MoveToStart,
            KeyCode::Esc => Action::None,
            _ => normal_key(
                key,
                search_active,
                right_is_diff,
                focus_right,
                graph_stash_focused,
                graph_commit_focused,
                hl_folds,
            ),
        },
        InputMode::Confirm => match key.code {
            KeyCode::Char('Y') => Action::ConfirmYesClean,
            KeyCode::Char('y') | KeyCode::Enter => Action::ConfirmYes,
            KeyCode::Char('n') | KeyCode::Esc => Action::ConfirmNo,
            _ => Action::None,
        },
        InputMode::SearchPrompt => match key.code {
            KeyCode::Esc => Action::SearchCancel,
            KeyCode::Enter => Action::SearchSubmit,
            KeyCode::Backspace => Action::SearchBackspace,
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::SearchBackspace
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::SearchChar(c)
            }
            _ => Action::None,
        },
        InputMode::StashMenu => match key.code {
            KeyCode::Esc => Action::StashMenuCancel,
            KeyCode::Enter => Action::StashMenuEnter,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::StashMenuChar(c)
            }
            _ => Action::None,
        },
        InputMode::BranchPicker => match key.code {
            KeyCode::Esc => Action::BranchCancel,
            KeyCode::Enter => Action::BranchSubmit,
            KeyCode::Backspace => Action::BranchBackspace,
            KeyCode::Char('j') | KeyCode::Down => Action::BranchMove(1),
            KeyCode::Char('k') | KeyCode::Up => Action::BranchMove(-1),
            KeyCode::Char('C') => Action::CreateBranchStart,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::BranchChar(c)
            }
            _ => Action::None,
        },
        InputMode::CreateBranch => match key.code {
            KeyCode::Esc => Action::CreateBranchCancel,
            KeyCode::Enter => Action::CreateBranchSubmit,
            KeyCode::Backspace => Action::CreateBranchBackspace,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::CreateBranchChar(c)
            }
            _ => Action::None,
        },
        InputMode::Normal { search_active } => normal_key(
            key,
            search_active,
            right_is_diff,
            focus_right,
            graph_stash_focused,
            graph_commit_focused,
            hl_folds,
        ),
    }
}

fn normal_key(
    key: KeyEvent,
    search_active: bool,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
    hl_folds: bool,
) -> Action {
    if graph_stash_focused {
        match key.code {
            KeyCode::Char('a') => return Action::GraphStashApply,
            KeyCode::Char('p') => return Action::GraphStashPop,
            KeyCode::Char('D') => return Action::GraphStashDrop,
            _ => {}
        }
    }
    if graph_commit_focused {
        match key.code {
            KeyCode::Char('b') => return Action::GraphCheckout,
            KeyCode::Char('c') => return Action::GraphCreateBranch,
            KeyCode::Char('m') => return Action::GraphMerge,
            _ => {}
        }
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
        return Action::ToggleFullContext;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
        return if focus_right && right_is_diff {
            Action::ScrollDiff(-5)
        } else {
            Action::Move(-5)
        };
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
        return if focus_right && right_is_diff {
            Action::ScrollDiff(5)
        } else {
            Action::Move(5)
        };
    }
    match key.code {
        KeyCode::Char('T') => Action::CycleTheme,
        KeyCode::Char('t') => Action::ToggleTreeMode,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('.') => Action::ToggleShowIgnored,
        KeyCode::Char('f') => Action::Fetch,
        KeyCode::Char('p') => Action::Pull,
        KeyCode::Char('d') => Action::DefaultBranch,
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char(' ') => Action::ToggleReviewed,
        KeyCode::Char('z') => Action::FoldToggle,
        KeyCode::Char('/') => Action::SearchStart,
        KeyCode::Char('s') => Action::Stage,
        KeyCode::Char('u') => Action::Unstage,
        KeyCode::Char('x') => Action::Revert,
        KeyCode::Char('e') => Action::Edit,
        KeyCode::Char('P') => Action::Push,
        KeyCode::Char('S') => Action::StashMenu,
        KeyCode::Char('b') => Action::Branch,
        KeyCode::Char('w') | KeyCode::Char('W') => Action::RemoveWorktree,
        KeyCode::Char('i') => Action::ToggleDiffMode,
        KeyCode::Char('m') => Action::ToggleMouse,
        KeyCode::Char('n') if search_active => Action::SearchNext,
        KeyCode::Char('N') if search_active => Action::SearchPrev,
        KeyCode::Tab => {
            if focus_right {
                Action::FocusLeft
            } else {
                Action::FocusRight
            }
        }
        KeyCode::Char('G') => Action::MoveToEnd,
        KeyCode::Char('g') => Action::ArmGChord,
        KeyCode::Home => Action::MoveToStart,
        KeyCode::End => Action::MoveToEnd,
        KeyCode::PageUp => {
            if focus_right && right_is_diff {
                Action::ScrollDiff(-10)
            } else {
                Action::PageMove(-1)
            }
        }
        KeyCode::PageDown => {
            if focus_right && right_is_diff {
                Action::ScrollDiff(10)
            } else {
                Action::PageMove(1)
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if focus_right && right_is_diff {
                Action::ScrollDiff(1)
            } else {
                Action::Move(1)
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if focus_right && right_is_diff {
                Action::ScrollDiff(-1)
            } else {
                Action::Move(-1)
            }
        }
        KeyCode::Char('h') | KeyCode::Left => hl_or_pan(key, -1, focus_right, hl_folds),
        KeyCode::Char('l') | KeyCode::Right => hl_or_pan(key, 1, focus_right, hl_folds),
        KeyCode::Enter => Action::NavEnter,
        KeyCode::Esc => Action::NavEsc,
        _ => Action::None,
    }
}

/// `h` / `l` / arrows: fold the workspace tree, otherwise pan.
///
/// Shift+Left / Shift+Right always pan so long tree paths can move without
/// stealing fold. Unshifted keys still fold when `hl_folds` is set and the
/// left tree is focused.
fn hl_or_pan(key: KeyEvent, delta: i32, focus_right: bool, hl_folds: bool) -> Action {
    let pan = if delta < 0 {
        Action::PanDiff(-1)
    } else {
        Action::PanDiff(1)
    };
    if key.modifiers.contains(KeyModifiers::SHIFT) || !hl_folds || focus_right {
        return pan;
    }
    if delta < 0 {
        Action::FoldClose
    } else {
        Action::FoldOpen
    }
}

fn mouse_to_action(mouse: MouseEvent) -> Action {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Action::Click {
            col: mouse.column,
            row: mouse.row,
        },
        MouseEventKind::Drag(MouseButton::Left) => Action::Drag {
            col: mouse.column,
            row: mouse.row,
        },
        MouseEventKind::Up(MouseButton::Left) => Action::Release,
        MouseEventKind::ScrollDown => wheel_action(mouse, 1, false),
        MouseEventKind::ScrollUp => wheel_action(mouse, -1, false),
        MouseEventKind::ScrollLeft => wheel_action(mouse, -1, true),
        MouseEventKind::ScrollRight => wheel_action(mouse, 1, true),
        _ => Action::None,
    }
}

/// Vertical wheel, or horizontal pan (wheel left/right / Shift+wheel).
///
/// Many terminals encode trackpad hscroll as Shift+wheel rather than
/// `ScrollLeft` / `ScrollRight`. Both must pan without moving the focused
/// row on the workspace tree.
fn wheel_action(mouse: MouseEvent, delta: i32, horizontal: bool) -> Action {
    Action::ScrollWheel {
        col: mouse.column,
        row: mouse.row,
        delta,
        horizontal: horizontal || mouse.modifiers.contains(KeyModifiers::SHIFT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn key_kind(code: KeyCode, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind))
    }

    fn normal() -> InputMode {
        InputMode::Normal {
            search_active: false,
        }
    }

    #[test]
    fn resize_is_not_ignored() {
        assert_eq!(
            event_to_action(&Event::Resize(120, 40), normal(), false, false),
            Action::Resize {
                cols: 120,
                rows: 40
            }
        );
        assert_eq!(
            event_to_action(&Event::Resize(80, 24), InputMode::Help, false, false),
            Action::Resize { cols: 80, rows: 24 }
        );
        assert_eq!(
            event_to_action(&Event::Resize(60, 18), InputMode::Confirm, true, true),
            Action::Resize { cols: 60, rows: 18 }
        );
    }

    #[test]
    fn daily_keys() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('q')), normal(), false, false),
            Action::Quit
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('c')), normal(), false, false),
            Action::CtrlC
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('c')), InputMode::Help, false, false),
            Action::CtrlC
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('c')), InputMode::Confirm, false, false),
            Action::CtrlC
        );
        assert_eq!(
            event_to_action(
                &ctrl(KeyCode::Char('c')),
                InputMode::SearchPrompt,
                false,
                false
            ),
            Action::CtrlC
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('?')), normal(), false, false),
            Action::ToggleHelp
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('.')), normal(), false, false),
            Action::ToggleShowIgnored
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('f')), normal(), false, false),
            Action::Fetch
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('p')), normal(), false, false),
            Action::Pull
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('d')), normal(), false, false),
            Action::DefaultBranch
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), normal(), false, false),
            Action::Move(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('k')), normal(), false, false),
            Action::Move(-1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('z')), normal(), false, false),
            Action::FoldToggle
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Left), normal(), false, false),
            Action::FoldClose
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Right), normal(), false, false),
            Action::FoldOpen
        );
    }

    #[test]
    fn search_and_file_keys() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('/')), normal(), false, false),
            Action::SearchStart
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('s')), normal(), false, false),
            Action::Stage
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('u')), normal(), false, false),
            Action::Unstage
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('x')), normal(), false, false),
            Action::Revert
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('e')), normal(), false, false),
            Action::Edit
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('n')), normal(), false, false),
            Action::None
        );
        let armed = InputMode::Normal {
            search_active: true,
        };
        assert_eq!(
            event_to_action(&key(KeyCode::Char('n')), armed, false, false),
            Action::SearchNext
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('N')), armed, false, false),
            Action::SearchPrev
        );
    }

    #[test]
    fn search_prompt_eats_chars() {
        let mode = InputMode::SearchPrompt;
        assert_eq!(
            event_to_action(&key(KeyCode::Char('s')), mode, false, false),
            Action::SearchChar('s')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Enter), mode, false, false),
            Action::SearchSubmit
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), mode, false, false),
            Action::SearchCancel
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Backspace), mode, false, false),
            Action::SearchBackspace
        );
    }

    #[test]
    fn confirm_y_n() {
        let mode = InputMode::Confirm;
        assert_eq!(
            event_to_action(&key(KeyCode::Char('y')), mode, false, false),
            Action::ConfirmYes
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('Y')), mode, false, false),
            Action::ConfirmYesClean
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('n')), mode, false, false),
            Action::ConfirmNo
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('s')), mode, false, false),
            Action::None
        );
    }

    #[test]
    fn help_overlay_swallows_ops() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('f')), InputMode::Help, false, false),
            Action::None
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('?')), InputMode::Help, false, false),
            Action::ToggleHelp
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), InputMode::Help, false, false),
            Action::ToggleHelp
        );
    }

    #[test]
    fn right_diff_j_scrolls() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), normal(), true, true),
            Action::ScrollDiff(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), normal(), true, false),
            Action::Move(1)
        );
    }

    #[test]
    fn space_is_reviewed_not_fold() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char(' ')), normal(), false, false),
            Action::ToggleReviewed
        );
    }

    #[test]
    fn stash_push_branch_keys() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('S')), normal(), false, false),
            Action::StashMenu
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('P')), normal(), false, false),
            Action::Push
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('b')), normal(), false, false),
            Action::Branch
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('w')), normal(), false, false),
            Action::RemoveWorktree
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('W')), normal(), false, false),
            Action::RemoveWorktree
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('p')), InputMode::StashMenu, false, false),
            Action::StashMenuChar('p')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), InputMode::StashMenu, false, false),
            Action::StashMenuCancel
        );
        assert_eq!(
            event_to_action(
                &key(KeyCode::Char('C')),
                InputMode::BranchPicker,
                false,
                false
            ),
            Action::CreateBranchStart
        );
    }

    #[test]
    fn enter_esc_and_graph_stash_keys() {
        assert_eq!(
            event_to_action(&key(KeyCode::Enter), normal(), false, true),
            Action::NavEnter
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), normal(), false, true),
            Action::NavEsc
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('a')), normal(), false, true, true, false),
            Action::GraphStashApply
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('p')), normal(), false, true, true, false),
            Action::GraphStashPop
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('D')), normal(), false, true, true, false),
            Action::GraphStashDrop
        );
        assert_eq!(
            event_to_action_ex(
                &key(KeyCode::Char('p')),
                normal(),
                false,
                true,
                false,
                false
            ),
            Action::Pull
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('b')), normal(), false, true, false, true),
            Action::GraphCheckout
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('c')), normal(), false, true, false, true),
            Action::GraphCreateBranch
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('m')), normal(), false, true, false, true),
            Action::GraphMerge
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('m')), normal(), false, true, true, false),
            Action::ToggleMouse
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('b')), normal(), false, true, true, false),
            Action::Branch
        );
        assert_eq!(
            event_to_action_ex(&key(KeyCode::Char('c')), normal(), false, true, true, false),
            Action::None
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('c')), normal(), false, false),
            Action::None
        );
        assert_eq!(
            event_to_action(
                &key(KeyCode::Char('j')),
                InputMode::BranchPicker,
                false,
                false
            ),
            Action::BranchMove(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Enter), InputMode::CreateBranch, false, false),
            Action::CreateBranchSubmit
        );
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> Event {
        mouse_mods(kind, col, row, KeyModifiers::NONE)
    }

    fn mouse_mods(kind: MouseEventKind, col: u16, row: u16, modifiers: KeyModifiers) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: col,
            row,
            modifiers,
        })
    }

    #[test]
    fn mouse_drag_and_release_and_inline_key() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('i')), normal(), true, true),
            Action::ToggleDiffMode
        );
        assert_eq!(
            event_to_action(
                &mouse(MouseEventKind::Drag(MouseButton::Left), 40, 4),
                normal(),
                false,
                false
            ),
            Action::Drag { col: 40, row: 4 }
        );
        assert_eq!(
            event_to_action(
                &mouse(MouseEventKind::Up(MouseButton::Left), 40, 4),
                normal(),
                false,
                false
            ),
            Action::Release
        );
        assert_eq!(
            event_to_action(
                &mouse(MouseEventKind::Drag(MouseButton::Left), 40, 4),
                InputMode::Help,
                false,
                false
            ),
            Action::None
        );
    }

    #[test]
    fn mouse_hscroll_and_shift_wheel_map_to_horizontal_pan() {
        assert_eq!(
            event_to_action(
                &mouse(MouseEventKind::ScrollLeft, 8, 4),
                normal(),
                false,
                false
            ),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: -1,
                horizontal: true,
            }
        );
        assert_eq!(
            event_to_action(
                &mouse(MouseEventKind::ScrollRight, 8, 4),
                normal(),
                false,
                false
            ),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 1,
                horizontal: true,
            }
        );
        assert_eq!(
            event_to_action(
                &mouse_mods(MouseEventKind::ScrollUp, 8, 4, KeyModifiers::SHIFT),
                normal(),
                false,
                false
            ),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: -1,
                horizontal: true,
            }
        );
        assert_eq!(
            event_to_action(
                &mouse_mods(MouseEventKind::ScrollDown, 8, 4, KeyModifiers::SHIFT),
                normal(),
                false,
                false
            ),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 1,
                horizontal: true,
            }
        );
        assert_eq!(
            event_to_action(
                &mouse(MouseEventKind::ScrollDown, 8, 4),
                normal(),
                false,
                false
            ),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 1,
                horizontal: false,
            }
        );
    }

    #[test]
    fn sgr_trackpad_hscroll_bytes_map_to_horizontal_pan() {
        use crate::tui::tty::{
            decode_sgr_mouse, sgr_mouse_report, SGR_SHIFT_WHEEL_DOWN, SGR_WHEEL_RIGHT,
            SGR_WHEEL_RIGHT_MOTION,
        };
        let right = decode_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT, 8, 4)).unwrap();
        assert_eq!(
            event_to_action(&right, normal(), false, false),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 1,
                horizontal: true,
            }
        );
        let shift_wheel = decode_sgr_mouse(&sgr_mouse_report(SGR_SHIFT_WHEEL_DOWN, 8, 4)).unwrap();
        assert_eq!(
            event_to_action(&shift_wheel, normal(), false, false),
            Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 1,
                horizontal: true,
            }
        );
        assert!(
            decode_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT_MOTION, 8, 4)).is_none(),
            "live event::read drops SGR 99; keymap must not see a kinder decode"
        );
    }

    fn ctrl(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    fn shift(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::SHIFT))
    }

    #[test]
    fn theme_and_reviewed_keys() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('T')), normal(), false, false),
            Action::CycleTheme
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('t')), normal(), false, false),
            Action::ToggleTreeMode
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char(' ')), normal(), false, false),
            Action::ToggleReviewed
        );
    }

    #[test]
    fn ctrl_o_and_diff_pan_keys() {
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('o')), normal(), true, true),
            Action::ToggleFullContext
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('h')), normal(), true, true),
            Action::PanDiff(-1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('l')), normal(), true, true),
            Action::PanDiff(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('h')), normal(), false, false),
            Action::FoldClose
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('l')), normal(), false, false),
            Action::FoldOpen
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('h')), normal(), false, true),
            Action::PanDiff(-1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('l')), normal(), false, true),
            Action::PanDiff(1)
        );
        assert_eq!(
            event_to_action_with(
                &shift(KeyCode::Left),
                normal(),
                false,
                false,
                false,
                false,
                true
            ),
            Action::PanDiff(-1)
        );
        assert_eq!(
            event_to_action_with(
                &shift(KeyCode::Right),
                normal(),
                false,
                false,
                false,
                false,
                true
            ),
            Action::PanDiff(1)
        );
        assert_eq!(
            event_to_action_with(
                &key(KeyCode::Char('h')),
                normal(),
                false,
                false,
                false,
                false,
                false
            ),
            Action::PanDiff(-1)
        );
        assert_eq!(
            event_to_action_with(
                &key(KeyCode::Char('l')),
                normal(),
                false,
                true,
                false,
                false,
                false
            ),
            Action::PanDiff(1)
        );
    }

    #[test]
    fn help_slash_starts_help_search() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('/')), InputMode::Help, false, false),
            Action::SearchStart
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('/')), InputMode::Help, true, true),
            Action::SearchStart
        );
        let searching = InputMode::HelpSearch;
        assert_eq!(
            event_to_action(&key(KeyCode::Char('f')), searching, false, false),
            Action::SearchChar('f')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('n')), searching, false, false),
            Action::SearchChar('n')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('N')), searching, false, false),
            Action::SearchChar('N')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Enter), searching, false, false),
            Action::None
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), searching, false, false),
            Action::SearchCancel
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('q')), searching, false, false),
            Action::SearchChar('q')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('?')), searching, false, false),
            Action::SearchChar('?')
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('h')), searching, false, false),
            Action::SearchBackspace
        );
    }

    #[test]
    fn ctrl_u_d_move_five_and_m_toggles_mouse() {
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('d')), normal(), false, false),
            Action::Move(5)
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('u')), normal(), false, false),
            Action::Move(-5)
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('d')), normal(), true, true),
            Action::ScrollDiff(5)
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('u')), normal(), true, true),
            Action::ScrollDiff(-5)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('m')), normal(), false, false),
            Action::ToggleMouse
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('u')), normal(), false, false),
            Action::Unstage
        );
        assert_eq!(
            event_to_action(&key(KeyCode::PageDown), normal(), false, false),
            Action::PageMove(1)
        );
    }

    #[test]
    fn zz_chord_is_toggle_then_subtree() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('z')), normal(), false, false),
            Action::FoldToggle
        );
        let pending = InputMode::ZPending {
            search_active: false,
        };
        assert_eq!(
            event_to_action(&key(KeyCode::Char('z')), pending, false, false),
            Action::FoldToggleSubtree
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), pending, false, false),
            Action::Move(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), pending, false, false),
            Action::None
        );
    }

    #[test]
    fn gg_chord_arms_then_moves_to_start() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('g')), normal(), false, false),
            Action::ArmGChord
        );
        let pending = InputMode::GPending {
            search_active: false,
        };
        assert_eq!(
            event_to_action(&key(KeyCode::Char('g')), pending, false, false),
            Action::MoveToStart
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Home), normal(), false, false),
            Action::MoveToStart
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('G')), normal(), false, false),
            Action::MoveToEnd
        );
    }

    #[test]
    fn nav_repeat_moves_release_does_not() {
        for (code, press) in [
            (KeyCode::Char('j'), Action::Move(1)),
            (KeyCode::Char('k'), Action::Move(-1)),
            (KeyCode::Char('h'), Action::FoldClose),
            (KeyCode::Char('l'), Action::FoldOpen),
            (KeyCode::Down, Action::Move(1)),
            (KeyCode::Up, Action::Move(-1)),
            (KeyCode::Left, Action::FoldClose),
            (KeyCode::Right, Action::FoldOpen),
        ] {
            assert_eq!(
                event_to_action(
                    &key_kind(code, KeyEventKind::Repeat),
                    normal(),
                    false,
                    false
                ),
                press,
                "Repeat {code:?}"
            );
            assert_eq!(
                event_to_action(
                    &key_kind(code, KeyEventKind::Release),
                    normal(),
                    false,
                    false
                ),
                Action::None,
                "Release {code:?}"
            );
        }
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('j'), KeyEventKind::Repeat),
                normal(),
                true,
                true
            ),
            Action::ScrollDiff(1)
        );
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('h'), KeyEventKind::Repeat),
                normal(),
                true,
                true
            ),
            Action::PanDiff(-1)
        );
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('l'), KeyEventKind::Repeat),
                normal(),
                false,
                true
            ),
            Action::PanDiff(1)
        );
        assert_eq!(
            event_to_action(
                &Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('d'),
                    KeyModifiers::CONTROL,
                    KeyEventKind::Repeat
                )),
                normal(),
                false,
                false
            ),
            Action::Move(5)
        );
    }

    #[test]
    fn repeat_does_not_fire_chords_or_writes() {
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('z'),
            KeyCode::Char('g'),
            KeyCode::Char('s'),
            KeyCode::Char(' '),
            KeyCode::Char('f'),
        ] {
            assert_eq!(
                event_to_action(
                    &key_kind(code, KeyEventKind::Repeat),
                    normal(),
                    false,
                    false
                ),
                Action::None,
                "Repeat {code:?} must stay one-shot"
            );
        }
        assert_eq!(
            event_to_action(
                &Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL,
                    KeyEventKind::Repeat
                )),
                normal(),
                false,
                false
            ),
            Action::None
        );
        let pending = InputMode::ZPending {
            search_active: false,
        };
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('z'), KeyEventKind::Repeat),
                pending,
                false,
                false
            ),
            Action::None
        );
        let g = InputMode::GPending {
            search_active: false,
        };
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('g'), KeyEventKind::Repeat),
                g,
                false,
                false
            ),
            Action::None
        );
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('y'), KeyEventKind::Repeat),
                InputMode::Confirm,
                false,
                false
            ),
            Action::None
        );
    }

    #[test]
    fn repeat_still_types_in_search() {
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('a'), KeyEventKind::Repeat),
                InputMode::SearchPrompt,
                false,
                false
            ),
            Action::SearchChar('a')
        );
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('j'), KeyEventKind::Repeat),
                InputMode::BranchPicker,
                false,
                false
            ),
            Action::BranchMove(1)
        );
        assert_eq!(
            event_to_action(
                &key_kind(KeyCode::Char('C'), KeyEventKind::Repeat),
                InputMode::BranchPicker,
                false,
                false
            ),
            Action::None
        );
    }

    #[test]
    fn held_nav_backlog_drops_same_key_not_others() {
        let held = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(is_held_nav_backlog(
            held,
            &key_kind(KeyCode::Char('j'), KeyEventKind::Repeat)
        ));
        assert!(is_held_nav_backlog(
            held,
            &key_kind(KeyCode::Char('j'), KeyEventKind::Press)
        ));
        assert!(is_held_nav_backlog(
            held,
            &key_kind(KeyCode::Char('j'), KeyEventKind::Release)
        ));
        assert!(!is_held_nav_backlog(
            held,
            &key_kind(KeyCode::Char('k'), KeyEventKind::Press)
        ));
        assert!(!is_held_nav_backlog(held, &Event::Resize(80, 24)));
        assert!(!is_held_nav_backlog(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &key_kind(KeyCode::Char('q'), KeyEventKind::Repeat)
        ));
        assert!(held_nav_key(&key(KeyCode::Char('j'))).is_some());
        assert!(held_nav_key(&key_kind(KeyCode::Char('j'), KeyEventKind::Repeat)).is_some());
        assert!(held_nav_key(&key(KeyCode::Char('q'))).is_none());
        assert!(held_nav_key(&key_kind(KeyCode::Char('j'), KeyEventKind::Release)).is_none());
    }
}
