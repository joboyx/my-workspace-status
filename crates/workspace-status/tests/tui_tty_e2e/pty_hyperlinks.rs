//! OSC 8 hyperlinks on file paths, read from the raw PTY bytes (`vt100`
//! drops OSC 8 from the screen model).

use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{GIT_WAIT, WAIT};

/// Encoded name of `my notes.txt` in a link URL, quoted or not.
const NOTES_URL_NAME: &str = "my%20notes.txt";

/// Every OSC 8 link in `raw`: its URL and the visible text it wraps, with
/// CSI sequences (cursor moves, colours) removed.
fn osc8_links(raw: &str) -> Vec<(String, String)> {
    raw.split("\x1b]8;;")
        .skip(1)
        .filter_map(|tail| tail.split_once("\x1b\\"))
        .filter(|(url, _)| !url.is_empty())
        .map(|(url, body)| (url.to_string(), strip_csi(body)))
        .collect()
}

/// `s` without its `ESC [ … final` sequences.
fn strip_csi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Links to `app/my notes.txt` in the raw output so far.
fn notes_links(tui: &PtySession) -> Vec<(String, String)> {
    osc8_links(&String::from_utf8_lossy(&tui.raw_output()))
        .into_iter()
        .filter(|(url, _)| url.starts_with("file://") && url.contains(NOTES_URL_NAME))
        .collect()
}

/// A tree file row paints its name with no `file://` link.
///
/// Positive control: the diff pane title of the same file does link
/// (`app/…` text), so the OSC 8 writer is live on this PTY and the frames
/// that painted the tree row are flushed before it. Every link to the file
/// in the whole output must be that title.
#[test]
fn pty_tree_file_row_emits_no_osc8_file_link() {
    let (_root, workspace) = daily_workspace();
    fs::write(workspace.join("app").join("my notes.txt"), "note\n").unwrap();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("my notes.txt", WAIT);
    tui.search("my notes");
    tui.wait_pred(
        |_| {
            notes_links(&tui)
                .iter()
                .any(|(_, text)| text.starts_with("app/"))
        },
        "diff title OSC 8 file link to app/my notes.txt",
        GIT_WAIT,
    );
    let links = notes_links(&tui);
    for (url, _) in &links {
        assert!(!url.contains("%22"), "quotes left out: {url:?}");
        assert!(
            url.bytes().all(|b| (0x21..=0x7E).contains(&b)),
            "printable ASCII only: {url:?}"
        );
    }
    let tree: Vec<_> = links
        .iter()
        .filter(|(_, text)| !text.starts_with("app/"))
        .collect();
    assert!(tree.is_empty(), "tree file row links: {tree:?}");
}
