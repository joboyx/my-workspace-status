//! Crossterm events to [`Action`].

use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::action::{Action, QuickOpenEntry};
use super::bindings::{
    self, has_command_modifier, is_ctrl_chord, COMMAND_MODIFIERS, FOLD_SUBTREE, GRAPH_COMMIT_KEYS,
    GRAPH_STASH_KEYS, G_CHORD_KEYS, HIGHLIGHT_CTRL_KEYS, HIGHLIGHT_KEYS, NORMAL_CTRL_KEYS,
    NORMAL_KEYS, OTHER_PANE, PAN_LEFT, PAN_RIGHT,
};

/// Rows PgUp / PgDn scroll the `?` help body.
///
/// The help dialog has no page height of its own in the key map.
pub const HELP_PAGE_ROWS: i32 = 10;

/// Check rows PgUp / PgDn and Ctrl-u / Ctrl-d move the Doctor cursor.
///
/// Doctor rows wrap to a few lines each, so a page is a few rows.
pub const DOCTOR_PAGE_ROWS: i32 = 5;

/// Window for `zz` / `gg` after the first key.
pub const DOUBLE_TAP_MS: u64 = 400;

/// Drop a protocol same-key Press only inside this VTE burst.
///
/// Typeless key-up arrives in the same stdin burst (a few milliseconds).
/// A human second `g` for `gg` is later and must stay inside
/// [`DOUBLE_TAP_MS`]. An 80 ms window ate that tap and left `GPending`
/// armed, so a later `t` / `T` fired bare. Raw bytes never use this
/// window.
pub const G_CHORD_PROTOCOL_ECHO_MS: u64 = 8;

/// Where a key event's bytes came from.
///
/// [`KeyStrokeOrigin::Protocol`] is CSI-u (typed or typeless) and every
/// constructed event. A same-key Press inside
/// [`G_CHORD_PROTOCOL_ECHO_MS`] is the release of that tap.
/// [`KeyStrokeOrigin::LegacyByte`] is a traditional single-byte key.
/// A same-key Press is a new tap (`gg`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyStrokeOrigin {
    /// CSI-u, other escape sequences, Resize, and constructed events.
    #[default]
    Protocol,
    /// Raw / legacy single-byte (or UTF-8) key. Never a protocol echo.
    LegacyByte,
}

/// Last `g`-chord Press and whether a matching protocol release is due.
///
/// VTE often sends all-keys CSI-u without event types, so press and
/// release are both [`KeyEventKind::Press`]. A same-key Press inside
/// [`G_CHORD_PROTOCOL_ECHO_MS`] is that release. A later same-key Press
/// is a new tap (`gg`). A raw-byte Press is never an echo.
#[derive(Clone, Copy, Debug, Default)]
pub struct GChordEchoState {
    last: Option<(KeyCode, KeyModifiers)>,
    last_at: Option<Instant>,
    awaiting_release: bool,
}

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
    ///
    /// Key release is ignored and must not clear this pending. CSI-u with
    /// event types sends a Release after every Press. CSI-u without event
    /// types sends a second Press. A protocol same-key Press inside
    /// [`G_CHORD_PROTOCOL_ECHO_MS`] is that release and is dropped. A
    /// later protocol `g` completes `gg` (one CSI-u per tap). A raw-byte
    /// second `g` also completes `gg`. A real Release clears the echo.
    /// When the pending expires, the echo for that arming `g` is cleared.
    GPending {
        search_active: bool,
    },
    SearchPrompt,
    Confirm,
    Help,
    /// `?` overlay with `/` help query open (chars append; highlight only).
    HelpSearch,
    StashMenu,
    /// `A` blame-actions menu: `c` / `p` / `w` / `g` pick a row, Esc / `q`
    /// close it.
    BlameMenu,
    BranchPicker,
    /// Compare-only branch or commit picker (no checkout).
    ComparePicker,
    /// Graph `o` overlay: mark local branches, Enter applies ancestor focus.
    GraphFocusPicker,
    CreateBranch,
    /// `;` comment overlay (type body, Shift+Enter newline, Enter save,
    /// Ctrl-R resolve).
    Comment,
    /// `V` visual-line highlight on a focused file diff (`j`/`k` extend,
    /// `s`/`u` stage/unstage the range).
    DiffVisual,
    /// `y` markdown export overlay (Esc closes).
    CommentExport,
    /// Quick Open (`:` / `>` commands, Ctrl-p / `F` files). Query, list, Enter.
    QuickOpen,
    /// Search-in-files dialog (Ctrl-f). `results` is the results zone:
    /// `j` / `k` move there, while the query zone types every printable key.
    SearchFiles {
        results: bool,
    },
    /// Pinned icon popover (click on an icon, or `gh`): `j` / `k` move,
    /// Enter runs, `y` copies the focused line, `r` refreshes the PR of a
    /// PR section, `g` arms [`Self::PopoverGPending`], Esc closes. The
    /// mouse stays live (click, drag-select, wheel). A hover peek is not a
    /// mode.
    Popover,
    /// `g` armed inside a pinned popover: `x` opens the PR of its PR
    /// section and `c` its CI page, like `gx` / `gc` on the row. Any other
    /// key (Esc, right click) only ends the chord; the popover stays.
    PopoverGPending,
    /// Doctor setup checkup overlay: `j` / `k` / arrows / page keys / `g` /
    /// `G` move, `y` copies the focused fix line, `r` re-runs, Esc / `q`
    /// close. Every other key is swallowed.
    Doctor,
}

fn same_g_chord_key(last: Option<(KeyCode, KeyModifiers)>, key: &KeyEvent) -> bool {
    last.is_some_and(|(code, mods)| code == key.code && mods == key.modifiers)
}

fn records_g_chord_press(mode: InputMode, key: &KeyEvent) -> bool {
    matches!(
        mode,
        InputMode::GPending { .. } | InputMode::PopoverGPending
    ) || (matches!(
        mode,
        InputMode::Normal { .. }
            | InputMode::ZPending { .. }
            | InputMode::DiffVisual
            | InputMode::Popover
    ) && matches!(key.code, KeyCode::Char('g')))
}

#[cfg(test)]
/// Drop a typeless CSI-u echo of a `g`-chord key. Record the Press when
/// it arms or consumes [`InputMode::GPending`].
///
/// Same as [`drop_g_chord_echo`] with [`KeyStrokeOrigin::Protocol`].
/// In-memory constructed events use this path.
///
/// Returns true when the caller must not dispatch (echo or Release).
pub fn drop_protocol_dup_g_chord_press(
    state: &mut GChordEchoState,
    mode: InputMode,
    event: &Event,
) -> bool {
    drop_g_chord_echo(state, mode, event, KeyStrokeOrigin::Protocol)
}

/// Drop a protocol echo of a `g`-chord key. Record the Press when it
/// arms or consumes [`InputMode::GPending`].
///
/// A [`KeyStrokeOrigin::Protocol`] same-key Press inside
/// [`G_CHORD_PROTOCOL_ECHO_MS`] is the release of that tap and is
/// dropped. A later protocol same-key Press is a new tap. A
/// [`KeyStrokeOrigin::LegacyByte`] same-key Press is a new tap. A real
/// [`KeyEventKind::Release`] clears the echo so the next protocol Press
/// is a new tap. Search, Quick Open, and other overlays do not record a
/// typed `g`.
///
/// Returns true when the caller must not dispatch (echo or Release).
#[cfg(test)]
pub fn drop_g_chord_echo(
    state: &mut GChordEchoState,
    mode: InputMode,
    event: &Event,
    origin: KeyStrokeOrigin,
) -> bool {
    drop_g_chord_echo_at(state, mode, event, origin, Instant::now())
}

/// [`drop_g_chord_echo`] at `now`, the instant the event's bytes arrived.
///
/// The live loop passes the stdin read instant, so a press and its
/// typeless release from one burst stay inside
/// [`G_CHORD_PROTOCOL_ECHO_MS`] however late the loop handles them.
pub(crate) fn drop_g_chord_echo_at(
    state: &mut GChordEchoState,
    mode: InputMode,
    event: &Event,
    origin: KeyStrokeOrigin,
    now: Instant,
) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind == KeyEventKind::Release {
        if state.awaiting_release && same_g_chord_key(state.last, key) {
            state.awaiting_release = false;
        }
        return true;
    }
    if key.kind != KeyEventKind::Press {
        return false;
    }
    if origin == KeyStrokeOrigin::LegacyByte {
        if records_g_chord_press(mode, key) {
            state.last = Some((key.code, key.modifiers));
            state.last_at = Some(now);
            state.awaiting_release = false;
        }
        return false;
    }
    if state.awaiting_release && same_g_chord_key(state.last, key) {
        let is_echo = state.last_at.is_some_and(|at| {
            now.saturating_duration_since(at) <= Duration::from_millis(G_CHORD_PROTOCOL_ECHO_MS)
        });
        if is_echo {
            state.awaiting_release = false;
            return true;
        }
    }
    if records_g_chord_press(mode, key) {
        state.last = Some((key.code, key.modifiers));
        state.last_at = Some(now);
        state.awaiting_release = true;
    }
    false
}

/// Clear a stale arming-`g` echo after [`DOUBLE_TAP_MS`] at `now`.
///
/// After `gt` / `gT` / `g1`–`g9` the pending is already gone and `last`
/// is that second key. Keep that echo so a typeless release in Normal
/// does not fire bare `t` / `T`.
pub fn expire_stale_g_chord_echo(
    echo: &mut GChordEchoState,
    g_pending_at: &mut Option<Instant>,
    now: Instant,
) {
    let Some(at) = *g_pending_at else {
        return;
    };
    if now.saturating_duration_since(at) <= Duration::from_millis(DOUBLE_TAP_MS) {
        return;
    }
    *g_pending_at = None;
    if matches!(echo.last, Some((KeyCode::Char('g'), _))) {
        echo.last = None;
        echo.last_at = None;
        echo.awaiting_release = false;
    }
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
/// Unshifted `h` / `l` / arrows fold the workspace tree when the left pane
/// is focused and pan otherwise; [`event_to_action_with`] takes the fold
/// decision from the caller.
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
        !focus_right,
    )
}

/// [`event_to_action_ex`] with an explicit fold-vs-pan flag for `h` / `l`.
///
/// `hl_folds` is true when `h` / `l` / arrows fold the focused row: the
/// workspace tree, or a folder row of a focused commit-file list. Other
/// rows and panes pass false so those keys pan.
///
/// In the search-in-files dialog the mouse only reaches the option chips:
/// a left click without Ctrl is [`Action::Click`] (`app::map_event` turns
/// it into [`Action::SearchFilesToggleOption`] on a chip and drops it
/// elsewhere), motion is [`Action::PointerMove`] for their hover, and any
/// other mouse event is [`Action::None`]. Prompts, menus, and the other
/// list dialogs drop the mouse.
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
                    | InputMode::BlameMenu
                    | InputMode::BranchPicker
                    | InputMode::ComparePicker
                    | InputMode::GraphFocusPicker
                    | InputMode::CreateBranch
                    | InputMode::Comment
                    | InputMode::CommentExport
                    | InputMode::QuickOpen
                    | InputMode::Doctor,
            ) {
                Action::None
            } else if matches!(mode, InputMode::SearchFiles { .. }) {
                // Only the option chips on the border take the mouse:
                // `app::map_event` keeps a left click only on a chip, and
                // motion feeds their hover. Anything else does nothing.
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left)
                        if !mouse.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        Action::Click {
                            col: mouse.column,
                            row: mouse.row,
                        }
                    }
                    MouseEventKind::Moved => Action::PointerMove {
                        col: mouse.column,
                        row: mouse.row,
                    },
                    _ => Action::None,
                }
            } else if mode == InputMode::Popover
                && mouse.kind == MouseEventKind::Down(MouseButton::Right)
            {
                // Right-click is Esc: it closes the pinned popover.
                Action::PopoverClose
            } else if matches!(
                mode,
                InputMode::ZPending { .. }
                    | InputMode::GPending { .. }
                    | InputMode::PopoverGPending
            ) && mouse.kind == MouseEventKind::Down(MouseButton::Right)
            {
                // Right-click is Esc; in a pending chord, Esc only ends the chord.
                key_to_action(
                    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                    mode,
                    right_is_diff,
                    focus_right,
                    graph_stash_focused,
                    graph_commit_focused,
                    hl_folds,
                )
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
    let key = fold_shift_letter(fold_altgr_symbol(fold_kitty_csi_u_arrows(key)));
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

/// Crossterm 0.28 `event::read` leaves kitty CSI-u arrow codepoints as
/// [`KeyCode::Char`] in the private-use range (`57350`–`57353`). Fold them
/// onto Left / Right / Up / Down so Shift+←/→ pan the same as a constructed
/// [`KeyCode::Left`] event. Keypad arrows (`57417`–`57420`) already map.
fn fold_kitty_csi_u_arrows(mut key: KeyEvent) -> KeyEvent {
    let KeyCode::Char(c) = key.code else {
        return key;
    };
    key.code = match u32::from(c) {
        57350 => KeyCode::Left,
        57351 => KeyCode::Right,
        57352 => KeyCode::Up,
        57353 => KeyCode::Down,
        _ => return key,
    };
    key
}

/// True when `key` is a modifier chord with no explicit binding in a
/// plain-key mode, so it must not run the plain-key action.
///
/// Esc is exempt: Esc with a modifier still cancels or closes.
fn is_unbound_chord(key: KeyEvent) -> bool {
    key.code != KeyCode::Esc && has_command_modifier(key)
}

/// True for a Ctrl-u or Ctrl-d press (or key repeat), the half-page keys
/// that glide the viewport ([`is_ctrl_chord`] rules). A release is false.
pub(crate) fn is_half_page_chord(event: &Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    key.kind != KeyEventKind::Release && (is_ctrl_chord(*key, 'u') || is_ctrl_chord(*key, 'd'))
}

/// Windows `event::read` reports a character typed with AltGr (`\`, `@`,
/// `{` on many European layouts) as that character plus Ctrl+Alt.
///
/// Fold exactly Ctrl+Alt on a symbol back to the plain character so it
/// still acts and types. Letters, digits, whitespace, and control
/// characters keep Ctrl+Alt, so Ctrl+Alt+p stays an unbound chord.
fn fold_altgr_symbol(mut key: KeyEvent) -> KeyEvent {
    let KeyCode::Char(c) = key.code else {
        return key;
    };
    let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
    if key.modifiers.intersection(COMMAND_MODIFIERS) != altgr
        || c.is_ascii_alphanumeric()
        || c.is_whitespace()
        || c.is_control()
    {
        return key;
    }
    key.modifiers.remove(altgr);
    key
}

/// A terminal that does not honour `REPORT_ALTERNATE_KEYS` reports
/// Shift+letter as the unshifted codepoint plus [`KeyModifiers::SHIFT`]
/// (`Char('o')` + SHIFT, not `Char('O')`). With the flag, `tty` already
/// decodes the shifted character.
///
/// Fold that into the uppercase binding so `O` / `S` capitals match
/// what a raw `'O'` byte already does. SHIFT stays set so Shift+h/l still pan.
fn fold_shift_letter(mut key: KeyEvent) -> KeyEvent {
    let KeyCode::Char(c) = key.code else {
        return key;
    };
    if !c.is_ascii_lowercase() || !key.modifiers.contains(KeyModifiers::SHIFT) {
        return key;
    }
    if has_command_modifier(key) {
        return key;
    }
    key.code = KeyCode::Char(c.to_ascii_uppercase());
    key
}

/// Keys that fire again while held (typical terminal key-repeat).
pub(crate) fn key_repeats_while_held(key: KeyEvent) -> bool {
    let key = fold_kitty_csi_u_arrows(key);
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(key.code, KeyCode::Char('u') | KeyCode::Char('d'));
    }
    matches!(
        key.code,
        KeyCode::Char('h')
            | KeyCode::Char('H')
            | KeyCode::Char('j')
            | KeyCode::Char('J')
            | KeyCode::Char('k')
            | KeyCode::Char('K')
            | KeyCode::Char('l')
            | KeyCode::Char('L')
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
        InputMode::Comment => {
            typing
                || matches!(
                    key.code,
                    KeyCode::Delete | KeyCode::Home | KeyCode::End | KeyCode::Enter
                )
        }
        InputMode::BranchPicker | InputMode::ComparePicker => match key.code {
            _ if list_overlay_move(key).is_some() => true,
            KeyCode::Backspace => true,
            KeyCode::Char(_) => typing,
            _ => false,
        },
        InputMode::GraphFocusPicker => match key.code {
            _ if list_overlay_move(key).is_some() => true,
            KeyCode::Backspace => true,
            KeyCode::Char(' ') => false,
            KeyCode::Char(_) => typing,
            _ => false,
        },
        InputMode::QuickOpen | InputMode::SearchFiles { .. } => match key.code {
            _ if list_overlay_move(key).is_some() => true,
            KeyCode::Backspace => true,
            // `e` in the results zone opens the editor: one-shot, like Enter.
            KeyCode::Char('e')
                if mode == (InputMode::SearchFiles { results: true })
                    && key.modifiers.is_empty() =>
            {
                false
            }
            KeyCode::Char(_) => typing,
            _ => false,
        },
        InputMode::Normal { .. }
        | InputMode::ZPending { .. }
        | InputMode::GPending { .. }
        | InputMode::DiffVisual
        | InputMode::Confirm
        | InputMode::Help
        | InputMode::StashMenu
        | InputMode::BlameMenu
        | InputMode::CommentExport
        | InputMode::Popover
        | InputMode::PopoverGPending
        | InputMode::Doctor => false,
    }
}

/// Queued press / repeat / release of the same held nav key.
///
/// The input thread forwards these marked; the loop thread drops one
/// unless it types into a text field, so a hold cannot flush as a burst
/// after release ([`super::event_pump::keep_held_nav_backlog`]).
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

/// Map one folded key press in `mode` to an [`Action`].
///
/// Explicit chords (Ctrl-c, the Ctrl-p Quick Open opener, and the per-mode
/// Ctrl bindings) match first. In Normal, pending `z` / `g`, Help, Confirm,
/// the stash and blame menus, comment export, and visual highlight a key
/// with Ctrl / Alt / Super / Hyper / Meta never runs the plain-key action
/// ([`is_unbound_chord`]).
/// Text overlays keep their own matching.
fn key_to_action(
    key: KeyEvent,
    mode: InputMode,
    right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
    hl_folds: bool,
) -> Action {
    // Chip: `bindings::CTRL_C_EXIT` (any Ctrl+c, other modifiers allowed).
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::CtrlC;
    }
    // Chip: `bindings::SEARCH_IN_FILES`.
    if is_search_files_key(key) {
        match mode {
            InputMode::Normal { .. }
            | InputMode::ZPending { .. }
            | InputMode::GPending { .. }
            | InputMode::DiffVisual
            | InputMode::SearchFiles { .. } => return Action::ToggleSearchFiles,
            // Text and list overlays keep their own Ctrl handling.
            InputMode::QuickOpen
            | InputMode::SearchPrompt
            | InputMode::HelpSearch
            | InputMode::Comment
            | InputMode::CreateBranch
            | InputMode::BranchPicker
            | InputMode::ComparePicker
            | InputMode::GraphFocusPicker
            | InputMode::StashMenu
            | InputMode::BlameMenu => {}
            _ => return Action::None,
        }
    }
    // Chips: `bindings::QUICK_OPEN_COMMANDS`, `GO_TO_FILE`, `GO_TO_FILE_F`.
    if let Some(entry) = quick_open_key(key) {
        match mode {
            InputMode::Normal { .. }
            | InputMode::ZPending { .. }
            | InputMode::GPending { .. }
            | InputMode::DiffVisual => {
                return Action::ToggleQuickOpen(entry);
            }
            InputMode::QuickOpen
            | InputMode::SearchFiles { .. }
            | InputMode::SearchPrompt
            | InputMode::HelpSearch
            | InputMode::Comment
            | InputMode::CreateBranch
            | InputMode::BranchPicker
            | InputMode::ComparePicker
            | InputMode::GraphFocusPicker
            | InputMode::StashMenu
            | InputMode::BlameMenu => {}
            _ => return Action::None,
        }
    }
    match mode {
        InputMode::Help => match key.code {
            _ if is_unbound_chord(key) => Action::None,
            KeyCode::Char('/') => Action::SearchStart,
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc | KeyCode::Char('?') => {
                Action::ToggleHelp
            }
            KeyCode::Char('j') | KeyCode::Down => Action::HelpScroll(1),
            KeyCode::Char('k') | KeyCode::Up => Action::HelpScroll(-1),
            KeyCode::PageDown => Action::HelpScroll(HELP_PAGE_ROWS),
            KeyCode::PageUp => Action::HelpScroll(-HELP_PAGE_ROWS),
            _ => Action::None,
        },
        // Letters type into the query; only the arrows and paging scroll.
        InputMode::HelpSearch => match key.code {
            KeyCode::Esc => Action::SearchCancel,
            KeyCode::Down => Action::HelpScroll(1),
            KeyCode::Up => Action::HelpScroll(-1),
            KeyCode::PageDown => Action::HelpScroll(HELP_PAGE_ROWS),
            KeyCode::PageUp => Action::HelpScroll(-HELP_PAGE_ROWS),
            KeyCode::Backspace => Action::SearchBackspace,
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::SearchBackspace
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::SearchChar(c)
            }
            _ => Action::None,
        },
        // A modifier chord falls through to `normal_key` (see GPending).
        InputMode::ZPending { .. } => match key.code {
            _ if !has_command_modifier(key) && FOLD_SUBTREE.matches(key) => {
                FOLD_SUBTREE.action.clone()
            }
            KeyCode::Esc => Action::None,
            _ => normal_key(
                key,
                right_is_diff,
                focus_right,
                graph_stash_focused,
                graph_commit_focused,
                hl_folds,
            ),
        },
        // A modifier chord falls through to `normal_key`, which keeps
        // Ctrl-o / Ctrl-u / Ctrl-d / Ctrl-w and maps the rest to None.
        InputMode::GPending { .. } => {
            if !has_command_modifier(key) {
                if let Some(action) = bound_action(G_CHORD_KEYS, key) {
                    return action;
                }
            }
            match key.code {
                KeyCode::Char(c @ '1'..='9') if !has_command_modifier(key) => {
                    Action::JumpToTab(c as u8 - b'0')
                }
                KeyCode::Esc => Action::None,
                _ => normal_key(
                    key,
                    right_is_diff,
                    focus_right,
                    graph_stash_focused,
                    graph_commit_focused,
                    hl_folds,
                ),
            }
        }
        InputMode::Confirm => match key.code {
            // Ctrl-y / Alt-y never confirm a write.
            _ if is_unbound_chord(key) => Action::None,
            KeyCode::Char('Y') => Action::ConfirmYesClean,
            KeyCode::Char('y') => Action::ConfirmYes,
            // Enter never confirms: a write needs the key the box shows.
            KeyCode::Enter => Action::ConfirmEnter,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Action::ConfirmNo,
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
            // Menu letters are hotkeys (pop, apply, drop), not typed text.
            _ if is_unbound_chord(key) => Action::None,
            KeyCode::Esc => Action::StashMenuCancel,
            KeyCode::Enter => Action::StashMenuEnter,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                Action::StashMenuChar(c)
            }
            _ => Action::None,
        },
        InputMode::BlameMenu => match key.code {
            // Menu letters are hotkeys (changes, previous, working tree, graph).
            _ if is_unbound_chord(key) => Action::None,
            KeyCode::Esc | KeyCode::Char('q') => Action::BlameMenuCancel,
            KeyCode::Enter => Action::BlameMenuEnter,
            KeyCode::Char(c) => Action::BlameMenuChar(c),
            _ => Action::None,
        },
        InputMode::ComparePicker => match list_overlay_move(key) {
            Some(delta) => Action::ComparePickerMove(delta),
            None => match key.code {
                KeyCode::Esc => Action::ComparePickerCancel,
                KeyCode::Enter => Action::ComparePickerSubmit,
                KeyCode::Backspace => Action::ComparePickerBackspace,
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::ComparePickerChar(c)
                }
                _ => Action::None,
            },
        },
        InputMode::BranchPicker => match list_overlay_move(key) {
            Some(delta) => Action::BranchMove(delta),
            None => match key.code {
                KeyCode::Esc => Action::BranchCancel,
                KeyCode::Enter => Action::BranchSubmit,
                KeyCode::Backspace => Action::BranchBackspace,
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::BranchChar(c)
                }
                _ => Action::None,
            },
        },
        InputMode::GraphFocusPicker => match list_overlay_move(key) {
            Some(delta) => Action::GraphFocusMove(delta),
            None => match key.code {
                KeyCode::Esc => Action::GraphFocusCancel,
                KeyCode::Enter => Action::GraphFocusSubmit,
                KeyCode::Backspace => Action::GraphFocusBackspace,
                // Branch names cannot contain a space, so space never types.
                KeyCode::Char(' ') => Action::GraphFocusToggle,
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::GraphFocusClear
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Action::GraphFocusChar(c)
                }
                _ => Action::None,
            },
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
        InputMode::Comment => {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R'))
            {
                return Action::CommentToggleResolved;
            }
            match key.code {
                KeyCode::Esc => Action::CommentCancel,
                KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                    Action::CommentSubmit
                }
                _ => Action::CommentInput(key),
            }
        }
        InputMode::CommentExport => match key.code {
            _ if is_unbound_chord(key) => Action::None,
            KeyCode::Esc | KeyCode::Enter => Action::ExportCommentsCancel,
            _ => Action::None,
        },
        InputMode::Popover => match key.code {
            _ if is_unbound_chord(key) => Action::None,
            KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => Action::PopoverMove(1),
            KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => Action::PopoverMove(-1),
            KeyCode::Enter => Action::PopoverRun,
            KeyCode::Char('y') => Action::PopoverCopyLine,
            // Refresh this PR; a popover with no PR section ignores it.
            KeyCode::Char('r') => Action::RefreshPullRequest,
            // `gx` / `gc` on the popover's PR section.
            KeyCode::Char('g') => Action::ArmGChord,
            KeyCode::Esc => Action::PopoverClose,
            _ => Action::None,
        },
        // Only `x` and `c` complete the chord; any other key ends it and
        // does nothing else (it never reaches the popover keys).
        InputMode::PopoverGPending => match key.code {
            KeyCode::Char('x') if !has_command_modifier(key) => Action::OpenPullRequest,
            KeyCode::Char('c') if !has_command_modifier(key) => Action::OpenPullRequestCi,
            _ => Action::GChordCancel,
        },
        InputMode::Doctor => doctor_key_action(key),
        InputMode::QuickOpen => quick_open_key_action(key),
        InputMode::SearchFiles { results } => search_files_key_action(key, results),
        InputMode::DiffVisual => diff_visual_key(key),
        InputMode::Normal { .. } => normal_key(
            key,
            right_is_diff,
            focus_right,
            graph_stash_focused,
            graph_commit_focused,
            hl_folds,
        ),
    }
}

/// Quick Open opener for `key`, if any: `:` opens commands (vim command
/// line), Ctrl-p and `F` open files.
///
/// `F` matches with or without SHIFT; a command modifier (Ctrl+Shift+F,
/// Alt+F) never opens. Ctrl-p matches only Ctrl alone, so Ctrl+Shift+P
/// stays unbound.
fn quick_open_key(key: KeyEvent) -> Option<QuickOpenEntry> {
    if key.code == KeyCode::Char('p') && key.modifiers == KeyModifiers::CONTROL {
        return Some(QuickOpenEntry::Files);
    }
    if has_command_modifier(key) {
        return None;
    }
    match key.code {
        KeyCode::Char(':') => Some(QuickOpenEntry::Commands),
        KeyCode::Char('F') => Some(QuickOpenEntry::Files),
        _ => None,
    }
}

/// True for the search-in-files opener: Ctrl-f, alone or with Shift.
///
/// Without the kitty keyboard protocol Ctrl-Shift-F sends the same byte as
/// Ctrl-f, so both map the same way. Alt, Super, Hyper, or Meta never
/// match, and plain `f` stays fetch.
fn is_search_files_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('f' | 'F'))
        && key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::CONTROL
}

/// Cursor step for a list overlay key (Quick Open and every picker), if any.
///
/// Up / Down, Ctrl-n / Ctrl-p, and Ctrl-j / Ctrl-k move. Letters never move,
/// so every printable character types into the filter. Quick Open follows
/// the same rule: Ctrl-p / Ctrl-k move up, `:` and `F` type, and only Esc
/// closes it.
fn list_overlay_move(key: KeyEvent) -> Option<i32> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Down => Some(1),
        KeyCode::Up => Some(-1),
        KeyCode::Char('n' | 'j') if ctrl => Some(1),
        KeyCode::Char('p' | 'k') if ctrl => Some(-1),
        _ => None,
    }
}

/// Keys inside the Doctor overlay. Anything else is swallowed.
///
/// Ctrl-d / Ctrl-u page like PgDn / PgUp; any other chord does nothing.
fn doctor_key_action(key: KeyEvent) -> Action {
    if is_ctrl_chord(key, 'd') {
        return Action::DoctorMove(DOCTOR_PAGE_ROWS);
    }
    if is_ctrl_chord(key, 'u') {
        return Action::DoctorMove(-DOCTOR_PAGE_ROWS);
    }
    match key.code {
        _ if is_unbound_chord(key) => Action::None,
        KeyCode::Char('j') | KeyCode::Down => Action::DoctorMove(1),
        KeyCode::Char('k') | KeyCode::Up => Action::DoctorMove(-1),
        KeyCode::PageDown => Action::DoctorMove(DOCTOR_PAGE_ROWS),
        KeyCode::PageUp => Action::DoctorMove(-DOCTOR_PAGE_ROWS),
        KeyCode::Char('g') | KeyCode::Home => Action::DoctorTop,
        KeyCode::Char('G') | KeyCode::End => Action::DoctorBottom,
        KeyCode::Char('y') => Action::DoctorCopy,
        KeyCode::Char('r') => Action::DoctorRerun,
        KeyCode::Esc | KeyCode::Char('q') => Action::DoctorClose,
        _ => Action::None,
    }
}

fn quick_open_key_action(key: KeyEvent) -> Action {
    if let Some(delta) = list_overlay_move(key) {
        return Action::QuickOpenMove(delta);
    }
    match key.code {
        KeyCode::Esc => Action::QuickOpenEscape,
        KeyCode::Enter => Action::QuickOpenSubmit,
        KeyCode::Backspace => Action::QuickOpenBackspace,
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            Action::QuickOpenChar(c)
        }
        _ => Action::None,
    }
}

/// Keys inside the search-in-files dialog (Ctrl-f is matched before).
///
/// Both zones: list moves and PgUp / PgDn move the highlight (and enter
/// the results zone), Enter opens the highlighted hit, Tab widens to all
/// repos, Esc clears the query (and closes on an empty query). The query
/// zone types every other printable key without a command modifier. The
/// results zone also moves on `j` / `k` and edits on `e`; any other
/// printable key or Backspace goes back to the query and applies there. No
/// key flips a match option: a click on an option chip does (see
/// [`event_to_action_with`]), so an Alt chord stays [`Action::None`] here.
fn search_files_key_action(key: KeyEvent, results: bool) -> Action {
    if let Some(delta) = list_overlay_move(key) {
        return Action::SearchFilesMove(delta);
    }
    match key.code {
        KeyCode::Esc => Action::SearchFilesEscape,
        KeyCode::Enter => Action::SearchFilesSubmit,
        KeyCode::Tab => Action::SearchFilesToggleScope,
        KeyCode::PageDown => Action::SearchFilesPage(1),
        KeyCode::PageUp => Action::SearchFilesPage(-1),
        KeyCode::Backspace => Action::SearchFilesBackspace,
        KeyCode::Char('j') if results && key.modifiers.is_empty() => Action::SearchFilesMove(1),
        KeyCode::Char('k') if results && key.modifiers.is_empty() => Action::SearchFilesMove(-1),
        KeyCode::Char('e') if results && key.modifiers.is_empty() => Action::SearchFilesEdit,
        KeyCode::Char(c) if !has_command_modifier(key) => Action::SearchFilesChar(c),
        _ => Action::None,
    }
}

/// Visual-line keys on a focused file diff.
///
/// `j` / `k` / arrows, `gg` / `G` / Home / End, PgUp / PgDn, and Ctrl-u /
/// Ctrl-d move the head (and extend or shrink the range). `;` comments that
/// range. `s` / `u` stage / unstage the highlighted add/del lines. `x`
/// reverts them from the worktree (after a confirm). `'` copies an entity
/// reference for the highlighted span. `?`, `q`, and `m` act as in
/// normal mode. Esc or a second `V` leaves highlight without commenting.
/// Any other key is [`Action::DiffVisualUnmapped`] so the status can say
/// how to leave; an unbound modifier chord on a non-character key is
/// [`Action::None`]. `:` (commands) and Ctrl-p / `F` (files) open Quick
/// Open before this map runs; the highlight stays.
fn diff_visual_key(key: KeyEvent) -> Action {
    if let Some(action) = bound_action(HIGHLIGHT_CTRL_KEYS, key) {
        return action;
    }
    if is_unbound_chord(key) {
        return match key.code {
            KeyCode::Char(_) => Action::DiffVisualUnmapped,
            _ => Action::None,
        };
    }
    if let Some(action) = bound_action(HIGHLIGHT_KEYS, key) {
        return action;
    }
    match key.code {
        KeyCode::Char(_)
        | KeyCode::Enter
        | KeyCode::Tab
        | KeyCode::BackTab
        | KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Insert
        | KeyCode::F(_) => Action::DiffVisualUnmapped,
        _ => Action::None,
    }
}

/// Normal-mode keys, including the graph-stash and graph-commit row keys.
///
/// Ctrl-o / Ctrl-u / Ctrl-d / Ctrl-w (close tab) match first. Any other key
/// with Ctrl / Alt / Super / Hyper / Meta is [`Action::None`], so Ctrl-f is
/// not `f` fetch (Ctrl-p opens Quick Open files before this map runs). Shift
/// alone still selects the shifted binding (`P` push).
fn normal_key(
    key: KeyEvent,
    _right_is_diff: bool,
    focus_right: bool,
    graph_stash_focused: bool,
    graph_commit_focused: bool,
    hl_folds: bool,
) -> Action {
    if let Some(action) = bound_action(NORMAL_CTRL_KEYS, key) {
        return action;
    }
    if is_unbound_chord(key) {
        return Action::None;
    }
    if graph_stash_focused {
        if let Some(action) = bound_action(GRAPH_STASH_KEYS, key) {
            return action;
        }
    }
    if graph_commit_focused {
        if let Some(action) = bound_action(GRAPH_COMMIT_KEYS, key) {
            return action;
        }
    }
    if let Some(action) = bound_action(NORMAL_KEYS, key) {
        return action;
    }
    // Chip: `bindings::OTHER_PANE`; the action follows the focused pane.
    if OTHER_PANE.matches(key) {
        return if focus_right {
            Action::FocusLeft
        } else {
            Action::FocusRight
        };
    }
    // `bindings::PAN_LEFT` / `PAN_RIGHT` keys fold or pan here.
    if PAN_LEFT.matches(key) {
        return hl_or_pan(key, -1, hl_folds);
    }
    if PAN_RIGHT.matches(key) {
        return hl_or_pan(key, 1, hl_folds);
    }
    Action::None
}

/// Action of the first `table` binding that matches `key`, if any.
fn bound_action(table: &'static [bindings::KeyBinding], key: KeyEvent) -> Option<Action> {
    bindings::find(table, key).map(|binding| binding.action.clone())
}

/// `h` / `l` / arrows: fold the focused tree or commit-file folder row,
/// otherwise pan.
///
/// Shift+Left / Shift+Right always pan so long paths can move without
/// stealing fold. Unshifted keys fold when `hl_folds` is set.
fn hl_or_pan(key: KeyEvent, delta: i32, hl_folds: bool) -> Action {
    let pan = if delta < 0 {
        Action::PanDiff(-1)
    } else {
        Action::PanDiff(1)
    };
    if key.modifiers.contains(KeyModifiers::SHIFT) || !hl_folds {
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
        MouseEventKind::Down(MouseButton::Left)
            if mouse.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Action::CtrlClick {
                col: mouse.column,
                row: mouse.row,
            }
        }
        MouseEventKind::Down(MouseButton::Left) => Action::Click {
            col: mouse.column,
            row: mouse.row,
        },
        MouseEventKind::Drag(MouseButton::Left) => Action::Drag {
            col: mouse.column,
            row: mouse.row,
        },
        MouseEventKind::Up(MouseButton::Left) => Action::Release,
        MouseEventKind::Down(MouseButton::Right) => Action::BackClick,
        MouseEventKind::Moved => Action::PointerMove {
            col: mouse.column,
            row: mouse.row,
        },
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
mod tests;
