//! Doctor setup checkup: the palette opens a report-only overlay that the
//! probe fills off the draw thread.

use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{open_command_palette, type_palette_filter, WAIT};

/// Dialog title the overlay paints on its first line.
const TITLE: &str = "Doctor · setup checkup";

/// Palette row that opens the overlay.
const PALETTE_ROW: &str = "Doctor: setup checkup";

/// Status chips, padded to one width as the overlay paints them.
const CHIPS: [&str; 3] = ["ok  ", "warn", "fail"];

/// True when the screen shows the summary line `<n> ok · <n> warn · <n> fail`.
fn has_summary(screen: &str) -> bool {
    screen.lines().any(|line| {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == '│')
            .filter(|word| !word.is_empty())
            .collect();
        words.windows(8).any(|w| {
            w[0].parse::<usize>().is_ok()
                && w[1] == "ok"
                && w[2] == "·"
                && w[3].parse::<usize>().is_ok()
                && w[4] == "warn"
                && w[5] == "·"
                && w[6].parse::<usize>().is_ok()
                && w[7] == "fail"
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
    // git is the first TOOLS row, after the seven TERMINAL rows on Unix.
    // Each j waits for the cursor to move before the next: the input
    // thread drops a queued copy of a held nav key, so two j presses that
    // arrive in one read move once.
    for _ in 0..7 {
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
        "seven j moves put the cursor on the git row",
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
