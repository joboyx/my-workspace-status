use std::path::Path;
use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::{compare_regions_workspace, git_stdout};
use crate::support::{
    compare_regions_diff_focused, crumb_line, open_compare_regions_diff, status_line,
    type_palette_filter, SETTLE_MS, VISUAL_KEY_GAP_MS, WAIT,
};

/// Default theme `palette.cursor_bg`: the focused diff cursor row and every
/// row inside a `V` range paint it.
const CURSOR_BG: (u8, u8, u8) = (0x28, 0x34, 0x57);

const CANNOT_STAGE: &str = "cannot stage a compare diff";
const CANNOT_UNSTAGE: &str = "cannot unstage a compare diff";
const SWITCH: &str = "Switch to Workspace tab";

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

/// `V` highlight is on the compare regions.txt diff (VISUAL hint row).
fn compare_highlight_active(screen: &str) -> bool {
    screen.contains("app ↔ origin/main")
        && screen.contains("VISUAL")
        && screen.contains("cancel highlight")
        && !palette_open(screen)
}

/// The diff cursor bar (`▌`) sits on the row that shows `needle`.
fn diff_cursor_on(screen: &str, needle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains(needle) && line.contains('▌'))
}

fn has_cursor_bg(tui: &PtySession, needle: &str) -> bool {
    tui.needle_has_bg(needle, CURSOR_BG.0, CURSOR_BG.1, CURSOR_BG.2)
}

fn lacks_cursor_bg(tui: &PtySession, needle: &str) -> bool {
    tui.needle_lacks_bg(needle, CURSOR_BG.0, CURSOR_BG.1, CURSOR_BG.2)
}

/// Poll a paint claim that `screen()` text cannot express (cell colours).
fn wait_paint(tui: &PtySession, pred: impl Fn(&PtySession) -> bool, what: &str) {
    let start = Instant::now();
    while !pred(tui) {
        if start.elapsed() >= WAIT {
            panic!("timeout waiting for {what}:\n{}", tui.screen());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Plain `j` presses with the human gap the held-nav backlog needs.
fn press_j(tui: &mut PtySession, times: usize) {
    for _ in 0..times {
        tui.letter_press('j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
}

fn start_highlight(tui: &mut PtySession) {
    tui.shift_letter('V');
    tui.wait_pred(compare_highlight_active, "Shift+V paints VISUAL", WAIT);
    tui.wait_ms(VISUAL_KEY_GAP_MS);
}

/// `git status --porcelain` in the compare checkout (index and worktree oracle).
fn porcelain(workspace: &Path) -> String {
    git_stdout(&workspace.join("app"), &["status", "--porcelain"])
}

/// `V`, `j`, Esc, and a second `V` on a compare diff.
///
/// The cursor starts on the COMMITTED header. `V` there and three `j`
/// presses put the cursor on keep-b: keep-a (inside the range, not the
/// cursor) paints the highlight bg. Esc clears it and keeps the cursor.
/// `V` again anchors on keep-b; `j` to keep-c paints keep-b; a second `V`
/// leaves highlight.
#[test]
fn pty_compare_v_highlights_extends_and_cancels() {
    let (_root, workspace) = compare_regions_workspace();
    let mut tui = open_compare_regions_diff(&workspace);
    assert!(
        lacks_cursor_bg(&tui, "keep-a"),
        "no highlight before V:\n{}",
        tui.screen()
    );

    start_highlight(&mut tui);
    press_j(&mut tui, 3);
    tui.wait_pred(
        |screen| compare_highlight_active(screen) && diff_cursor_on(screen, "keep-b"),
        "j extends the compare highlight to keep-b",
        WAIT,
    );
    wait_paint(
        &tui,
        |tui| has_cursor_bg(tui, "keep-a") && has_cursor_bg(tui, "keep-b"),
        "keep-a (inside the range) paints the highlight bg",
    );
    assert!(
        !status_line(&tui.screen()).contains(SWITCH),
        "V is not refused on a compare tab:\n{}",
        tui.screen()
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            compare_regions_diff_focused(screen)
                && !screen.contains("VISUAL")
                && diff_cursor_on(screen, "keep-b")
        },
        "Esc leaves highlight, keeps the tab, the diff focus, and the cursor",
        WAIT,
    );
    wait_paint(
        &tui,
        |tui| lacks_cursor_bg(tui, "keep-a"),
        "Esc clears the keep-a highlight bg",
    );

    start_highlight(&mut tui);
    press_j(&mut tui, 1);
    tui.wait_pred(
        |screen| compare_highlight_active(screen) && diff_cursor_on(screen, "keep-c"),
        "j extends the second highlight to keep-c",
        WAIT,
    );
    wait_paint(
        &tui,
        |tui| has_cursor_bg(tui, "keep-b"),
        "keep-b (the anchor) paints the highlight bg",
    );
    tui.shift_letter('V');
    tui.wait_pred(
        |screen| compare_regions_diff_focused(screen) && !screen.contains("VISUAL"),
        "a second V leaves highlight",
        WAIT,
    );
    wait_paint(
        &tui,
        |tui| lacks_cursor_bg(tui, "keep-b"),
        "V toggles the keep-b highlight bg off",
    );
}

/// `V`, `j`, `'` on a compare diff copies a range reference.
///
/// Two `j` presses put the cursor on keep-a (new line 1). `V` then two `j`
/// presses cover lines 1-3. The OSC 52 payload names that range and the
/// compare source.
#[test]
fn pty_compare_v_apostrophe_copies_range_reference() {
    let (_root, workspace) = compare_regions_workspace();
    let mut tui = open_compare_regions_diff(&workspace);
    press_j(&mut tui, 2);
    tui.wait_pred(
        |screen| diff_cursor_on(screen, "keep-a"),
        "two j presses move the compare diff cursor to keep-a",
        WAIT,
    );
    start_highlight(&mut tui);
    press_j(&mut tui, 2);
    tui.wait_pred(
        |screen| compare_highlight_active(screen) && diff_cursor_on(screen, "keep-c"),
        "j extends the highlight to keep-c",
        WAIT,
    );
    assert!(
        tui.clipboard_payloads().is_empty(),
        "no copy before ':\n{}",
        tui.screen()
    );

    tui.key('\'');
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains("kind: diff")
                    && text.contains("path: regions.txt")
                    && text.contains("lines: 1-3")
                    && text.contains("source: compare origin/main...")
            })
        },
        "OSC 52 payload has lines: 1-3 and source: compare origin/main...<head>",
        WAIT,
    );
    tui.wait_pred(
        |screen| screen.contains("copied") && !screen.contains(SWITCH),
        "' flashes copied, not the compare refusal",
        WAIT,
    );
}

/// Palette and `s` / `u` while lines are highlighted on a compare diff.
///
/// `:` shows the HIGHLIGHT rows. Stage / Unstage highlighted lines are
/// disabled and name the compare reason. On this clean checkout at the
/// compare head, Revert highlighted lines (to the merge base) is the first
/// enabled row, so the cursor lands there. `s` and `u` in the
/// highlight give the same reason and leave the index untouched.
#[test]
fn pty_compare_highlight_palette_refuses_stage_and_unstage() {
    let (_root, workspace) = compare_regions_workspace();
    assert_eq!(porcelain(&workspace), "", "seed worktree is clean");
    let mut tui = open_compare_regions_diff(&workspace);
    press_j(&mut tui, 5);
    start_highlight(&mut tui);
    press_j(&mut tui, 1);
    tui.wait_pred(
        |screen| compare_highlight_active(screen) && diff_cursor_on(screen, "ALPHA-NEW"),
        "the highlight covers ALPHA-OLD and ALPHA-NEW",
        WAIT,
    );

    tui.key(':');
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("HIGHLIGHT")
                && screen.contains("Stage highlighted lines")
                && screen.contains("Unstage highlighted lines")
                && screen.contains("Revert highlighted lines")
                && screen.contains("❯ Revert highlighted lines")
                && screen.contains("Exit highlight")
        },
        "`:` shows the HIGHLIGHT rows with the cursor on Revert highlighted lines",
        WAIT,
    );
    type_palette_filter(&mut tui, "stage highlighted");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("❯ Stage highlighted lines")
                && screen.contains(CANNOT_STAGE)
        },
        "Stage highlighted lines shows the compare reason",
        WAIT,
    );
    tui.enter();
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains(CANNOT_STAGE),
        "Enter on the disabled row keeps the palette and the reason",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        compare_highlight_active,
        "Esc closes the palette and keeps the highlight",
        WAIT,
    );

    tui.key(':');
    tui.wait_pred(palette_open, "`:` reopens the palette", WAIT);
    type_palette_filter(&mut tui, "unstage highlighted");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("❯ Unstage highlighted lines")
                && screen.contains(CANNOT_UNSTAGE)
        },
        "Unstage highlighted lines shows the compare reason",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(compare_highlight_active, "Esc closes the palette", WAIT);

    tui.letter_press('s');
    tui.wait_pred(
        |screen| compare_highlight_active(screen) && screen.contains(CANNOT_STAGE),
        "s in the highlight shows the compare stage reason",
        WAIT,
    );
    tui.wait_ms(VISUAL_KEY_GAP_MS);
    tui.letter_press('u');
    tui.wait_pred(
        |screen| compare_highlight_active(screen) && screen.contains(CANNOT_UNSTAGE),
        "u in the highlight shows the compare unstage reason",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert_eq!(
        porcelain(&workspace),
        "",
        "s / u on a compare highlight must not touch the index or worktree"
    );
}

/// Fetch and the stash menu stay refused on a compare tab.
///
/// The palette Fetch remotes row names the switch reason. `S` on the
/// compare diff puts it on the status line and the stash menu does not
/// open. `f` then leaves the checkout's remote-tracking refs as they were.
#[test]
fn pty_compare_fetch_and_stash_menu_stay_refused() {
    let (_root, workspace) = compare_regions_workspace();
    let repo = workspace.join("app");
    let origin = workspace.join("app.origin.git");
    // Advance main on the remote only, so a fetch that ran would move origin/main.
    let remote_tip = git_stdout(
        &origin,
        &[
            "commit-tree",
            "main^{tree}",
            "-p",
            "main",
            "-m",
            "remote only",
        ],
    );
    git_stdout(&origin, &["update-ref", "refs/heads/main", &remote_tip]);
    let tracking_before = git_stdout(&repo, &["rev-parse", "origin/main"]);
    let origin_main = git_stdout(&origin, &["rev-parse", "main"]);
    assert_ne!(
        tracking_before, origin_main,
        "origin moved past origin/main"
    );

    let mut tui = open_compare_regions_diff(&workspace);
    tui.key(':');
    tui.wait_pred(palette_open, "`:` opens the palette", WAIT);
    type_palette_filter(&mut tui, "fetch");
    tui.wait_pred(
        |screen| {
            palette_open(screen) && screen.contains("❯ Fetch remotes") && screen.contains(SWITCH)
        },
        "Fetch remotes shows the switch reason in the palette",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| compare_regions_diff_focused(screen) && !screen.contains(SWITCH),
        "Esc closes the palette with no reason left on the status line",
        WAIT,
    );

    tui.shift_letter('S');
    tui.wait_pred(
        |screen| compare_regions_diff_focused(screen) && status_line_or_crumb_has(screen, SWITCH),
        "S on a compare tab shows the switch reason",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert!(
        !tui.screen().contains('╭'),
        "S must not open the stash menu:\n{}",
        tui.screen()
    );

    tui.letter_press('f');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| compare_regions_diff_focused(screen) && screen.contains(SWITCH),
        "f on a compare tab keeps the switch reason",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "origin/main"]),
        tracking_before,
        "f on a compare tab must not fetch"
    );
}

/// The reason sits at the right of the breadcrumb row above the key hints.
fn status_line_or_crumb_has(screen: &str, needle: &str) -> bool {
    status_line(screen).contains(needle) || crumb_line(screen).contains(needle)
}
