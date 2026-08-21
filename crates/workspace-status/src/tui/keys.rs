//! Crossterm events to [`Action`].

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::action::Action;

/// Map one terminal event to an [`Action`].
///
/// Key release / repeat events are ignored. Help overlay swallows
/// everything except `q`, `Esc`, and `?`.
pub fn event_to_action(event: &Event, help_open: bool, right_is_diff: bool, focus_right: bool) -> Action {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat => {
            key_to_action(*key, help_open, right_is_diff, focus_right)
        }
        Event::Mouse(mouse) => mouse_to_action(*mouse),
        _ => Action::None,
    }
}

fn key_to_action(key: KeyEvent, help_open: bool, right_is_diff: bool, focus_right: bool) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }
    if help_open {
        return match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') => Action::ToggleHelp,
            _ => Action::None,
        };
    }
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

    #[test]
    fn daily_keys() {
        assert_eq!(event_to_action(&key(KeyCode::Char('q')), false, false, false), Action::Quit);
        assert_eq!(event_to_action(&key(KeyCode::Char('?')), false, false, false), Action::ToggleHelp);
        assert_eq!(event_to_action(&key(KeyCode::Char('.')), false, false, false), Action::ToggleShowIgnored);
        assert_eq!(event_to_action(&key(KeyCode::Char('f')), false, false, false), Action::Fetch);
        assert_eq!(event_to_action(&key(KeyCode::Char('p')), false, false, false), Action::Pull);
        assert_eq!(event_to_action(&key(KeyCode::Char('d')), false, false, false), Action::DefaultBranch);
        assert_eq!(event_to_action(&key(KeyCode::Char('j')), false, false, false), Action::Move(1));
        assert_eq!(event_to_action(&key(KeyCode::Char('k')), false, false, false), Action::Move(-1));
        assert_eq!(event_to_action(&key(KeyCode::Char('z')), false, false, false), Action::FoldToggle);
        assert_eq!(event_to_action(&key(KeyCode::Left), false, false, false), Action::FoldClose);
        assert_eq!(event_to_action(&key(KeyCode::Right), false, false, false), Action::FoldOpen);
    }

    #[test]
    fn help_overlay_swallows_ops() {
        assert_eq!(event_to_action(&key(KeyCode::Char('f')), true, false, false), Action::None);
        assert_eq!(event_to_action(&key(KeyCode::Char('?')), true, false, false), Action::ToggleHelp);
        assert_eq!(event_to_action(&key(KeyCode::Esc), true, false, false), Action::ToggleHelp);
    }

    #[test]
    fn right_diff_j_scrolls() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), false, true, true),
            Action::ScrollDiff(1)
        );
        assert_eq!(
            event_to_action(&key(KeyCode::Char('j')), false, true, false),
            Action::Move(1)
        );
    }

    #[test]
    fn space_is_reviewed_not_fold() {
        assert_eq!(
            event_to_action(&key(KeyCode::Char(' ')), false, false, false),
            Action::ToggleReviewed
        );
    }
}
