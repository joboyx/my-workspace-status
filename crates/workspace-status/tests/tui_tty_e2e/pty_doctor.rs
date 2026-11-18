//! Doctor setup checkup: the palette opens a report-only overlay that the
//! probe fills off the draw thread, with the terminal's answers to the
//! capability query.

use std::fs;
use std::path::Path;

use crate::harness::{byte_positions, find_bytes, PtySession, CAPS_QUERY, COLS};
use crate::seed::daily_workspace;
use crate::support::{open_command_palette, type_palette_filter, WAIT};

/// Dialog title the overlay paints on its first line.
const TITLE: &str = "Doctor · setup checkup";

/// Palette row that opens the overlay.
const PALETTE_ROW: &str = "Doctor: setup checkup";

/// Status chips, padded to one width as the overlay paints them.
const CHIPS: [&str; 4] = ["ok  ", "?   ", "warn", "fail"];

/// True when the screen shows the summary line
/// `<n> ok · <n> warn · <n> fail · <n> unknown`.
fn has_summary(screen: &str) -> bool {
    const LABELS: [&str; 4] = ["ok", "warn", "fail", "unknown"];
    screen.lines().any(|line| {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == '│')
            .filter(|word| !word.is_empty())
            .collect();
        words.windows(11).any(|w| {
            LABELS.iter().enumerate().all(|(i, label)| {
                w[i * 3].parse::<usize>().is_ok()
                    && w[i * 3 + 1] == *label
                    && (i == LABELS.len() - 1 || w[i * 3 + 2] == "·")
            })
        })
    })
}

/// True when the screen paints the check row `name`: a status chip, the
/// name, then only padding up to the dialog border.
fn has_row(screen: &str, name: &str) -> bool {
    screen.lines().any(|line| {
        CHIPS.iter().any(|chip| {
            let head = format!("{chip} {name}");
            line.match_indices(&head).any(|(at, _)| {
                let rest = line[at + head.len()..].trim_start_matches(' ');
                rest.is_empty() || rest.starts_with('│')
            })
        })
    })
}

/// True when the cursor mark (`> ` in ASCII glyphs) sits on row `name`.
fn cursor_on(screen: &str, name: &str) -> bool {
    screen.lines().any(|line| {
        CHIPS
            .iter()
            .any(|chip| line.contains(&format!("> {chip} {name}")))
    })
}

/// Text after the cursor mark (`> <chip> <name>`), or `None` with no cursor.
fn cursor_row(screen: &str) -> Option<String> {
    screen.lines().find_map(|line| {
        CHIPS.iter().find_map(|chip| {
            let mark = format!("> {chip} ");
            line.find(&mark).map(|at| line[at..].to_string())
        })
    })
}

/// The open Doctor after the probe answered: title, summary, and `name`.
fn doctor_filled_with(screen: &str, name: &str) -> bool {
    screen.contains(TITLE)
        && has_summary(screen)
        && !screen.contains("checking…")
        && has_row(screen, name)
}

/// Every entry under `dir`, by name.
fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// `:` → `doctor` → Enter opens the Doctor; the probe fills its rows (git
/// is on PATH in CI). `j` / `G` / `g` move, `r` re-runs, Esc closes. Fail if
/// the dialog never fills, stays open after Esc, or any of it writes the
/// UI-state store the spawn passes.
#[test]
fn pty_doctor_opens_from_palette_fills_rows_and_writes_no_ui_state() {
    let (_root, workspace) = daily_workspace();
    let ui_dir = workspace.join(".doctor-ui");
    fs::create_dir_all(&ui_dir).unwrap();
    let ui_store = ui_dir.join("ui-state.json");
    let ui_store_env = ui_store.display().to_string();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_GLYPHS", "ascii"),
            ("WS_STATUS_UI_STORE", ui_store_env.as_str()),
        ],
    );
    assert!(
        dir_entries(&ui_dir).is_empty(),
        "launch writes no UI-state store"
    );

    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "doctor");
    tui.wait_contains(PALETTE_ROW, WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| doctor_filled_with(screen, "Terminal size") && screen.contains("TERMINAL"),
        "Enter on `Doctor: setup checkup` opens the dialog and the probe fills it",
        WAIT,
    );
    assert!(
        !tui.screen().contains(PALETTE_ROW),
        "the palette closes when the Doctor opens:\n{}",
        tui.screen()
    );

    tui.key('G');
    tui.wait_pred(
        |screen| doctor_filled_with(screen, "State files") && screen.contains("FILES"),
        "G scrolls to the last row",
        WAIT,
    );
    tui.key('g');
    tui.wait_pred(
        |screen| doctor_filled_with(screen, "Terminal size"),
        "g goes back to the first row",
        WAIT,
    );
    tui.wait_pred(
        |screen| cursor_on(screen, "Terminal size"),
        "g puts the cursor on the first row",
        WAIT,
    );
    // git is the first TOOLS row, after the thirteen TERMINAL rows on Unix
    // (seven probe rows and six capability rows). Each j waits for the
    // cursor to move before the next: the event loop drops a queued copy
    // of a held nav key, so two j presses that arrive in one read move once.
    for _ in 0..13 {
        let before = cursor_row(&tui.screen());
        tui.key('j');
        tui.wait_pred(
            |screen| cursor_row(screen).is_some_and(|row| Some(row) != before),
            "j moves the Doctor cursor one row",
            WAIT,
        );
    }
    tui.wait_pred(
        |screen| doctor_filled_with(screen, "git") && cursor_on(screen, "git"),
        "thirteen j moves put the cursor on the git row",
        WAIT,
    );

    // Not a re-run check (the rows are already filled): it checks that the
    // app stays up and the rows and the cursor survive `r`. The unit tests
    // cover the re-run itself.
    tui.key('r');
    tui.wait_pred(
        |screen| doctor_filled_with(screen, "git") && cursor_on(screen, "git"),
        "after r the rows are still painted and the cursor is still on git",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains(TITLE) && !has_summary(screen),
        "Esc closes the Doctor",
        WAIT,
    );
    tui.assert_running("after the Doctor closes");
    assert!(
        !ui_store.exists() && dir_entries(&ui_dir).is_empty(),
        "opening, moving, re-running and closing the Doctor writes no UI-state store: {:?}",
        dir_entries(&ui_dir)
    );
}

/// Rows of the tall terminal the capability tests use, so every TERMINAL
/// row and its text fit in the dialog without scrolling.
const TALL_ROWS: u16 = 80;

/// A whole answer to the capability query, as one terminal write:
/// XTVERSION, DECRPM 1006 → 2, 2004 → 2, 2026 → 0, kitty flags 1, OSC 11
/// black, then DA1. The text holds `r`, `k`, `g`, `m`, `y`, and `c`, which
/// are Doctor or app keys if a reply byte were read as a key.
const FAKE_TERM_ANSWER: &[u8] = b"\x1bP>|fake-term(1.2)\x1b\\\
\x1b[?1006;2$y\x1b[?2004;2$y\x1b[?2026;0$y\
\x1b[?1u\
\x1b]11;rgb:0000/0000/0000\x1b\\\
\x1b[?62c";

/// True when the child wrote `text` after its `nth` capability query
/// (1-based).
fn painted_after_query(tui: &PtySession, nth: usize, text: &[u8]) -> bool {
    let raw = tui.raw_output();
    byte_positions(&raw, CAPS_QUERY)
        .get(nth - 1)
        .is_some_and(|at| find_bytes(&raw, text, *at).is_some())
}

/// Open the Doctor from the palette.
fn open_doctor(tui: &mut PtySession) {
    open_command_palette(tui);
    type_palette_filter(tui, "doctor");
    tui.wait_contains(PALETTE_ROW, WAIT);
    tui.enter();
}

/// The open Doctor shows the fake terminal's answers.
fn shows_fake_term_answers(screen: &str) -> bool {
    doctor_filled_with(screen, "Terminal (XTVERSION)")
        && has_row(screen, "Background (OSC 11)")
        && [
            "fake-term(1.2)",
            "Not supported (optional): a large redraw can flicker.",
            "Supported (flags 1).",
            "#000000, dark.",
        ]
        .iter()
        .all(|text| screen.contains(text))
}

/// A terminal that answers the capability query: the TERMINAL rows show
/// its answers, the query is written once per request (open, then `r`),
/// and no reply byte acts as a key. Fail if a row misses an answer, the
/// query is written again on later frames, or the Doctor cursor moves,
/// the Doctor closes, or a fix line is copied while the replies land.
#[test]
fn pty_doctor_shows_the_terminal_answers_and_reads_no_reply_as_keys() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size_with_caps_reply(
        &workspace,
        COLS,
        TALL_ROWS,
        &[("WS_STATUS_GLYPHS", "ascii")],
        FAKE_TERM_ANSWER,
    );
    assert_eq!(
        tui.raw_count(CAPS_QUERY),
        0,
        "no query before the Doctor opens"
    );
    open_doctor(&mut tui);
    let screen = tui.wait_screen(
        shows_fake_term_answers,
        "the Doctor shows the fake terminal's answers",
        WAIT,
    );
    assert!(
        has_row(&screen, "SGR mouse (1006)")
            && has_row(&screen, "Bracketed paste (2004)")
            && has_row(&screen, "Kitty keyboard (CSI u)"),
        "every capability row paints:\n{screen}"
    );
    assert!(
        cursor_on(&screen, "Terminal size"),
        "the replies moved no cursor:\n{screen}"
    );
    assert_eq!(tui.raw_count(CAPS_QUERY), 1, "one query for the open");

    // Move off the first row, so a reply byte read as `k` or `g` would
    // move the cursor back up.
    for _ in 0..2 {
        let before = cursor_row(&tui.screen());
        tui.key('j');
        tui.wait_pred(
            |screen| cursor_row(screen).is_some_and(|row| Some(row) != before),
            "j moves the Doctor cursor one row",
            WAIT,
        );
    }
    let focused = cursor_row(&tui.screen()).expect("cursor row");
    tui.key('r');
    // The second query goes out in the `checking…` frame, so the refilled
    // rows are the ones painted after it: a screen that still shows the
    // first answer must not pass.
    // The asserts read the screen that matched, not a later one.
    let screen = tui.wait_screen(
        |screen| painted_after_query(&tui, 2, b"fake-term(1.2)") && shows_fake_term_answers(screen),
        "r writes the query again and the rows show the answers again",
        WAIT,
    );
    assert_eq!(
        cursor_row(&screen).as_deref(),
        Some(focused.as_str()),
        "the replies to the second query moved no cursor:\n{screen}"
    );
    assert!(screen.contains(TITLE), "the Doctor stays open:\n{screen}");
    assert!(
        tui.clipboard_payloads().is_empty(),
        "no reply byte copied a fix line"
    );
    assert_eq!(tui.raw_count(CAPS_QUERY), 2, "one query per request");
    tui.assert_running("after the replies");
}

/// A terminal that answers nothing, not even DA1: after the 200 ms wait the
/// capability rows show unknown with the wait in their text, and the head
/// shows four counts. Fail if the Doctor stays `checking…` or a capability
/// row shows anything but unknown.
#[test]
fn pty_doctor_capability_rows_are_unknown_when_the_terminal_does_not_answer() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size_with_env(
        &workspace,
        COLS,
        TALL_ROWS,
        &[("WS_STATUS_GLYPHS", "ascii")],
    );
    open_doctor(&mut tui);
    let screen = tui.wait_screen(
        |screen| {
            doctor_filled_with(screen, "Terminal (XTVERSION)")
                && screen.contains("The terminal did not report its name and version.")
                && screen.contains("(no answer within 200 ms)")
        },
        "the capability rows show unknown after the wait",
        WAIT,
    );
    for name in [
        "Terminal (XTVERSION)",
        "SGR mouse (1006)",
        "Bracketed paste (2004)",
        "Synchronized output (2026)",
        "Kitty keyboard (CSI u)",
        "Background (OSC 11)",
    ] {
        assert!(
            screen.contains(&format!("?    {name}")),
            "{name} is unknown:\n{screen}"
        );
    }
    assert!(has_summary(&screen), "four counts in the head:\n{screen}");
    assert!(
        cursor_on(&screen, "Terminal size"),
        "the cursor stays on the first row:\n{screen}"
    );
    assert_eq!(tui.raw_count(CAPS_QUERY), 1, "the query is written once");
}
