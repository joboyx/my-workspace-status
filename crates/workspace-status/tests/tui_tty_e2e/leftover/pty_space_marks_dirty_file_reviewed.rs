use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_space_reviewed, idle_dirty_readme_unreviewed, GIT_WAIT, WAIT};

/// Tokyo Night viewed token (`palette.viewed`). Distinct from muted.
const TOKYO_VIEWED: (u8, u8, u8) = (0x73, 0xda, 0xca);

/// Space on a dirty file paints the ASCII reviewed mark (`*`).
///
/// Docs: Space reviewed (`*` ASCII). Help / keymap: `space` / "mark dirty
/// file reviewed (eye)". Configuration: trailing eye, teal/cyan viewed
/// token, then Space again unmarks while contents are unchanged. Live PTY
/// after first paint did that toggle. The `*` cell fg is Tokyo Night
/// viewed, not muted. Not `z` fold (`pty_z_folds_focused_repo`). Not
/// `s`/`u` stage. Not graph-focus overlay Space (`[x]`).
///
/// After first paint the cursor is already on the dirty README (not a
/// repo row). Do not `/` search. A no-op, a cursor-only move, a fold that
/// hides the file, a stage, or a `*` that never lands on that row is red.
#[test]
fn pty_space_marks_dirty_file_reviewed() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "first paint: cursor on dirty README, no reviewed mark",
        WAIT,
    );
    tui.wait_pred(
        |_| {
            !tui.glyph_after_needle_has_fg(
                "README.md",
                '*',
                TOKYO_VIEWED.0,
                TOKYO_VIEWED.1,
                TOKYO_VIEWED.2,
            )
        },
        "first paint: no viewed-colour `*` after README.md",
        WAIT,
    );

    tui.key(' ');
    tui.wait_pred(
        documented_space_reviewed,
        "Space paints ASCII `*` on the focused README row; file stays; not staged",
        WAIT,
    );
    tui.wait_pred(
        |_| {
            tui.glyph_after_needle_has_fg(
                "README.md",
                '*',
                TOKYO_VIEWED.0,
                TOKYO_VIEWED.1,
                TOKYO_VIEWED.2,
            )
        },
        "reviewed `*` uses Tokyo Night viewed teal, not muted",
        WAIT,
    );

    tui.key(' ');
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "second Space clears the reviewed mark (documented toggle)",
        WAIT,
    );
}
