use crate::harness::PtySession;
use crate::seed::{word_and_space_change_workspace, WORD_SPACE_NEW};
use crate::support::{after_ack, assert_word_bgs, right_pane, tree_cursor_on, WAIT};

/// Part of the `+` line with the word change and the doubled space.
const ADD_NEEDLE: &str = "is two words, also  extra";
/// The same part of the `-` line.
const DEL_NEEDLE: &str = "is one word, also extra";

/// Word background (`#`) or row background (any other char) of each cell
/// of [`ADD_NEEDLE`] / [`DEL_NEEDLE`], whitespace shown and hidden.
const ADD_SHOWN: &str = "...###.#####......##.....";
const ADD_HIDDEN: &str = "...###.#####.............";
const DEL_SHOWN: &str = "...###.####......#.....";
const DEL_HIDDEN: &str = "...###.####............";

/// The first right-pane row: the diff header (`{path}  inline|split · …`).
fn diff_header(screen: &str) -> String {
    right_pane(screen).lines().next().unwrap_or("").to_string()
}

/// `notes.txt` focused, wrap on, both needles painted, and the `+` line
/// wrapped (no row holds the whole line). `dash_w` is the ` · -w` mark.
fn word_space_diff(dash_w: bool) -> impl Fn(&str) -> bool {
    move |screen| {
        let header = diff_header(screen);
        let right = right_pane(screen);
        tree_cursor_on(screen, "notes.txt")
            && header.contains("notes.txt")
            && header.contains("· wrap")
            && header.contains("· -w") == dash_w
            && right.contains(ADD_NEEDLE)
            && right.contains(DEL_NEEDLE)
            && !right.lines().any(|row| row.contains(WORD_SPACE_NEW))
    }
}

/// [`assert_word_bgs`] on this test's needles. The `-w` reload anchors the
/// (unfocused) diff cursor on the `-` line, which tints its backgrounds.
fn assert_needle_bgs(tui: &PtySession, add_mask: &str, del_mask: &str, when: &str) {
    assert_word_bgs(tui, (ADD_NEEDLE, add_mask), (DEL_NEEDLE, del_mask), when);
}

/// While `-w` is on, the word highlight skips whitespace.
///
/// `notes.txt` changes `one word` to `two words` and doubles the space
/// after `also` on one long line. Wrap is on at launch, so the line wraps.
/// With whitespace shown, the changed words and both sides of the space
/// change sit on the word background. `w` reloads the diff with `-w`: the
/// line still shows (it has a real change), the changed words keep the
/// word background, and the doubled space and its single-space partner go
/// back to the row background. A highlight that still marks the space, or
/// a reload that drops the word highlight, cannot pass.
#[test]
fn pty_w_skips_whitespace_in_the_word_highlight() {
    let (_root, workspace) = word_and_space_change_workspace("ws-tui-tty-w-word-highlight");
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        word_space_diff(false),
        "launch shows the wrapped notes.txt diff without -w",
        WAIT,
    );
    assert_needle_bgs(&tui, ADD_SHOWN, DEL_SHOWN, "whitespace shown");

    tui.key('w');
    tui.wait_pred(
        after_ack("whitespace hidden", word_space_diff(true)),
        "w reloads notes.txt with -w; the real change still shows",
        WAIT,
    );
    assert_needle_bgs(&tui, ADD_HIDDEN, DEL_HIDDEN, "whitespace hidden");
}
