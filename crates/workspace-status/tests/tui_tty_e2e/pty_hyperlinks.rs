//! OSC 8 hyperlinks on file paths, read from the raw PTY bytes (`vt100`
//! drops OSC 8 from the screen model).

use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// A file row whose name has a space links to its `file://` URL, the space
/// percent-encoded and the quotes git puts around that name left out.
#[test]
fn pty_file_row_emits_osc8_file_link() {
    let (_root, workspace) = daily_workspace();
    fs::write(workspace.join("app").join("my notes.txt"), "note\n").unwrap();
    let tui = PtySession::open(&workspace);
    tui.wait_contains("my notes.txt", WAIT);
    let link_url = || {
        let raw = String::from_utf8_lossy(&tui.raw_output()).into_owned();
        raw.split("\x1b]8;;")
            .skip(1)
            .filter_map(|tail| tail.split_once("\x1b\\").map(|(url, _)| url.to_string()))
            .find(|url| {
                url.starts_with("file://")
                    && url.contains("/app/my%20notes.txt")
                    && !url.contains("%22")
            })
    };
    tui.wait_pred(
        |_| link_url().is_some(),
        "OSC 8 file link to app/my notes.txt",
        WAIT,
    );
    let url = link_url().unwrap();
    assert!(
        url.bytes().all(|b| (0x21..=0x7E).contains(&b)),
        "printable ASCII only: {url:?}"
    );
}
