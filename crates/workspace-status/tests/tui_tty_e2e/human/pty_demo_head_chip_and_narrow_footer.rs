use std::path::{Path, PathBuf};
use std::process::Command;

use crate::harness::{PtySession, ROWS};
use crate::seed::unique_root;
use crate::support::{
    graph_pane_focused, right_pane, title_has_files, title_has_graph, tree_cursor_on, GIT_WAIT,
    SETTLE_MS, WAIT,
};

const FULL_REF: &str = "feature/reconciliation";
const ASCII_MERGED_CHIP: &str = "[+=feature/reconciliation]";

fn seed_demo_workspace(dest: &Path) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/seed-demo-workspace.sh");
    let status = Command::new("bash")
        .arg(&script)
        .arg(dest)
        .status()
        .expect("seed script runs");
    assert!(status.success(), "seed-demo-workspace.sh failed");
}

fn open_demo_pty() -> PtySession {
    let root = unique_root("ws-tui-demo");
    let dest = root.join("workspace");
    seed_demo_workspace(&dest);
    assert!(
        dest.join("merger/.worktrees/recon").is_dir(),
        "seed must include a linked worktree on the current branch"
    );
    PtySession::open(&dest)
}

fn first_bracket_chip(line: &str) -> &str {
    let start = line.find('[').unwrap_or(0);
    let Some(rel_end) = line[start..].find(']') else {
        return "";
    };
    &line[start..=start + rel_end]
}

/// Fully hidden leftover count (`[+N]`). ASCII checkout `[+branch]` / `[+=branch]`
/// also starts with `[+`, so a raw substring is not the overflow chip.
fn has_overflow_plus_n(line: &str) -> bool {
    line.split("[+").any(|rest| {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        !digits.is_empty() && rest[digits.len()..].starts_with(']')
    })
}

fn demo_launch_idle(screen: &str) -> bool {
    screen.contains("merger")
        && (screen.contains("auth.ts") || screen.contains("src/auth.ts"))
        && screen.contains("? help")
        && !screen.contains("SEARCH")
}

fn demo_user_chip_spacer(screen: &str) -> Option<&str> {
    screen
        .lines()
        .find(|line| line.contains(ASCII_MERGED_CHIP) && line.contains("Demo User"))
}

fn merged_chip_footer(screen: &str) -> Option<&str> {
    screen
        .lines()
        .find(|line| line.contains(ASCII_MERGED_CHIP) && !line.contains("Demo User"))
}

/// Right pane is the demo merger graph. Files drill cannot pass.
fn demo_merger_graph_body(screen: &str) -> bool {
    title_has_graph(screen)
        && !title_has_files(screen)
        && screen.contains(FULL_REF)
        && screen.contains(ASCII_MERGED_CHIP)
        && demo_user_chip_spacer(screen).is_some()
        && !screen.contains("SEARCH")
}

fn demo_merger_graph_left(screen: &str) -> bool {
    tree_cursor_on(screen, "merger")
        && !graph_pane_focused(screen)
        && demo_merger_graph_body(screen)
}

fn merged_head_chip_matches_footer(screen: &str) -> bool {
    if !screen.contains(ASCII_MERGED_CHIP) {
        return false;
    }
    let Some(spacer) = demo_user_chip_spacer(screen) else {
        return false;
    };
    let Some(footer) = merged_chip_footer(screen) else {
        return false;
    };
    first_bracket_chip(spacer) == first_bracket_chip(footer)
        && !spacer.contains(".worktrees")
        && !spacer.contains("[+]")
        && !spacer.contains("[=]")
}

fn painted_merged_head_on_left_graph(screen: &str) -> bool {
    demo_merger_graph_left(screen) && merged_head_chip_matches_footer(screen)
}

fn painted_shorter_than_launch(screen: &str) -> bool {
    screen.lines().count() < usize::from(ROWS)
}

/// Selection-footer chip. Not the unbracketed `format_sync` header.
fn narrow_footer_line(screen: &str) -> Option<&str> {
    if let Some(line) = merged_chip_footer(screen) {
        return Some(line);
    }
    screen.lines().find(|line| {
        !line.contains("Demo User")
            && line.contains(FULL_REF)
            && line.contains('[')
            && footer_line_is_chip(line)
    })
}

fn footer_line_is_chip(line: &str) -> bool {
    let chip = first_bracket_chip(line);
    chip.contains(FULL_REF) || (line.contains('…') && line.contains("…]"))
}

fn narrow_spacer_line(screen: &str) -> Option<&str> {
    screen
        .lines()
        .find(|line| line.contains("Demo User") && line.contains('['))
}

fn narrow_footer_keeps_full_ref(screen: &str) -> bool {
    let Some(footer) = narrow_footer_line(screen) else {
        return false;
    };
    if has_overflow_plus_n(footer) {
        return false;
    }
    let Some(spacer) = narrow_spacer_line(screen) else {
        return false;
    };
    let has_full_name = spacer.contains(FULL_REF);
    let truncated = spacer.contains('…') && spacer.contains("…]");
    if !has_full_name && !truncated {
        return false;
    }
    if truncated && !has_full_name && has_overflow_plus_n(spacer) {
        return false;
    }
    true
}

fn narrow_graph_footer_on_short_grid(screen: &str) -> bool {
    painted_shorter_than_launch(screen)
        && graph_pane_focused(screen)
        && !title_has_files(screen)
        && narrow_footer_keeps_full_ref(screen)
}

fn land_demo_merger_graph() -> PtySession {
    let mut tui = open_demo_pty();
    tui.wait_pred(
        demo_launch_idle,
        "demo first paint: merger on the tree, auth.ts, idle chrome",
        WAIT,
    );
    assert!(
        !merged_head_chip_matches_footer(&tui.screen()),
        "merged chip claim must be false before search:\n{}",
        tui.screen()
    );
    tui.search("merger");
    tui.wait_pred(
        painted_merged_head_on_left_graph,
        "search merger loads the demo graph; tree stays focused; HEAD chip matches footer",
        GIT_WAIT,
    );
    tui
}

/// Demo merger HEAD chip matches the selection footer on the painted row.
///
/// `/` `merger` Enter loads the demo graph. ASCII paint is one bracket
/// pair `[+=feature/reconciliation]` (`+` checkout, `=` sync). The commit
/// spacer chip equals the footer chip. A second `.worktrees` chip or
/// split `[+]` / `[=]` marks cannot pass. Launch file-diff must not
/// already satisfy the claim.
///
/// MYWS-005.
#[test]
fn pty_demo_merged_head_chip_matches_footer_on_painted_row() {
    let tui = land_demo_merger_graph();
    tui.wait_pred(
        painted_merged_head_on_left_graph,
        "merged HEAD chip matches the footer on the painted spacer",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        painted_merged_head_on_left_graph,
        "merged HEAD chip holds (not a flicker or launch file-diff)",
        WAIT,
    );
}

/// Narrow demo graph truncates the spacer chip; the footer keeps the full ref.
///
/// Same seed and `/` `merger` Enter as the merged-chip test, then Tab
/// focuses the graph and a 64×28 resize clips the spacer. The footer
/// still lists `feature/reconciliation` and does not collapse leftover
/// refs to `[+N]`. `l` pans the graph and leaves the full ref in the
/// footer.
///
/// MYWS-005.
#[test]
fn pty_demo_narrow_graph_truncates_chip_name_footer_keeps_full_ref() {
    let mut tui = land_demo_merger_graph();
    assert!(
        !graph_pane_focused(&tui.screen()),
        "graph-focused claim must be false before Tab:\n{}",
        tui.screen()
    );
    tui.tab();
    tui.wait_pred(
        graph_pane_focused,
        "Tab moves focus to the demo merger graph",
        WAIT,
    );

    assert!(
        !narrow_graph_footer_on_short_grid(&tui.screen()),
        "narrow-footer short-grid claim must be false before resize (launch is 140x32):\n{}",
        tui.screen()
    );
    tui.resize(64, 28);
    tui.wait_pred(
        narrow_graph_footer_on_short_grid,
        "64x28 resize keeps the full ref in the footer; spacer keeps a full or truncated chip",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        narrow_graph_footer_on_short_grid,
        "narrow footer holds (not a flicker, same-size no-op, or [+N] collapse)",
        WAIT,
    );

    let before_right = right_pane(&tui.screen());
    assert!(
        tui.screen().contains(FULL_REF),
        "full ref must be on screen before l pan:\n{}",
        tui.screen()
    );
    for _ in 0..24 {
        tui.key('l');
    }
    tui.wait_pred(
        |screen| {
            painted_shorter_than_launch(screen)
                && graph_pane_focused(screen)
                && narrow_footer_line(screen).is_some_and(|footer| !has_overflow_plus_n(footer))
                && right_pane(screen) != before_right
        },
        "l pan keeps the full ref in the footer (graph body moved)",
        WAIT,
    );
}
