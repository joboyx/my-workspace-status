//! Crossterm events to [`Action`].

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::action::Action;

/// How the keymap reads the next key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Normal { search_active: bool },
    SearchPrompt,
    Confirm,
    Help,
    StashMenu,
    BranchPicker,
    CreateBranch,
}

/// Map one terminal event to an [`Action`].
///
/// Key release events are ignored. Repeat is accepted for movement.
pub fn event_to_action(
    event: &Event,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
) -> Action {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
            key_to_action(*key, mode, right_is_diff, focus_right)
        }
        Event::Mouse(mouse) => {
            if matches!(
                mode,
                InputMode::SearchPrompt
                    | InputMode::Confirm
                    | InputMode::Help
                    | InputMode::StashMenu
                    | InputMode::BranchPicker
                    | InputMode::CreateBranch
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
) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }
    match mode {
        InputMode::Help => match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') => Action::ToggleHelp,
            _ => Action::None,
        },
        InputMode::Confirm => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Action::ConfirmYes,
            KeyCode::Char('n') | KeyCode::Esc => Action::ConfirmNo,
            _ => Action::None,
        },
        InputMode::SearchPrompt => match key.code {
            KeyCode::Esc => Action::SearchCancel,
            KeyCode::Enter => Action::SearchSubmit,
            KeyCode::Backspace => Action::SearchBackspace,
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
        InputMode::Normal { search_active } => {
            normal_key(key, search_active, right_is_diff, focus_right)
        }
    }
}

fn normal_key(
    key: KeyEvent,
    search_active: bool,
    right_is_diff: bool,
    focus_right: bool,
) -> Action {
    match key.code {
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
        KeyCode::Char('g') => Action::MoveToStart,
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
        KeyCode::Char('h') | KeyCode::Left => Action::FoldClose,
        KeyCode::Char('l') | KeyCode::Right => Action::FoldOpen,
        KeyCode::Esc => Action::None,
        _ => Action::None,
    }
}

fn mouse_to_action(mouse: MouseEvent) -> Action {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Action::Click {
            col: mouse.column,
            row: mouse.row,
        },
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
    fn daily_keys() {
        assert_eq!(event_to_action(&key(KeyCode::Char('q')), normal(), false, false), Action::Quit);
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
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), InputMode::BranchPicker, false, false),
            Action::BranchMove(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Enter), InputMode::CreateBranch, false, false),
            Action::CreateBranchSubmit
        );
    }
}
