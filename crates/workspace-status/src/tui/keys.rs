//! Crossterm events to [`Action`].

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::action::Action;

/// Window for `zz` / `gg` after the first key.
pub const DOUBLE_TAP_MS: u64 = 400;

/// How the keymap reads the next key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Normal { search_active: bool },
    /// First `z` already toggled; second `z` in the window is subtree.
    ZPending { search_active: bool },
    /// First `g` armed; second `g` in the window is `gg` (move to start).
    GPending { search_active: bool },
    SearchPrompt,
    Confirm,
    Help,
    /// `?` overlay with `/` help query open (chars append; highlight only).
    HelpSearch,
    StashMenu,
    BranchPicker,
    CreateBranch,
    EasyMotion,
}

/// Map one terminal event to an [`Action`].
///
/// Key release events are ignored. Repeat is accepted for movement.
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
pub fn event_to_action_ex(
    event: &Event,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
) -> Action {
    match event {
        Event::Resize(cols, rows) => Action::Resize {
            cols: *cols,
            rows: *rows,
        },
        Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
            key_to_action(
                *key,
                mode,
                right_is_diff,
                focus_right,
                graph_stash_focused,
                graph_commit_focused,
            )
        }
        Event::Mouse(mouse) => {
            if matches!(
                mode,
                InputMode::SearchPrompt
                    | InputMode::Confirm
                    | InputMode::Help
                    | InputMode::HelpSearch
                    | InputMode::StashMenu
                    | InputMode::BranchPicker
                    | InputMode::CreateBranch
                    | InputMode::EasyMotion,
            ) {
                Action::None
            } else {
                mouse_to_action(*mouse)
            }
        }
        _ => Action::None,
    }
}

fn key_to_action(
    key: KeyEvent,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
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
        InputMode::EasyMotion => match key.code {
            KeyCode::Esc => Action::EasyMotionCancel,
            KeyCode::Char(c)
                if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Action::EasyMotionChar(c.to_ascii_lowercase())
            }
            _ => Action::None,
        },
        InputMode::Normal { search_active } => {
            normal_key(
                key,
                search_active,
                right_is_diff,
                focus_right,
                graph_stash_focused,
                graph_commit_focused,
            )
        }
    }
}

fn normal_key(
    key: KeyEvent,
    search_active: bool,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
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
    if is_easy_motion_start(key) {
        return Action::EasyMotionStart;
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
        KeyCode::Char('h') | KeyCode::Left => {
            if focus_right && right_is_diff {
                Action::PanDiff(-1)
            } else if focus_right {
                Action::None
            } else {
                Action::FoldClose
            }
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if focus_right && right_is_diff {
                Action::PanDiff(1)
            } else if focus_right {
                Action::None
            } else {
                Action::FoldOpen
            }
        }
        KeyCode::Enter => Action::NavEnter,
        KeyCode::Esc => Action::NavEsc,
        _ => Action::None,
    }
}

fn is_easy_motion_start(key: KeyEvent) -> bool {
    if key.code == KeyCode::Char(';') && !key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if key.code == KeyCode::Null {
        return true;
    }
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    matches!(key.code, KeyCode::Char(' ') | KeyCode::Char('`'))
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
        MouseEventKind::ScrollDown => Action::ScrollWheel {
            col: mouse.column,
            row: mouse.row,
            delta: 1,
        },
        MouseEventKind::ScrollUp => Action::ScrollWheel {
            col: mouse.column,
            row: mouse.row,
            delta: -1,
        },
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
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
            Action::Resize { cols: 120, rows: 40 }
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
        assert_eq!(event_to_action(&key(KeyCode::Char('q')), normal(), false, false), Action::Quit);
        assert_eq!(event_to_action(&ctrl(KeyCode::Char('c')), normal(), false, false), Action::CtrlC);
        assert_eq!(event_to_action(&ctrl(KeyCode::Char('c')), InputMode::Help, false, false), Action::CtrlC);
        assert_eq!(event_to_action(&ctrl(KeyCode::Char('c')), InputMode::Confirm, false, false), Action::CtrlC);
        assert_eq!(event_to_action(&ctrl(KeyCode::Char('c')), InputMode::SearchPrompt, false, false), Action::CtrlC);
        assert_eq!(event_to_action(&key(KeyCode::Char('?')), normal(), false, false), Action::ToggleHelp);
        assert_eq!(event_to_action(&key(KeyCode::Char('.')), normal(), false, false), Action::ToggleShowIgnored);
        assert_eq!(event_to_action(&key(KeyCode::Char('f')), normal(), false, false), Action::Fetch);
        assert_eq!(event_to_action(&key(KeyCode::Char('p')), normal(), false, false), Action::Pull);
        assert_eq!(event_to_action(&key(KeyCode::Char('d')), normal(), false, false), Action::DefaultBranch);
        assert_eq!(event_to_action(&key(KeyCode::Char('j')), normal(), false, false), Action::Move(1));
        assert_eq!(event_to_action(&key(KeyCode::Char('k')), normal(), false, false), Action::Move(-1));
        assert_eq!(event_to_action(&key(KeyCode::Char('z')), normal(), false, false), Action::FoldToggle);
        assert_eq!(event_to_action(&key(KeyCode::Left), normal(), false, false), Action::FoldClose);
        assert_eq!(event_to_action(&key(KeyCode::Right), normal(), false, false), Action::FoldOpen);
    }

    #[test]
    fn search_and_file_keys() {
        assert_eq!(event_to_action(&key(KeyCode::Char('/')), normal(), false, false), Action::SearchStart);
        assert_eq!(event_to_action(&key(KeyCode::Char('s')), normal(), false, false), Action::Stage);
        assert_eq!(event_to_action(&key(KeyCode::Char('u')), normal(), false, false), Action::Unstage);
        assert_eq!(event_to_action(&key(KeyCode::Char('x')), normal(), false, false), Action::Revert);
        assert_eq!(event_to_action(&key(KeyCode::Char('e')), normal(), false, false), Action::Edit);
        assert_eq!(
            event_to_action(&key(KeyCode::Char('n')), normal(), false, false),
            Action::None
        );
        let armed = InputMode::Normal {
            search_active: true,
        };
        assert_eq!(event_to_action(&key(KeyCode::Char('n')), armed, false, false), Action::SearchNext);
        assert_eq!(event_to_action(&key(KeyCode::Char('N')), armed, false, false), Action::SearchPrev);
    }

    #[test]
    fn search_prompt_eats_chars() {
        let mode = InputMode::SearchPrompt;
        assert_eq!(event_to_action(&key(KeyCode::Char('s')), mode, false, false), Action::SearchChar('s'));
        assert_eq!(event_to_action(&key(KeyCode::Enter), mode, false, false), Action::SearchSubmit);
        assert_eq!(event_to_action(&key(KeyCode::Esc), mode, false, false), Action::SearchCancel);
        assert_eq!(event_to_action(&key(KeyCode::Backspace), mode, false, false), Action::SearchBackspace);
    }

    #[test]
    fn confirm_y_n() {
        let mode = InputMode::Confirm;
        assert_eq!(event_to_action(&key(KeyCode::Char('y')), mode, false, false), Action::ConfirmYes);
        assert_eq!(event_to_action(&key(KeyCode::Char('Y')), mode, false, false), Action::ConfirmYesClean);
        assert_eq!(event_to_action(&key(KeyCode::Char('n')), mode, false, false), Action::ConfirmNo);
        assert_eq!(event_to_action(&key(KeyCode::Char('s')), mode, false, false), Action::None);
    }

    #[test]
    fn help_overlay_swallows_ops() {
        assert_eq!(event_to_action(&key(KeyCode::Char('f')), InputMode::Help, false, false), Action::None);
        assert_eq!(event_to_action(&key(KeyCode::Char('?')), InputMode::Help, false, false), Action::ToggleHelp);
        assert_eq!(event_to_action(&key(KeyCode::Esc), InputMode::Help, false, false), Action::ToggleHelp);
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
        assert_eq!(event_to_action(&key(KeyCode::Char('S')), normal(), false, false), Action::StashMenu);
        assert_eq!(event_to_action(&key(KeyCode::Char('P')), normal(), false, false), Action::Push);
        assert_eq!(event_to_action(&key(KeyCode::Char('b')), normal(), false, false), Action::Branch);
        assert_eq!(event_to_action(&key(KeyCode::Char('w')), normal(), false, false), Action::RemoveWorktree);
        assert_eq!(event_to_action(&key(KeyCode::Char('W')), normal(), false, false), Action::RemoveWorktree);
        assert_eq!(
            event_to_action(&key(KeyCode::Char('p')), InputMode::StashMenu, false, false),
            Action::StashMenuChar('p')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), InputMode::StashMenu, false, false),
            Action::StashMenuCancel
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('C')), InputMode::BranchPicker, false, false),
            Action::CreateBranchStart
        );
    }

    #[test]
    fn enter_esc_and_graph_stash_keys() {
        assert_eq!(event_to_action(&key(KeyCode::Enter), normal(), false, true), Action::NavEnter);
        assert_eq!(event_to_action(&key(KeyCode::Esc), normal(), false, true), Action::NavEsc);
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
            event_to_action_ex(&key(KeyCode::Char('p')), normal(), false, true, false, false),
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
            event_to_action(&key(KeyCode::Char('j')), InputMode::BranchPicker, false, false),
            Action::BranchMove(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Enter), InputMode::CreateBranch, false, false),
            Action::CreateBranchSubmit
        );
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
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

    fn ctrl(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    #[test]
    fn easy_motion_and_theme_keys() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char(';')), normal(), false, false),
            Action::EasyMotionStart
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char(' ')), normal(), false, false),
            Action::EasyMotionStart
        );
        assert_eq!(
            event_to_action(&ctrl(KeyCode::Char('`')), normal(), false, false),
            Action::EasyMotionStart
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Null), normal(), false, false),
            Action::EasyMotionStart
        );
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
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), InputMode::EasyMotion, false, false),
            Action::EasyMotionCancel
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('a')), InputMode::EasyMotion, false, false),
            Action::EasyMotionChar('a')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('A')), InputMode::EasyMotion, false, false),
            Action::EasyMotionChar('a')
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('q')), InputMode::EasyMotion, false, false),
            Action::EasyMotionChar('q')
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
            Action::None
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('l')), normal(), false, true),
            Action::None
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
}
