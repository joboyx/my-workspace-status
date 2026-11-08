use super::*;
use crate::testutil::{ctrl, key};

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
fn angle_brackets_resize_the_tree_in_normal_mode_only() {
    assert_eq!(
        event_to_action(&key(KeyCode::Char('<')), normal(), false, false),
        Action::ResizeTree(-1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('>')), normal(), true, true),
        Action::ResizeTree(1)
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Char('>')),
            InputMode::SearchPrompt,
            false,
            false
        ),
        Action::SearchChar('>')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('<')), InputMode::Help, false, false),
        Action::None
    );
}

/// `-` is [`Action::ExplorerReveal`] (dispatch opens Explorer, goes to
/// the parent folder, or refuses); `+` / `=` are unbound. The commit
/// message height is the palette rows only.
#[test]
fn minus_is_explorer_and_plus_equals_are_unbound_in_normal_mode() {
    for (code, want) in [
        (KeyCode::Char('-'), Action::ExplorerReveal),
        (KeyCode::Char('+'), Action::None),
        (KeyCode::Char('='), Action::None),
    ] {
        assert_eq!(event_to_action(&key(code), normal(), false, false), want);
        assert_eq!(event_to_action(&key(code), normal(), true, true), want);
    }
    // Terminals may report `+` with Shift held.
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('+')), normal(), false, false),
        Action::None
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Char('-')),
            InputMode::SearchPrompt,
            false,
            false
        ),
        Action::SearchChar('-')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('+')), InputMode::Help, false, false),
        Action::None
    );
    // The list keys stay on the list.
    assert_eq!(
        event_to_action(&key(KeyCode::Char('j')), normal(), false, false),
        Action::Move(1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Down), normal(), false, false),
        Action::Move(1)
    );
}

#[test]
fn right_click_is_back_outside_overlays() {
    let right = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(
        event_to_action(&right, normal(), false, true),
        Action::BackClick
    );
    assert_eq!(
        event_to_action(&right, InputMode::DiffVisual, true, true),
        Action::BackClick
    );
    for mode in [
        InputMode::Help,
        InputMode::Confirm,
        InputMode::BranchPicker,
        InputMode::QuickOpen,
        InputMode::SearchPrompt,
    ] {
        assert_eq!(event_to_action(&right, mode, false, true), Action::None);
    }
    // A pending chord maps right-click exactly like Esc: no step back.
    for mode in [
        InputMode::ZPending {
            search_active: false,
        },
        InputMode::GPending {
            search_active: false,
        },
    ] {
        assert_eq!(
            event_to_action(&right, mode, false, true),
            event_to_action(&key(KeyCode::Esc), mode, false, true),
            "{mode:?}"
        );
        assert_ne!(
            event_to_action(&right, mode, false, true),
            Action::BackClick
        );
    }
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
        Action::ToggleShowHidden
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
        event_to_action(&key(KeyCode::Char('E')), normal(), false, false),
        Action::ExternalDiff
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('e')), normal(), false, false),
        Action::ExternalDiff
    );
    // Unarmed `n` still dispatches so the status can say "no search".
    assert_eq!(
        event_to_action(&key(KeyCode::Char('n')), normal(), false, false),
        Action::SearchNext
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
        event_to_action(&key(KeyCode::Char('E')), mode, false, false),
        Action::SearchChar('E')
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('e')), mode, false, false),
        Action::SearchChar('E')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('\'')), mode, false, false),
        Action::SearchChar('\'')
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
        event_to_action(&key(KeyCode::Esc), mode, false, false),
        Action::ConfirmNo
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Enter), mode, false, false),
        Action::ConfirmEnter
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
fn right_diff_j_moves_focused_row() {
    assert_eq!(
        event_to_action(&key(KeyCode::Char('j')), normal(), true, true),
        Action::Move(1)
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
        Action::None,
        "unshifted w must not remove a worktree"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('W')), normal(), false, false),
        Action::RemoveWorktree
    );
    assert_eq!(
        event_to_action(
            &Event::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::SHIFT)),
            normal(),
            false,
            false
        ),
        Action::RemoveWorktree,
        "CSI-u Shift+w folds to W"
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
        Action::BranchChar('C'),
        "C types into the filter; the create row replaced picker C"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('o')), normal(), false, true),
        Action::GraphFocusBranches
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('O')), normal(), false, true),
        Action::GraphFocusClear
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Char(' ')),
            InputMode::GraphFocusPicker,
            false,
            true
        ),
        Action::GraphFocusToggle
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Enter),
            InputMode::GraphFocusPicker,
            false,
            true
        ),
        Action::GraphFocusSubmit
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Char('O')),
            InputMode::GraphFocusPicker,
            false,
            true
        ),
        Action::GraphFocusChar('O')
    );
    assert_eq!(
        event_to_action(
            &ctrl(KeyCode::Char('o')),
            InputMode::GraphFocusPicker,
            false,
            true
        ),
        Action::GraphFocusClear
    );
}

#[test]
fn shift_letter_folds_to_uppercase_binding() {
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('o')), normal(), false, true),
        Action::GraphFocusClear
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('s')), normal(), false, false),
        Action::StashMenu
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('p')), normal(), false, false),
        Action::Push
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('e')), normal(), false, false),
        Action::ExternalDiff
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('m')), normal(), false, false),
        Action::ToggleCommitMsgExpand
    );
    assert_eq!(
        event_to_action(
            &shift(KeyCode::Char('m')),
            InputMode::SearchPrompt,
            false,
            false
        ),
        Action::SearchChar('M')
    );
    assert_eq!(
        event_to_action(
            &shift(KeyCode::Char('o')),
            InputMode::GraphFocusPicker,
            false,
            true
        ),
        Action::GraphFocusChar('O')
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('y')), InputMode::Confirm, false, false),
        Action::ConfirmYesClean
    );
    assert_eq!(
        event_to_action_with(
            &shift(KeyCode::Char('h')),
            normal(),
            false,
            false,
            false,
            false,
            true
        ),
        Action::PanDiff(-1)
    );
}

/// A kitty Shift+symbol report with the shifted-key field
/// (`REPORT_ALTERNATE_KEYS`) acts as the shifted character, not as the
/// unshifted key: `:` opens the command palette, not `;`. A letter keeps
/// SHIFT like its legacy byte, and Ctrl-Shift keeps the base key.
#[test]
fn shifted_symbol_reports_act_as_the_shifted_key() {
    use super::super::action::QuickOpenEntry;
    use super::super::tty::parse_tty_chunk;
    let decode = |bytes: &[u8]| {
        let events = parse_tty_chunk(bytes);
        assert_eq!(events.len(), 1, "{bytes:?}");
        events[0].0.clone()
    };
    let cases: [(&[u8], InputMode, Action); 6] = [
        (
            b"\x1b[59:58;2:1u",
            normal(),
            Action::ToggleQuickOpen(QuickOpenEntry::Commands),
        ),
        (b"\x1b[47:63;2:1u", normal(), Action::ToggleHelp),
        (b"\x1b[46:62;2:1u", normal(), Action::ResizeTree(1)),
        (
            b"\x1b[45:95;2:1u",
            InputMode::SearchPrompt,
            Action::SearchChar('_'),
        ),
        (
            b"\x1b[49:33;2:1u",
            InputMode::SearchPrompt,
            Action::SearchChar('!'),
        ),
        (b"\x1b[115:83;2:1u", normal(), Action::StashMenu),
    ];
    for (bytes, mode, action) in cases {
        assert_eq!(
            event_to_action(&decode(bytes), mode, false, false),
            action,
            "{bytes:?}"
        );
    }
    assert_eq!(
        decode(b"\x1b[59:58;2:1u"),
        Event::Key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
    );
    assert_eq!(
        decode(b"\x1b[104:72;2:1u"),
        Event::Key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT))
    );
    assert_eq!(
        decode(b"\x1b[117:85;6:1u"),
        Event::Key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ))
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
        event_to_action_ex(&key(KeyCode::Char('M')), normal(), false, true, false, true),
        Action::ToggleCommitMsgExpand
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
        event_to_action(&key(KeyCode::Char(';')), normal(), false, false),
        Action::CommentStart
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('\'')), normal(), false, false),
        Action::CopyEntityReference
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('V')), normal(), true, true),
        Action::DiffVisualStart
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('v')), normal(), true, true),
        Action::DiffVisualStart
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Esc), InputMode::DiffVisual, true, true),
        Action::DiffVisualCancel
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('V')), InputMode::DiffVisual, true, true),
        Action::DiffVisualCancel
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char(';')), InputMode::DiffVisual, true, true),
        Action::CommentStart
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('\'')), InputMode::DiffVisual, true, true),
        Action::CopyEntityReference
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('j')), InputMode::DiffVisual, true, true),
        Action::Move(1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Down), InputMode::DiffVisual, true, true),
        Action::Move(1)
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Char(char::from_u32(57353).unwrap())),
            InputMode::DiffVisual,
            true,
            true
        ),
        Action::Move(1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('k')), InputMode::DiffVisual, true, true),
        Action::Move(-1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Up), InputMode::DiffVisual, true, true),
        Action::Move(-1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('s')), InputMode::DiffVisual, true, true),
        Action::Stage
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('u')), InputMode::DiffVisual, true, true),
        Action::Unstage
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('x')), InputMode::DiffVisual, true, true),
        Action::Revert
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('y')), normal(), false, false),
        Action::ExportComments
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('\'')), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Char('\''), KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Enter), InputMode::Comment, false, false),
        Action::CommentSubmit
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Enter), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Left), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Right), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Home), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::End), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Delete), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('a')), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('e')), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL))
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Left), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL))
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Right), InputMode::Comment, false, false),
        Action::CommentInput(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL))
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('r')), InputMode::Comment, false, false),
        Action::CommentToggleResolved
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('R')), InputMode::Comment, false, false),
        Action::CommentToggleResolved
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Char(char::from_u32(57350).unwrap())),
            InputMode::Comment,
            false,
            false
        ),
        Action::CommentInput(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Esc), InputMode::CommentExport, false, false),
        Action::ExportCommentsCancel
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Down), InputMode::BranchPicker, false, false),
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
        event_to_action(&key(KeyCode::Char('\\')), normal(), true, true),
        Action::ToggleDiffWrap
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('\\')), normal(), false, false),
        Action::ToggleDiffWrap
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('M')), normal(), false, false),
        Action::ToggleCommitMsgExpand
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('M')), normal(), true, true),
        Action::ToggleCommitMsgExpand
    );
    for (right_is_diff, focus_right) in [(false, false), (true, true)] {
        assert_eq!(
            event_to_action(
                &key(KeyCode::Char('B')),
                normal(),
                right_is_diff,
                focus_right
            ),
            Action::ToggleLineBlame
        );
    }
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
fn ctrl_left_press_is_a_ctrl_click_and_other_buttons_are_unchanged() {
    let ctrl = KeyModifiers::CONTROL;
    let left = MouseEventKind::Down(MouseButton::Left);
    assert_eq!(
        event_to_action(&mouse_mods(left, 12, 3, ctrl), normal(), false, false),
        Action::CtrlClick { col: 12, row: 3 }
    );
    assert_eq!(
        event_to_action(&mouse(left, 12, 3), normal(), false, false),
        Action::Click { col: 12, row: 3 }
    );
    assert_eq!(
        event_to_action(
            &mouse_mods(MouseEventKind::Drag(MouseButton::Left), 12, 3, ctrl),
            normal(),
            false,
            false
        ),
        Action::Drag { col: 12, row: 3 }
    );
    assert_eq!(
        event_to_action(
            &mouse_mods(MouseEventKind::Up(MouseButton::Left), 12, 3, ctrl),
            normal(),
            false,
            false
        ),
        Action::Release
    );
    assert_eq!(
        event_to_action(
            &mouse_mods(MouseEventKind::Down(MouseButton::Right), 12, 3, ctrl),
            normal(),
            false,
            false
        ),
        Action::BackClick
    );
    assert_eq!(
        event_to_action(
            &mouse_mods(MouseEventKind::ScrollDown, 12, 3, ctrl),
            normal(),
            false,
            false
        ),
        Action::ScrollWheel {
            col: 12,
            row: 3,
            delta: 1,
            horizontal: false,
        }
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
    let right_motion = decode_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT_MOTION, 8, 4)).unwrap();
    assert_eq!(
        event_to_action(&right_motion, normal(), false, false),
        Action::ScrollWheel {
            col: 8,
            row: 4,
            delta: 1,
            horizontal: true,
        },
        "SGR 99 (wheel right + any-event motion bit) pans like 67"
    );
    let down_motion = decode_sgr_mouse(&sgr_mouse_report(65 | 32, 8, 4)).unwrap();
    assert_eq!(
        event_to_action(&down_motion, normal(), false, false),
        Action::ScrollWheel {
            col: 8,
            row: 4,
            delta: 1,
            horizontal: false,
        },
        "SGR 97 (wheel down + motion bit) scrolls like 65"
    );
}

#[test]
fn buttonless_motion_maps_to_pointer_move_outside_overlays() {
    use crate::tui::tty::{decode_sgr_mouse, sgr_mouse_report, SGR_POINTER_MOVE};
    let moved = decode_sgr_mouse(&sgr_mouse_report(SGR_POINTER_MOVE, 30, 0)).unwrap();
    assert_eq!(
        event_to_action(&moved, normal(), false, false),
        Action::PointerMove { col: 30, row: 0 }
    );
    assert_eq!(
        event_to_action(&moved, InputMode::QuickOpen, false, false),
        Action::None
    );
}

#[test]
fn highlight_keeps_global_keys_and_moves_the_head() {
    let visual = InputMode::DiffVisual;
    for (event, want) in [
        (key(KeyCode::Char('?')), Action::ToggleHelp),
        (key(KeyCode::Char('q')), Action::Quit),
        (key(KeyCode::Char('m')), Action::ToggleMouse),
        (key(KeyCode::Char('T')), Action::DiffVisualUnmapped),
        (key(KeyCode::Char('g')), Action::ArmGChord),
        (key(KeyCode::Char('G')), Action::MoveToEnd),
        (key(KeyCode::Home), Action::MoveToStart),
        (key(KeyCode::End), Action::MoveToEnd),
        (key(KeyCode::PageUp), Action::PageMove(-1)),
        (key(KeyCode::PageDown), Action::PageMove(1)),
        (ctrl(KeyCode::Char('u')), Action::Move(-5)),
        (ctrl(KeyCode::Char('d')), Action::Move(5)),
        (key(KeyCode::Char('j')), Action::Move(1)),
        (key(KeyCode::Esc), Action::DiffVisualCancel),
    ] {
        assert_eq!(
            event_to_action(&event, visual, true, true),
            want,
            "{event:?}"
        );
    }
}

#[test]
fn highlight_unmapped_keys_say_how_to_leave() {
    let visual = InputMode::DiffVisual;
    for event in [
        key(KeyCode::Char('f')),
        key(KeyCode::Char('z')),
        key(KeyCode::Enter),
        key(KeyCode::Tab),
        ctrl(KeyCode::Char('o')),
    ] {
        assert_eq!(
            event_to_action(&event, visual, true, true),
            Action::DiffVisualUnmapped,
            "{event:?}"
        );
    }
    // A held unmapped key does not repeat the warning.
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('f'), KeyEventKind::Repeat),
            visual,
            true,
            true
        ),
        Action::None
    );
}

#[test]
fn highlight_g_press_records_the_chord_echo() {
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    assert!(!drop_protocol_dup_g_chord_press(
        &mut echo,
        InputMode::DiffVisual,
        &g
    ));
    // The CSI-u echo of that same tap is dropped, not a second `g`.
    assert!(drop_protocol_dup_g_chord_press(
        &mut echo,
        InputMode::DiffVisual,
        &g
    ));
}

fn shift(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::SHIFT))
}

#[test]
fn theme_and_reviewed_keys() {
    // Themes are palette rows only: bare `T` and CSI-u Shift+T are unbound.
    for event in [
        key(KeyCode::Char('T')),
        shift(KeyCode::Char('t')),
        shift(KeyCode::Char('T')),
    ] {
        assert_eq!(
            event_to_action(&event, normal(), false, false),
            Action::None,
            "{event:?}"
        );
    }
    let pending = InputMode::GPending {
        search_active: false,
    };
    assert_eq!(
        event_to_action(&key(KeyCode::Char('T')), pending, false, false),
        Action::PreviousTab,
        "gT still goes to the previous tab"
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
    // Kitty CSI-u Left/Right (`57350`/`57351`): crossterm 0.28 yields Char PUA.
    // 57350 is U+E006, not U+E00E (that is CapsLock 57358).
    let kitty_left = char::from_u32(57350).unwrap();
    let kitty_right = char::from_u32(57351).unwrap();
    assert_eq!(
        event_to_action_with(
            &shift(KeyCode::Char(kitty_left)),
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
            &shift(KeyCode::Char(kitty_right)),
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
            &key(KeyCode::Char(kitty_left)),
            normal(),
            false,
            false,
            false,
            false,
            true
        ),
        Action::FoldClose
    );
    assert_eq!(
        event_to_action_with(
            &key(KeyCode::Char(kitty_right)),
            normal(),
            false,
            false,
            false,
            false,
            true
        ),
        Action::FoldOpen
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
fn help_j_k_pgup_pgdn_scroll() {
    let help = InputMode::Help;
    for (event, action) in [
        (key(KeyCode::Char('j')), Action::HelpScroll(1)),
        (key(KeyCode::Down), Action::HelpScroll(1)),
        (key(KeyCode::Char('k')), Action::HelpScroll(-1)),
        (key(KeyCode::Up), Action::HelpScroll(-1)),
        (key(KeyCode::PageDown), Action::HelpScroll(HELP_PAGE_ROWS)),
        (key(KeyCode::PageUp), Action::HelpScroll(-HELP_PAGE_ROWS)),
    ] {
        assert_eq!(event_to_action(&event, help, false, false), action);
    }
    let searching = InputMode::HelpSearch;
    assert_eq!(
        event_to_action(&key(KeyCode::Char('j')), searching, false, false),
        Action::SearchChar('j')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('k')), searching, false, false),
        Action::SearchChar('k')
    );
    for (event, action) in [
        (key(KeyCode::Down), Action::HelpScroll(1)),
        (key(KeyCode::Up), Action::HelpScroll(-1)),
        (key(KeyCode::PageDown), Action::HelpScroll(HELP_PAGE_ROWS)),
        (key(KeyCode::PageUp), Action::HelpScroll(-HELP_PAGE_ROWS)),
    ] {
        assert_eq!(event_to_action(&event, searching, false, false), action);
    }
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
        Action::Move(5)
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('u')), normal(), true, true),
        Action::Move(-5)
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
fn half_page_chord_detects_ctrl_d_and_ctrl_u_only() {
    assert!(is_half_page_chord(&ctrl(KeyCode::Char('d'))));
    assert!(is_half_page_chord(&ctrl(KeyCode::Char('u'))));
    let repeat = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('d'),
        KeyModifiers::CONTROL,
        KeyEventKind::Repeat,
    ));
    assert!(is_half_page_chord(&repeat), "a held Ctrl-d keeps gliding");
    let release = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('d'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    ));
    assert!(!is_half_page_chord(&release), "a release is not a press");
    for other in [
        key(KeyCode::Char('d')),
        key(KeyCode::Char('u')),
        ctrl(KeyCode::Char('o')),
        ctrl(KeyCode::Char('w')),
        key(KeyCode::PageDown),
        Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        )),
        Event::Resize(80, 24),
    ] {
        assert!(!is_half_page_chord(&other), "{other:?}");
    }
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
fn g_chord_tabs_and_gg_stay_distinct() {
    let pending = InputMode::GPending {
        search_active: false,
    };
    assert_eq!(
        event_to_action(&key(KeyCode::Char('t')), pending, false, false),
        Action::NextTab
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('t')), pending, false, false),
        Action::PreviousTab
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('1')), pending, false, false),
        Action::JumpToTab(1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('9')), pending, false, false),
        Action::JumpToTab(9)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Esc), pending, false, false),
        Action::GChordCancel
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('%')), pending, false, false),
        Action::GChordCancel,
        "a key with no action ends the chord"
    );
    assert_eq!(
        event_to_action(
            &key(KeyCode::Modifier(
                crossterm::event::ModifierKeyCode::LeftShift
            )),
            pending,
            false,
            false
        ),
        Action::None,
        "a bare Shift press before `T` keeps the chord"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('g')), pending, false, false),
        Action::MoveToStart
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('x')), pending, false, false),
        Action::OpenPullRequest
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('w')), pending, false, false),
        Action::ViewRemotely
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('c')), pending, false, false),
        Action::OpenPullRequestCi
    );
    assert_eq!(
        event_to_action_ex(&key(KeyCode::Char('c')), pending, false, true, false, true),
        Action::OpenPullRequestCi,
        "gc on a graph commit opens CI, not graph c (create branch)"
    );
    assert_eq!(
        event_to_action_ex(&key(KeyCode::Char('c')), normal(), false, true, false, true),
        Action::GraphCreateBranch,
        "bare c on a graph commit still creates a branch"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('x')), normal(), false, false),
        Action::Revert,
        "bare x stays revert"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('t')), normal(), false, false),
        Action::ToggleTreeMode
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('T')), normal(), false, false),
        Action::None,
        "bare T sets no theme; gT is the only T binding"
    );
}

#[test]
fn typeless_csi_u_echo_of_g_chord_key_is_dropped() {
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    let g_release = key_kind(KeyCode::Char('g'), KeyEventKind::Release);
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, normal(), &g),
        "first g arms the chord"
    );
    assert!(
        drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &g),
        "typeless g release must not complete gg"
    );
    let t = key(KeyCode::Char('t'));
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &t),
        "gt still maps"
    );
    assert!(
        drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &t),
        "typeless t release must not fire ToggleTreeMode"
    );
    let home = key(KeyCode::Home);
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &home),
        "Home is not a g-chord echo"
    );
    echo = GChordEchoState::default();
    assert!(!drop_protocol_dup_g_chord_press(&mut echo, normal(), &g));
    assert!(
        drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &g_release),
        "typed Release clears the echo"
    );
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &g),
        "human gg after a real Release still completes"
    );
    echo = GChordEchoState::default();
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, InputMode::SearchPrompt, &g),
        "typed g in search is not a chord press"
    );
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, normal(), &g),
        "g after a search query still arms GPending"
    );
}

#[test]
fn raw_byte_second_g_is_not_a_protocol_echo() {
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    assert!(!drop_g_chord_echo(
        &mut echo,
        normal(),
        &g,
        KeyStrokeOrigin::LegacyByte
    ));
    assert!(
        !drop_g_chord_echo(&mut echo, pending_g(), &g, KeyStrokeOrigin::LegacyByte),
        "two raw g bytes complete gg"
    );
}

#[test]
fn typeless_second_g_after_echo_window_is_a_new_tap() {
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    let t0 = Instant::now();
    assert!(!drop_g_chord_echo_at(
        &mut echo,
        normal(),
        &g,
        KeyStrokeOrigin::Protocol,
        t0
    ));
    let later = t0 + Duration::from_millis(G_CHORD_PROTOCOL_ECHO_MS + 1);
    assert!(
        !drop_g_chord_echo_at(&mut echo, pending_g(), &g, KeyStrokeOrigin::Protocol, later),
        "one typeless CSI-u per tap must complete gg after the echo window"
    );
}

#[test]
fn typeless_second_g_at_40ms_is_a_new_tap() {
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    let t0 = Instant::now();
    assert!(!drop_g_chord_echo_at(
        &mut echo,
        normal(),
        &g,
        KeyStrokeOrigin::Protocol,
        t0
    ));
    let later = t0 + Duration::from_millis(40);
    assert!(
        !drop_g_chord_echo_at(&mut echo, pending_g(), &g, KeyStrokeOrigin::Protocol, later),
        "40ms is inside DOUBLE_TAP_MS and under the old 80ms echo; completing g must stay"
    );
}

#[test]
fn completing_g_does_not_eat_next_g_after_burst() {
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    let t0 = Instant::now();
    assert!(!drop_g_chord_echo_at(
        &mut echo,
        normal(),
        &g,
        KeyStrokeOrigin::Protocol,
        t0
    ));
    let tap2 = t0 + Duration::from_millis(40);
    assert!(!drop_g_chord_echo_at(
        &mut echo,
        pending_g(),
        &g,
        KeyStrokeOrigin::Protocol,
        tap2
    ));
    let after_burst = tap2 + Duration::from_millis(G_CHORD_PROTOCOL_ECHO_MS + 1);
    assert!(
        !drop_g_chord_echo_at(
            &mut echo,
            normal(),
            &g,
            KeyStrokeOrigin::Protocol,
            after_burst
        ),
        "completing g must not eat the next g of gt after the VTE burst"
    );
}

#[test]
fn expired_arming_g_does_not_drop_the_next_protocol_g() {
    let mut echo = GChordEchoState::default();
    let mut pending = Some(Instant::now());
    let g = key(KeyCode::Char('g'));
    assert!(!drop_protocol_dup_g_chord_press(&mut echo, normal(), &g));
    expire_stale_g_chord_echo(&mut echo, &mut pending, Instant::now());
    assert!(pending.is_some(), "fresh pending must stay");
    pending = Some(crate::testutil::instant_ago(Duration::from_millis(
        DOUBLE_TAP_MS + 1,
    )));
    expire_stale_g_chord_echo(&mut echo, &mut pending, Instant::now());
    assert!(pending.is_none());
    assert!(
        !drop_protocol_dup_g_chord_press(&mut echo, normal(), &g),
        "after expiry the next g arms again"
    );
}

#[test]
fn expire_does_not_clear_t_echo_after_next_tab() {
    let mut echo = GChordEchoState::default();
    // Nothing reads `pending` before the expire call, so it starts clear.
    let mut pending = None;
    let g = key(KeyCode::Char('g'));
    let t = key(KeyCode::Char('t'));
    assert!(!drop_protocol_dup_g_chord_press(&mut echo, normal(), &g));
    assert!(!drop_protocol_dup_g_chord_press(&mut echo, pending_g(), &t));
    expire_stale_g_chord_echo(&mut echo, &mut pending, Instant::now());
    assert!(
        drop_protocol_dup_g_chord_press(&mut echo, normal(), &t),
        "typeless t release after NextTab must still drop"
    );
}

/// `A` opens the blame menu from every Normal context (tree, graph
/// commit or stash row, diff, either pane) and through pending `z` /
/// `g`. Inside the menu its letters, Enter, Esc, and `q` stay menu keys,
/// so nothing moves the focused line; mouse input does nothing.
#[test]
fn a_opens_the_blame_menu_and_the_menu_owns_its_keys() {
    let a = key(KeyCode::Char('A'));
    let pending_z = InputMode::ZPending {
        search_active: false,
    };
    for mode in [normal(), pending_z, pending_g()] {
        for (right_is_diff, focus_right) in [(false, false), (false, true), (true, true)] {
            for (stash, commit) in [(false, false), (true, false), (false, true)] {
                assert_eq!(
                    event_to_action_ex(&a, mode, right_is_diff, focus_right, stash, commit),
                    Action::BlameMenu,
                    "{mode:?} diff={right_is_diff} right={focus_right} {stash} {commit}"
                );
            }
        }
    }
    assert_eq!(
        event_to_action(&key(KeyCode::Char('w')), normal(), true, true),
        Action::None,
        "w stays unbound outside the menu"
    );
    let menu = InputMode::BlameMenu;
    for c in ['c', 'p', 'w', 'g', 'j', 'k', ':'] {
        assert_eq!(
            event_to_action(&key(KeyCode::Char(c)), menu, true, true),
            Action::BlameMenuChar(c)
        );
    }
    for code in [KeyCode::Esc, KeyCode::Char('q')] {
        assert_eq!(
            event_to_action(&key(code), menu, true, true),
            Action::BlameMenuCancel,
            "{code:?}"
        );
    }
    assert_eq!(
        event_to_action(&key(KeyCode::Enter), menu, true, true),
        Action::BlameMenuEnter
    );
    for code in [KeyCode::Down, KeyCode::Up, KeyCode::PageDown, KeyCode::Tab] {
        assert_eq!(
            event_to_action(&key(code), menu, true, true),
            Action::None,
            "{code:?}"
        );
    }
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('k')), menu, true, true),
        Action::None,
        "no Quick Open over the menu"
    );
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::ScrollDown,
    ] {
        assert_eq!(
            event_to_action(&mouse(kind, 40, 4), menu, true, true),
            Action::None
        );
    }
}

fn pending_g() -> InputMode {
    InputMode::GPending {
        search_active: false,
    }
}

#[test]
fn gh_pins_the_icon_popover_and_bare_h_still_folds() {
    assert_eq!(
        event_to_action(&key(KeyCode::Char('h')), pending_g(), false, false),
        Action::PopoverOpenFocused
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('h')), normal(), false, false),
        Action::None
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('h')), pending_g(), false, false),
        Action::GChordCancel,
        "an unbound modifier chord falls through and ends the chord"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('h')), normal(), false, false),
        Action::FoldClose,
        "bare h folds the tree"
    );
}

#[test]
fn popover_mode_keys() {
    let at = |event: Event| event_to_action(&event, InputMode::Popover, false, false);
    for (event, want) in [
        (key(KeyCode::Char('j')), Action::PopoverMove(1)),
        (shift(KeyCode::Char('J')), Action::PopoverMove(1)),
        (key(KeyCode::Down), Action::PopoverMove(1)),
        (key(KeyCode::Char('k')), Action::PopoverMove(-1)),
        (shift(KeyCode::Char('K')), Action::PopoverMove(-1)),
        (key(KeyCode::Up), Action::PopoverMove(-1)),
        (key(KeyCode::Enter), Action::PopoverRun),
        (key(KeyCode::Char('y')), Action::PopoverCopyLine),
        (key(KeyCode::Char('r')), Action::RefreshPullRequest),
        (key(KeyCode::Char('g')), Action::ArmGChord),
        (key(KeyCode::Char('c')), Action::None),
        (key(KeyCode::Char('x')), Action::None),
        (key(KeyCode::Esc), Action::PopoverClose),
        (key(KeyCode::Char('p')), Action::None),
        (key(KeyCode::Char('q')), Action::None),
        (key(KeyCode::Char(':')), Action::None),
        (ctrl(KeyCode::Char('j')), Action::None),
        (ctrl(KeyCode::Char('p')), Action::None),
        (ctrl(KeyCode::Char('g')), Action::None),
        (ctrl(KeyCode::Char('c')), Action::CtrlC),
    ] {
        assert_eq!(at(event.clone()), want, "{event:?}");
    }
}

/// `g` in a pinned popover: `x` and `c` complete `gx` / `gc`; any
/// other key, Esc, and a right click only end the chord.
#[test]
fn popover_g_chord_keys() {
    let mode = InputMode::PopoverGPending;
    let at = |event: Event| event_to_action(&event, mode, false, false);
    for (event, want) in [
        (key(KeyCode::Char('x')), Action::OpenPullRequest),
        (key(KeyCode::Char('c')), Action::OpenPullRequestCi),
        (key(KeyCode::Esc), Action::GChordCancel),
        (key(KeyCode::Char('j')), Action::GChordCancel),
        (key(KeyCode::Enter), Action::GChordCancel),
        (key(KeyCode::Char('y')), Action::GChordCancel),
        (key(KeyCode::Char('g')), Action::GChordCancel),
        (key(KeyCode::Char('w')), Action::GChordCancel),
        (ctrl(KeyCode::Char('x')), Action::GChordCancel),
        (ctrl(KeyCode::Char('c')), Action::CtrlC),
        (
            mouse(MouseEventKind::Down(MouseButton::Right), 4, 4),
            Action::GChordCancel,
        ),
    ] {
        assert_eq!(at(event.clone()), want, "{event:?}");
    }
    // The arming `g` and the second key both feed the echo guard.
    let mut echo = GChordEchoState::default();
    let g = key(KeyCode::Char('g'));
    assert!(!drop_protocol_dup_g_chord_press(
        &mut echo,
        InputMode::Popover,
        &g
    ));
    assert!(
        drop_protocol_dup_g_chord_press(&mut echo, mode, &g),
        "a typeless echo of the arming g is dropped"
    );
}

#[test]
fn doctor_mode_keys_move_copy_rerun_close_and_swallow_the_rest() {
    let at = |event: Event| event_to_action(&event, InputMode::Doctor, false, false);
    let page = DOCTOR_PAGE_ROWS;
    for (event, want) in [
        (key(KeyCode::Char('j')), Action::DoctorMove(1)),
        (key(KeyCode::Down), Action::DoctorMove(1)),
        (key(KeyCode::Char('k')), Action::DoctorMove(-1)),
        (key(KeyCode::Up), Action::DoctorMove(-1)),
        (key(KeyCode::PageDown), Action::DoctorMove(page)),
        (key(KeyCode::PageUp), Action::DoctorMove(-page)),
        (ctrl(KeyCode::Char('d')), Action::DoctorMove(page)),
        (ctrl(KeyCode::Char('u')), Action::DoctorMove(-page)),
        (key(KeyCode::Char('g')), Action::DoctorTop),
        (shift(KeyCode::Char('G')), Action::DoctorBottom),
        (key(KeyCode::Char('y')), Action::DoctorCopy),
        (key(KeyCode::Char('r')), Action::DoctorRerun),
        (key(KeyCode::Esc), Action::DoctorClose),
        (key(KeyCode::Char('q')), Action::DoctorClose),
        // Swallowed: no pane key, opener, or chord leaks through.
        (key(KeyCode::Enter), Action::None),
        (key(KeyCode::Char('m')), Action::None),
        (key(KeyCode::Char('?')), Action::None),
        (key(KeyCode::Char(':')), Action::None),
        (shift(KeyCode::Char('F')), Action::None),
        (ctrl(KeyCode::Char('p')), Action::None),
        (ctrl(KeyCode::Char('f')), Action::None),
        (ctrl(KeyCode::Char('y')), Action::None),
        (ctrl(KeyCode::Char('c')), Action::CtrlC),
    ] {
        assert_eq!(at(event.clone()), want, "{event:?}");
    }
    let wheel = mouse(MouseEventKind::ScrollDown, 40, 4);
    assert_eq!(
        event_to_action(&wheel, InputMode::Doctor, false, false),
        Action::None
    );
}

#[test]
fn popover_mode_keeps_the_mouse_and_right_click_closes() {
    let at = |kind| event_to_action(&mouse(kind, 40, 4), InputMode::Popover, false, false);
    assert_eq!(
        at(MouseEventKind::Down(MouseButton::Left)),
        Action::Click { col: 40, row: 4 }
    );
    assert_eq!(
        at(MouseEventKind::Drag(MouseButton::Left)),
        Action::Drag { col: 40, row: 4 }
    );
    assert_eq!(at(MouseEventKind::Up(MouseButton::Left)), Action::Release);
    assert_eq!(
        at(MouseEventKind::Moved),
        Action::PointerMove { col: 40, row: 4 }
    );
    assert_eq!(
        at(MouseEventKind::Down(MouseButton::Right)),
        Action::PopoverClose
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
    assert_eq!(
        event_to_action(&key(KeyCode::Char('G')), normal(), true, true),
        Action::MoveToEnd
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('g')), normal(), true, true),
        Action::ArmGChord
    );
    let pending = InputMode::GPending {
        search_active: false,
    };
    assert_eq!(
        event_to_action(&key(KeyCode::Char('g')), pending, true, true),
        Action::MoveToStart
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Home), normal(), true, true),
        Action::MoveToStart
    );
    assert_eq!(
        event_to_action(&key(KeyCode::End), normal(), true, true),
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
        Action::Move(1)
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
        Action::BranchChar('j')
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('C'), KeyEventKind::Repeat),
            InputMode::BranchPicker,
            false,
            false
        ),
        Action::BranchChar('C')
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Down, KeyEventKind::Repeat),
            InputMode::BranchPicker,
            false,
            false
        ),
        Action::BranchMove(1)
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
    assert!(held_nav_key(&shift(KeyCode::Char(char::from_u32(57350).unwrap()))).is_some());
    assert!(held_nav_key(&key(KeyCode::Char('q'))).is_none());
    assert!(held_nav_key(&key_kind(KeyCode::Char('j'), KeyEventKind::Release)).is_none());
}

fn quick_open() -> InputMode {
    InputMode::QuickOpen
}

#[test]
fn colon_opens_quick_open_commands_and_ctrl_p_or_shift_f_files() {
    use super::super::action::QuickOpenEntry;
    let openers = [
        normal(),
        InputMode::ZPending {
            search_active: false,
        },
        InputMode::GPending {
            search_active: false,
        },
        InputMode::DiffVisual,
    ];
    let files = Action::ToggleQuickOpen(QuickOpenEntry::Files);
    for mode in openers {
        assert_eq!(
            event_to_action(&key(KeyCode::Char(':')), mode, true, true),
            Action::ToggleQuickOpen(QuickOpenEntry::Commands),
            "{mode:?} `:`"
        );
        for event in [
            ctrl(KeyCode::Char('p')),
            key(KeyCode::Char('F')),
            shift(KeyCode::Char('F')),
            shift(KeyCode::Char('f')),
        ] {
            assert_eq!(
                event_to_action(&event, mode, true, true),
                files,
                "{mode:?} {event:?}"
            );
        }
        let ctrl_k = event_to_action(&ctrl(KeyCode::Char('k')), mode, true, true);
        assert!(
            !matches!(ctrl_k, Action::ToggleQuickOpen(_)),
            "{mode:?} Ctrl-k no longer opens Quick Open, got {ctrl_k:?}"
        );
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        for (c, mods) in [
            ('F', ctrl_shift),
            ('f', ctrl_shift),
            ('F', KeyModifiers::ALT),
            ('f', KeyModifiers::ALT),
            ('P', ctrl_shift),
            ('p', ctrl_shift),
        ] {
            let chord = Event::Key(KeyEvent::new(KeyCode::Char(c), mods));
            let got = event_to_action(&chord, mode, true, true);
            assert!(
                !matches!(got, Action::ToggleQuickOpen(_)),
                "{mode:?} {c} {mods:?} must not open Quick Open, got {got:?}"
            );
        }
    }
    // Lowercase `f` / `p` and Shift `P` keep their remote bindings.
    assert_eq!(
        event_to_action(&key(KeyCode::Char('f')), normal(), false, false),
        Action::Fetch
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('p')), normal(), false, false),
        Action::Pull
    );
    assert_eq!(
        event_to_action(&shift(KeyCode::Char('P')), normal(), false, false),
        Action::Push
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('k')), normal(), false, false),
        Action::None
    );
    assert_eq!(
        event_to_action(
            &ctrl(KeyCode::Char('k')),
            InputMode::DiffVisual,
            false,
            false
        ),
        Action::DiffVisualUnmapped
    );
    for c in [':', '>', '<', 'F'] {
        assert_eq!(
            event_to_action(&key(KeyCode::Char(c)), quick_open(), false, false),
            Action::QuickOpenChar(c),
            "`{c}` types inside Quick Open"
        );
    }
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('k')), quick_open(), false, false),
        Action::QuickOpenMove(-1)
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('p')), quick_open(), false, false),
        Action::QuickOpenMove(-1)
    );
}

#[test]
fn overlays_do_not_open_quick_open_on_colon_ctrl_p_or_shift_f() {
    let overlays = [
        InputMode::Help,
        InputMode::HelpSearch,
        InputMode::SearchPrompt,
        InputMode::Comment,
        InputMode::Confirm,
        InputMode::BranchPicker,
        InputMode::ComparePicker,
        InputMode::GraphFocusPicker,
        InputMode::CreateBranch,
        InputMode::StashMenu,
        InputMode::BlameMenu,
        InputMode::CommentExport,
        InputMode::QuickOpen,
    ];
    for mode in overlays {
        for event in [
            key(KeyCode::Char(':')),
            ctrl(KeyCode::Char('p')),
            ctrl(KeyCode::Char('k')),
            key(KeyCode::Char('F')),
            shift(KeyCode::Char('F')),
        ] {
            let got = event_to_action(&event, mode, false, false);
            assert!(
                !matches!(got, Action::ToggleQuickOpen(_)),
                "{mode:?} {event:?} must not open Quick Open, got {got:?}"
            );
        }
    }
}

#[test]
fn text_overlays_type_colon_and_shift_f() {
    for c in [':', 'F'] {
        let typed = key(KeyCode::Char(c));
        let at = |mode| event_to_action(&typed, mode, false, false);
        assert_eq!(at(InputMode::SearchPrompt), Action::SearchChar(c));
        assert_eq!(at(InputMode::HelpSearch), Action::SearchChar(c));
        assert_eq!(at(InputMode::CreateBranch), Action::CreateBranchChar(c));
        assert_eq!(at(InputMode::BranchPicker), Action::BranchChar(c));
        assert_eq!(at(InputMode::ComparePicker), Action::ComparePickerChar(c));
        assert_eq!(at(InputMode::GraphFocusPicker), Action::GraphFocusChar(c));
        assert_eq!(at(InputMode::StashMenu), Action::StashMenuChar(c));
        assert_eq!(at(InputMode::BlameMenu), Action::BlameMenuChar(c));
        assert_eq!(at(InputMode::QuickOpen), Action::QuickOpenChar(c));
        match at(InputMode::Comment) {
            Action::CommentInput(key) => assert_eq!(key.code, KeyCode::Char(c)),
            other => panic!("Comment `{c}` must be CommentInput, got {other:?}"),
        }
    }
}

#[test]
fn comment_ctrl_k_is_textarea_input() {
    match event_to_action(&ctrl(KeyCode::Char('k')), InputMode::Comment, false, false) {
        Action::CommentInput(key) => {
            assert_eq!(key.code, KeyCode::Char('k'));
            assert!(key.modifiers.contains(KeyModifiers::CONTROL));
        }
        other => panic!("Comment Ctrl-K must be CommentInput, got {other:?}"),
    }
}

#[test]
fn help_confirm_comment_export_swallow_quick_open_keys() {
    for mode in [
        InputMode::Help,
        InputMode::Confirm,
        InputMode::CommentExport,
    ] {
        for event in [
            ctrl(KeyCode::Char('k')),
            ctrl(KeyCode::Char('p')),
            key(KeyCode::Char(':')),
            key(KeyCode::Char('F')),
            shift(KeyCode::Char('F')),
        ] {
            assert_eq!(
                event_to_action(&event, mode, false, false),
                Action::None,
                "{mode:?} {event:?}"
            );
        }
    }
    assert_eq!(
        event_to_action(
            &ctrl(KeyCode::Char('k')),
            InputMode::SearchPrompt,
            false,
            false
        ),
        Action::None,
        "search Ctrl-K stays unused (CONTROL is not a typed char)"
    );
}

#[test]
fn quick_open_keys_move_filter_submit_and_only_esc_cancels() {
    assert_eq!(
        event_to_action(&key(KeyCode::Char('j')), quick_open(), false, false),
        Action::QuickOpenChar('j')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Down), quick_open(), false, false),
        Action::QuickOpenMove(1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('k')), quick_open(), false, false),
        Action::QuickOpenChar('k')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Up), quick_open(), false, false),
        Action::QuickOpenMove(-1)
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('p')), quick_open(), false, false),
        Action::QuickOpenChar('p')
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Backspace), quick_open(), false, false),
        Action::QuickOpenBackspace
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Enter), quick_open(), false, false),
        Action::QuickOpenSubmit
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Esc), quick_open(), false, false),
        Action::QuickOpenEscape
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('c')), quick_open(), false, false),
        Action::CtrlC
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Char('k')), quick_open(), false, false),
        Action::QuickOpenMove(-1),
        "Ctrl-k moves up like every picker; it does not close"
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char(':')), quick_open(), false, false),
        Action::QuickOpenChar(':'),
        "`:` types into the filter; it does not close"
    );
}

fn search_files(results: bool) -> InputMode {
    InputMode::SearchFiles { results }
}

fn with_mods(c: char, mods: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), mods))
}

#[test]
fn ctrl_w_closes_the_tab_in_normal_keymaps_only() {
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    let normal_like = [
        normal(),
        InputMode::Normal {
            search_active: true,
        },
        InputMode::ZPending {
            search_active: false,
        },
        InputMode::GPending {
            search_active: false,
        },
    ];
    for mode in normal_like {
        for event in [
            ctrl(KeyCode::Char('w')),
            with_mods('w', ctrl_shift),
            with_mods('W', ctrl_shift),
            with_mods('W', KeyModifiers::CONTROL),
        ] {
            assert_eq!(
                event_to_action(&event, mode, true, true),
                Action::CloseTab,
                "{mode:?} {event:?}"
            );
        }
        // Alt / Super chords stay unbound; in a pending `g` they end it.
        let unbound = if matches!(mode, InputMode::GPending { .. }) {
            Action::GChordCancel
        } else {
            Action::None
        };
        for mods in [
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SUPER,
        ] {
            assert_eq!(
                event_to_action(&with_mods('w', mods), mode, true, true),
                unbound,
                "{mode:?} {mods:?}"
            );
        }
        // Plain `w` is unbound, except `gw` (view remotely).
        let plain_w = if matches!(mode, InputMode::GPending { .. }) {
            Action::ViewRemotely
        } else {
            Action::None
        };
        assert_eq!(
            event_to_action(&key(KeyCode::Char('w')), mode, true, true),
            plain_w,
            "{mode:?} plain w"
        );
    }
    // Plain `W` still removes a worktree.
    assert_eq!(
        event_to_action(&key(KeyCode::Char('W')), normal(), true, true),
        Action::RemoveWorktree
    );
    // Text input keeps Ctrl-w: the comment editor gets the key (delete
    // word), the filter prompts and Quick Open do not close the tab.
    let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(
        event_to_action(&Event::Key(ctrl_w), InputMode::Comment, true, true),
        Action::CommentInput(ctrl_w)
    );
    for mode in [
        InputMode::QuickOpen,
        InputMode::SearchPrompt,
        InputMode::HelpSearch,
        InputMode::CreateBranch,
    ] {
        assert_eq!(
            event_to_action(&Event::Key(ctrl_w), mode, true, true),
            Action::None,
            "{mode:?}"
        );
    }
}

#[test]
fn ctrl_f_opens_search_files_and_plain_f_stays_fetch() {
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    let openers = [
        normal(),
        InputMode::Normal {
            search_active: true,
        },
        InputMode::ZPending {
            search_active: false,
        },
        InputMode::GPending {
            search_active: false,
        },
        InputMode::DiffVisual,
    ];
    for mode in openers {
        for event in [
            ctrl(KeyCode::Char('f')),
            with_mods('f', ctrl_shift),
            with_mods('F', ctrl_shift),
        ] {
            assert_eq!(
                event_to_action(&event, mode, true, true),
                Action::ToggleSearchFiles,
                "{mode:?} {event:?}"
            );
        }
        for (c, mods) in [
            ('f', KeyModifiers::ALT),
            ('F', KeyModifiers::ALT),
            ('f', KeyModifiers::CONTROL | KeyModifiers::ALT),
            ('f', KeyModifiers::CONTROL | KeyModifiers::SUPER),
        ] {
            let got = event_to_action(&with_mods(c, mods), mode, true, true);
            assert_ne!(got, Action::ToggleSearchFiles, "{mode:?} {c} {mods:?}");
        }
    }
    assert_eq!(
        event_to_action(&key(KeyCode::Char('f')), normal(), false, false),
        Action::Fetch
    );
    assert_eq!(
        event_to_action(&key(KeyCode::Char('/')), normal(), false, false),
        Action::SearchStart
    );
    // A held Ctrl-f does not toggle the dialog shut again.
    let held = Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('f'),
        KeyModifiers::CONTROL,
        KeyEventKind::Repeat,
    ));
    assert_eq!(event_to_action(&held, normal(), false, false), Action::None);
}

#[test]
fn overlays_do_not_open_search_files() {
    let overlays = [
        InputMode::Help,
        InputMode::HelpSearch,
        InputMode::SearchPrompt,
        InputMode::Comment,
        InputMode::Confirm,
        InputMode::BranchPicker,
        InputMode::ComparePicker,
        InputMode::GraphFocusPicker,
        InputMode::CreateBranch,
        InputMode::StashMenu,
        InputMode::BlameMenu,
        InputMode::CommentExport,
        InputMode::QuickOpen,
        InputMode::Popover,
    ];
    for mode in overlays {
        let got = event_to_action(&ctrl(KeyCode::Char('f')), mode, false, false);
        assert_ne!(got, Action::ToggleSearchFiles, "{mode:?}");
    }
}

#[test]
fn search_files_query_zone_types_letters_and_moves_on_arrows() {
    let query = search_files(false);
    let at = |event: Event| event_to_action(&event, query, false, false);
    for c in ['f', 'e', 'n', 'j', 'k', 'N', ':', 'F', '#', '/', ' '] {
        assert_eq!(at(key(KeyCode::Char(c))), Action::SearchFilesChar(c), "{c}");
    }
    assert_eq!(at(shift(KeyCode::Char('Q'))), Action::SearchFilesChar('Q'));
    assert_eq!(at(key(KeyCode::Down)), Action::SearchFilesMove(1));
    assert_eq!(at(key(KeyCode::Up)), Action::SearchFilesMove(-1));
    for (c, delta) in [('n', 1), ('j', 1), ('p', -1), ('k', -1)] {
        assert_eq!(
            at(ctrl(KeyCode::Char(c))),
            Action::SearchFilesMove(delta),
            "Ctrl-{c}"
        );
    }
    assert_eq!(at(key(KeyCode::PageDown)), Action::SearchFilesPage(1));
    assert_eq!(at(key(KeyCode::PageUp)), Action::SearchFilesPage(-1));
    assert_eq!(at(key(KeyCode::Backspace)), Action::SearchFilesBackspace);
    assert_eq!(at(key(KeyCode::Tab)), Action::SearchFilesToggleScope);
    assert_eq!(at(key(KeyCode::Esc)), Action::SearchFilesEscape);
    assert_eq!(at(ctrl(KeyCode::Char('f'))), Action::ToggleSearchFiles);
    assert_eq!(at(ctrl(KeyCode::Char('c'))), Action::CtrlC);
    assert_eq!(at(key(KeyCode::Enter)), Action::SearchFilesSubmit);
    assert_eq!(at(with_mods('x', KeyModifiers::ALT)), Action::None);
    assert_eq!(at(ctrl(KeyCode::Char('x'))), Action::None);
}

#[test]
fn search_files_alt_c_w_r_do_nothing_in_either_zone() {
    // Alt chords can close the terminal window on some hosts, so no Alt
    // chord toggles an option (the chips take a click) or types.
    let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
    for results in [false, true] {
        for c in ['c', 'w', 'r', 'C', 'W', 'R'] {
            for mods in [KeyModifiers::ALT, alt_shift] {
                assert_eq!(
                    event_to_action(&with_mods(c, mods), search_files(results), false, false),
                    Action::None,
                    "{mods:?} {c} (results zone: {results})"
                );
            }
        }
    }
}

#[test]
fn search_files_results_zone_moves_on_j_k_and_types_the_rest() {
    let results = search_files(true);
    let at = |event: Event| event_to_action(&event, results, false, false);
    assert_eq!(at(key(KeyCode::Char('j'))), Action::SearchFilesMove(1));
    assert_eq!(at(key(KeyCode::Char('k'))), Action::SearchFilesMove(-1));
    assert_eq!(at(key(KeyCode::Down)), Action::SearchFilesMove(1));
    for c in ['x', 'n', 'f', 'J', 'K', 'E'] {
        assert_eq!(at(key(KeyCode::Char(c))), Action::SearchFilesChar(c), "{c}");
    }
    assert_eq!(at(key(KeyCode::Char('e'))), Action::SearchFilesEdit);
    assert_eq!(at(key(KeyCode::Enter)), Action::SearchFilesSubmit);
    assert_eq!(at(key(KeyCode::Backspace)), Action::SearchFilesBackspace);
    assert_eq!(at(key(KeyCode::Tab)), Action::SearchFilesToggleScope);
    assert_eq!(at(key(KeyCode::Esc)), Action::SearchFilesEscape);
}

#[test]
fn search_files_mouse_maps_left_click_and_motion_only() {
    use crate::tui::tty::{decode_sgr_mouse, sgr_mouse_report, SGR_POINTER_MOVE};
    let moved = decode_sgr_mouse(&sgr_mouse_report(SGR_POINTER_MOVE, 30, 0)).unwrap();
    let mouse = |kind: MouseEventKind, modifiers: KeyModifiers| {
        Event::Mouse(MouseEvent {
            kind,
            column: 30,
            row: 2,
            modifiers,
        })
    };
    for results in [false, true] {
        let at = |event: &Event| event_to_action(event, search_files(results), false, false);
        assert_eq!(at(&moved), Action::PointerMove { col: 30, row: 0 });
        assert_eq!(
            at(&mouse(
                MouseEventKind::Down(MouseButton::Left),
                KeyModifiers::NONE
            )),
            Action::Click { col: 30, row: 2 }
        );
        for (kind, modifiers) in [
            (
                MouseEventKind::Down(MouseButton::Left),
                KeyModifiers::CONTROL,
            ),
            (MouseEventKind::Down(MouseButton::Right), KeyModifiers::NONE),
            (
                MouseEventKind::Down(MouseButton::Middle),
                KeyModifiers::NONE,
            ),
            (MouseEventKind::Up(MouseButton::Left), KeyModifiers::NONE),
            (MouseEventKind::Drag(MouseButton::Left), KeyModifiers::NONE),
            (MouseEventKind::ScrollDown, KeyModifiers::NONE),
            (MouseEventKind::ScrollUp, KeyModifiers::SHIFT),
            (MouseEventKind::ScrollLeft, KeyModifiers::NONE),
        ] {
            assert_eq!(
                at(&mouse(kind, modifiers)),
                Action::None,
                "{kind:?} {modifiers:?}"
            );
        }
    }
    // Quick Open still drops every mouse event.
    assert_eq!(
        event_to_action(&moved, InputMode::QuickOpen, false, false),
        Action::None
    );
}

#[test]
fn search_files_repeats_typing_but_not_enter_or_e() {
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('a'), KeyEventKind::Repeat),
            search_files(false),
            false,
            false
        ),
        Action::SearchFilesChar('a')
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Enter, KeyEventKind::Repeat),
            search_files(true),
            false,
            false
        ),
        Action::None
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('e'), KeyEventKind::Repeat),
            search_files(true),
            false,
            false
        ),
        Action::None,
        "a held `e` opens the editor once"
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('e'), KeyEventKind::Repeat),
            search_files(false),
            false,
            false
        ),
        Action::SearchFilesChar('e')
    );
}

#[test]
fn quick_open_repeat_types_and_moves_but_not_enter_or_esc() {
    use super::super::action::QuickOpenEntry;
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('a'), KeyEventKind::Repeat),
            quick_open(),
            false,
            false
        ),
        Action::QuickOpenChar('a')
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Backspace, KeyEventKind::Repeat),
            quick_open(),
            false,
            false
        ),
        Action::QuickOpenBackspace
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char('j'), KeyEventKind::Repeat),
            quick_open(),
            false,
            false
        ),
        Action::QuickOpenChar('j')
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Enter, KeyEventKind::Repeat),
            quick_open(),
            false,
            false
        ),
        Action::None
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Esc, KeyEventKind::Repeat),
            quick_open(),
            false,
            false
        ),
        Action::None
    );
    assert_eq!(
        event_to_action(
            &key_kind(KeyCode::Char(':'), KeyEventKind::Repeat),
            quick_open(),
            false,
            false
        ),
        Action::QuickOpenChar(':')
    );
    assert_eq!(
        event_to_action(
            &Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
                KeyEventKind::Repeat
            )),
            quick_open(),
            false,
            false
        ),
        Action::QuickOpenMove(-1)
    );
    // A held opener does not toggle Quick Open shut again.
    assert_ne!(
        event_to_action(
            &Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL,
                KeyEventKind::Repeat
            )),
            normal(),
            false,
            false
        ),
        Action::ToggleQuickOpen(QuickOpenEntry::Files)
    );
}

#[test]
fn list_overlays_type_every_letter_and_move_on_arrows_and_ctrl() {
    let modes = [
        InputMode::QuickOpen,
        InputMode::BranchPicker,
        InputMode::ComparePicker,
        InputMode::GraphFocusPicker,
    ];
    let moved = |mode: InputMode, delta: i32| match mode {
        InputMode::QuickOpen => Action::QuickOpenMove(delta),
        InputMode::BranchPicker => Action::BranchMove(delta),
        InputMode::ComparePicker => Action::ComparePickerMove(delta),
        _ => Action::GraphFocusMove(delta),
    };
    let typed = |mode: InputMode, c: char| match mode {
        InputMode::QuickOpen => Action::QuickOpenChar(c),
        InputMode::BranchPicker => Action::BranchChar(c),
        InputMode::ComparePicker => Action::ComparePickerChar(c),
        _ => Action::GraphFocusChar(c),
    };
    for mode in modes {
        for c in ['j', 'k', 'J', 'K', 'C', 'O', 'n', 'p'] {
            assert_eq!(
                event_to_action(&key(KeyCode::Char(c)), mode, false, false),
                typed(mode, c),
                "{mode:?} {c}"
            );
        }
        for (event, delta) in [
            (key(KeyCode::Down), 1),
            (key(KeyCode::Up), -1),
            (ctrl(KeyCode::Char('n')), 1),
            (ctrl(KeyCode::Char('p')), -1),
            (ctrl(KeyCode::Char('j')), 1),
        ] {
            assert_eq!(
                event_to_action(&event, mode, false, false),
                moved(mode, delta),
                "{mode:?} {event:?}"
            );
        }
        let ctrl_k = event_to_action(&ctrl(KeyCode::Char('k')), mode, false, false);
        assert_eq!(ctrl_k, moved(mode, -1), "{mode:?} Ctrl-k");
        assert_eq!(
            event_to_action(
                &Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('n'),
                    KeyModifiers::CONTROL,
                    KeyEventKind::Repeat
                )),
                mode,
                false,
                false
            ),
            moved(mode, 1),
            "{mode:?} held Ctrl-n repeats"
        );
    }
}

/// Every plain binding in Normal, the graph row keys, Help, Confirm,
/// pending `z` / `g`, and highlight, with and without a command modifier.
#[test]
fn unbound_modifier_chords_do_not_run_plain_bindings() {
    use KeyCode::{Char, Down, End, Enter, Home, Left, PageDown, Right, Tab, Up};
    const STASH: u8 = 1;
    const COMMIT: u8 = 2;
    let pending_z = InputMode::ZPending {
        search_active: false,
    };
    let n = normal();
    let rows: Vec<(InputMode, u8, KeyCode, Action)> = vec![
        (n, 0, Char('T'), Action::None),
        (n, 0, Char('t'), Action::ToggleTreeMode),
        (n, 0, Char('q'), Action::Quit),
        (n, 0, Char('?'), Action::ToggleHelp),
        (n, 0, Char('.'), Action::ToggleShowHidden),
        (
            n,
            0,
            Char(':'),
            Action::ToggleQuickOpen(QuickOpenEntry::Commands),
        ),
        (
            n,
            0,
            Char('F'),
            Action::ToggleQuickOpen(QuickOpenEntry::Files),
        ),
        (n, 0, Char('f'), Action::Fetch),
        (n, 0, Char('p'), Action::Pull),
        (n, 0, Char('d'), Action::DefaultBranch),
        (n, 0, Char('r'), Action::Refresh),
        (n, 0, Char(' '), Action::ToggleReviewed),
        (n, 0, Char('z'), Action::FoldToggle),
        (n, 0, Char('Z'), Action::ToggleFinishedPrGroups),
        (n, 0, Char('/'), Action::SearchStart),
        (n, 0, Char('s'), Action::Stage),
        (n, 0, Char('u'), Action::Unstage),
        (n, 0, Char('x'), Action::Revert),
        (n, 0, Char('e'), Action::Edit),
        (n, 0, Char('E'), Action::ExternalDiff),
        (n, 0, Char('P'), Action::Push),
        (n, 0, Char('S'), Action::StashMenu),
        (n, 0, Char('b'), Action::Branch),
        (n, 0, Char('o'), Action::GraphFocusBranches),
        (n, 0, Char('O'), Action::GraphFocusClear),
        (n, 0, Char('W'), Action::RemoveWorktree),
        (n, 0, Char('i'), Action::ToggleDiffMode),
        (n, 0, Char('<'), Action::ResizeTree(-1)),
        (n, 0, Char('>'), Action::ResizeTree(1)),
        (n, 0, Char('\\'), Action::ToggleDiffWrap),
        (n, 0, Char('M'), Action::ToggleCommitMsgExpand),
        (n, 0, Char('-'), Action::ExplorerReveal),
        (n, 0, Char('B'), Action::ToggleLineBlame),
        (n, 0, Char('A'), Action::BlameMenu),
        (n, 0, Char('m'), Action::ToggleMouse),
        (n, 0, Char(';'), Action::CommentStart),
        (n, 0, Char('V'), Action::DiffVisualStart),
        (n, 0, Char('y'), Action::ExportComments),
        (n, 0, Char('\''), Action::CopyEntityReference),
        (n, 0, Char('n'), Action::SearchNext),
        (n, 0, Char('N'), Action::SearchPrev),
        (n, 0, Char('G'), Action::MoveToEnd),
        (n, 0, Char('g'), Action::ArmGChord),
        (n, 0, Char('j'), Action::Move(1)),
        (n, 0, Char('J'), Action::Move(1)),
        (n, 0, Char('k'), Action::Move(-1)),
        (n, 0, Char('K'), Action::Move(-1)),
        (n, 0, Char('h'), Action::FoldClose),
        (n, 0, Char('H'), Action::FoldClose),
        (n, 0, Char('l'), Action::FoldOpen),
        (n, 0, Char('L'), Action::FoldOpen),
        (n, 0, Tab, Action::FocusRight),
        (n, 0, Enter, Action::NavEnter),
        (n, 0, Home, Action::MoveToStart),
        (n, 0, End, Action::MoveToEnd),
        (n, 0, PageDown, Action::PageMove(1)),
        (n, 0, Down, Action::Move(1)),
        (n, 0, Up, Action::Move(-1)),
        (n, 0, Left, Action::FoldClose),
        (n, 0, Right, Action::FoldOpen),
        (n, STASH, Char('a'), Action::GraphStashApply),
        (n, STASH, Char('p'), Action::GraphStashPop),
        (n, STASH, Char('D'), Action::GraphStashDrop),
        (n, COMMIT, Char('b'), Action::GraphCheckout),
        (n, COMMIT, Char('c'), Action::GraphCreateBranch),
        (n, COMMIT, Char('m'), Action::GraphMerge),
        (InputMode::Help, 0, Char('q'), Action::ToggleHelp),
        (InputMode::Help, 0, Char('Q'), Action::ToggleHelp),
        (InputMode::Help, 0, Char('?'), Action::ToggleHelp),
        (InputMode::Help, 0, Char('/'), Action::SearchStart),
        (InputMode::Confirm, 0, Char('y'), Action::ConfirmYes),
        (InputMode::Confirm, 0, Char('Y'), Action::ConfirmYesClean),
        (InputMode::Confirm, 0, Char('n'), Action::ConfirmNo),
        (InputMode::Confirm, 0, Char('N'), Action::ConfirmNo),
        (InputMode::Confirm, 0, Enter, Action::ConfirmEnter),
        (
            InputMode::StashMenu,
            0,
            Char('s'),
            Action::StashMenuChar('s'),
        ),
        (
            InputMode::StashMenu,
            0,
            Char('a'),
            Action::StashMenuChar('a'),
        ),
        (
            InputMode::StashMenu,
            0,
            Char('p'),
            Action::StashMenuChar('p'),
        ),
        (
            InputMode::StashMenu,
            0,
            Char('D'),
            Action::StashMenuChar('D'),
        ),
        (
            InputMode::StashMenu,
            0,
            Char('d'),
            Action::StashMenuChar('d'),
        ),
        (InputMode::StashMenu, 0, Enter, Action::StashMenuEnter),
        (
            InputMode::BlameMenu,
            0,
            Char('c'),
            Action::BlameMenuChar('c'),
        ),
        (
            InputMode::BlameMenu,
            0,
            Char('p'),
            Action::BlameMenuChar('p'),
        ),
        (
            InputMode::BlameMenu,
            0,
            Char('w'),
            Action::BlameMenuChar('w'),
        ),
        (
            InputMode::BlameMenu,
            0,
            Char('g'),
            Action::BlameMenuChar('g'),
        ),
        (InputMode::BlameMenu, 0, Enter, Action::BlameMenuEnter),
        (InputMode::BlameMenu, 0, Char('q'), Action::BlameMenuCancel),
        (
            InputMode::CommentExport,
            0,
            Enter,
            Action::ExportCommentsCancel,
        ),
        (pending_z, 0, Char('z'), Action::FoldToggleSubtree),
        (pending_z, 0, Char('p'), Action::Pull),
        (pending_g(), 0, Char('g'), Action::MoveToStart),
        (pending_g(), 0, Char('t'), Action::NextTab),
        (pending_g(), 0, Char('T'), Action::PreviousTab),
        (pending_g(), 0, Char('x'), Action::OpenPullRequest),
        (pending_g(), 0, Char('c'), Action::OpenPullRequestCi),
        (pending_g(), 0, Char('w'), Action::ViewRemotely),
        (pending_g(), 0, Char('1'), Action::JumpToTab(1)),
        (pending_g(), 0, Char('9'), Action::JumpToTab(9)),
        (pending_g(), 0, Char('p'), Action::OpenPrsTab),
        (
            pending_z,
            0,
            Char('F'),
            Action::ToggleQuickOpen(QuickOpenEntry::Files),
        ),
        (
            pending_g(),
            0,
            Char(':'),
            Action::ToggleQuickOpen(QuickOpenEntry::Commands),
        ),
        (
            InputMode::DiffVisual,
            0,
            Char('F'),
            Action::ToggleQuickOpen(QuickOpenEntry::Files),
        ),
        (InputMode::DiffVisual, 0, Char('s'), Action::Stage),
        (InputMode::DiffVisual, 0, Char('x'), Action::Revert),
        (InputMode::DiffVisual, 0, Char('q'), Action::Quit),
        (
            InputMode::DiffVisual,
            0,
            Char('V'),
            Action::DiffVisualCancel,
        ),
        (InputMode::DiffVisual, 0, Char('j'), Action::Move(1)),
        (InputMode::DiffVisual, 0, Enter, Action::DiffVisualUnmapped),
        (InputMode::DiffVisual, 0, Down, Action::Move(1)),
    ];
    let map = |mode: InputMode, graph: u8, code: KeyCode, mods: KeyModifiers| {
        event_to_action_ex(
            &Event::Key(KeyEvent::new(code, mods)),
            mode,
            false,
            false,
            graph == STASH,
            graph == COMMIT,
        )
    };
    let command = [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
    ];
    for (mode, graph, code, plain) in rows {
        assert_eq!(
            map(mode, graph, code, KeyModifiers::NONE),
            plain,
            "{mode:?} {code:?}"
        );
        let mut chords: Vec<(KeyCode, KeyModifiers)> = command.iter().map(|&m| (code, m)).collect();
        if let Char(c) = code {
            if c.is_ascii_alphabetic() {
                for m in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
                    let m = m | KeyModifiers::SHIFT;
                    chords.push((Char(c.to_ascii_lowercase()), m));
                    chords.push((Char(c.to_ascii_uppercase()), m));
                }
            }
        }
        for (code, mods) in chords {
            // Explicit Ctrl chords keep their own bindings: Ctrl-c
            // everywhere, Ctrl-p (Ctrl alone) where Quick Open opens,
            // Ctrl-f (Shift allowed) where search in files opens,
            // Ctrl-o / u / d / w in Normal and pending chords, Ctrl-u / d in
            // highlight.
            let ctrl_only = !mods.intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
                && mods.contains(KeyModifiers::CONTROL);
            let normal_like = matches!(
                mode,
                InputMode::Normal { .. } | InputMode::ZPending { .. } | InputMode::GPending { .. }
            );
            let visual = mode == InputMode::DiffVisual;
            let bound = match code {
                Char('c') => true,
                Char('p') => (normal_like || visual) && mods == KeyModifiers::CONTROL,
                Char('f' | 'F') => normal_like || visual,
                Char('o') => normal_like,
                Char('w' | 'W') => normal_like,
                Char('u' | 'd') => normal_like || visual,
                _ => false,
            };
            if ctrl_only && bound {
                continue;
            }
            let want = match (mode, code) {
                (InputMode::DiffVisual, Char(_)) => Action::DiffVisualUnmapped,
                (InputMode::GPending { .. }, _) => Action::GChordCancel,
                _ => Action::None,
            };
            assert_eq!(
                map(mode, graph, code, mods),
                want,
                "{mode:?} {code:?} {mods:?}"
            );
        }
    }

    // The reported cases, spelled out.
    let ev = |code, mods| Event::Key(KeyEvent::new(code, mods));
    for mods in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        assert_ne!(
            event_to_action(&ev(Char('p'), mods), n, false, false),
            Action::Pull
        );
    }
    let ctrl_p = event_to_action(&ctrl(Char('p')), n, false, false);
    assert_eq!(ctrl_p, Action::ToggleQuickOpen(QuickOpenEntry::Files));
    for push in [key(Char('P')), shift(Char('P')), shift(Char('p'))] {
        assert_eq!(event_to_action(&push, n, false, false), Action::Push);
    }
    assert_ne!(
        event_to_action(&ctrl(Char('y')), InputMode::Confirm, false, false),
        Action::ConfirmYes
    );
    assert_ne!(
        event_to_action(&ctrl(Char('q')), InputMode::Help, false, false),
        Action::ToggleHelp
    );
    assert_ne!(
        event_to_action_ex(&ctrl(Char('p')), n, false, false, true, false),
        Action::GraphStashPop
    );
    // Shift alone still pans; Esc with a modifier still cancels.
    assert_eq!(
        event_to_action(&shift(Left), n, false, false),
        Action::PanDiff(-1)
    );
    assert_eq!(
        event_to_action(&ctrl(KeyCode::Esc), InputMode::Confirm, false, false),
        Action::ConfirmNo
    );
    // Explicit chords still win in pending modes.
    assert_eq!(
        event_to_action(&ctrl(Char('u')), pending_z, false, false),
        Action::Move(-5)
    );
    assert_eq!(
        event_to_action(&ctrl(Char('d')), pending_g(), false, false),
        Action::Move(5)
    );
    assert_eq!(
        event_to_action(&ctrl(Char('o')), pending_g(), true, true),
        Action::ToggleFullContext
    );
    assert_eq!(
        event_to_action(&ctrl(Char('u')), InputMode::DiffVisual, true, true),
        Action::Move(-5)
    );
}

/// Windows AltGr symbols arrive as the char plus Ctrl+Alt.
#[test]
fn altgr_symbols_act_and_type_as_the_plain_char() {
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    let ev = |c: char, mods| Event::Key(KeyEvent::new(KeyCode::Char(c), mods));
    assert_eq!(
        event_to_action(&ev('\\', altgr), normal(), false, false),
        Action::ToggleDiffWrap
    );
    assert_eq!(
        event_to_action(&ev('@', altgr), InputMode::SearchPrompt, false, false),
        Action::SearchChar('@')
    );
    assert_eq!(
        event_to_action(
            &ev('{', altgr | KeyModifiers::SHIFT),
            quick_open(),
            false,
            false
        ),
        Action::QuickOpenChar('{')
    );
    assert_eq!(
        event_to_action(&ev('p', altgr), normal(), false, false),
        Action::None
    );
    assert_eq!(
        event_to_action(&ev('1', altgr), pending_g(), false, false),
        Action::GChordCancel
    );
    assert_eq!(
        event_to_action(
            &ev('\\', altgr | KeyModifiers::SUPER),
            normal(),
            false,
            false
        ),
        Action::None
    );
}
