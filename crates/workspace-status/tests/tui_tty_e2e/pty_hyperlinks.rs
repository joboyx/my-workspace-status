//! OSC 8 hyperlinks read from the raw PTY bytes (`vt100` drops OSC 8 from
//! the screen model).

use std::fs;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git, git_stdout};
use crate::support::{
    crumb_line, graph_cursor_on, links_since, panes_tree_unfocused_graph_focused, title_has_diff,
    title_has_files, tree_cursor_on, GIT_WAIT, WAIT,
};

/// GitHub-form origin of `app`. The `.invalid` host keeps the PR lookup
/// off the network; the lookup still caches the remote for commit links.
const APP_REMOTE: &str = "https://github.invalid/octo/demo.git";
/// Subject of the `app` commit that adds `my notes.txt`.
const NOTES_COMMIT: &str = "notes-link";

/// A diff pane title paints `app/my notes.txt`, or `app/"my notes.txt"`
/// when git C-quotes the path.
fn notes_diff_title(screen: &str) -> bool {
    screen.contains("app/my notes.txt") || screen.contains("app/\"my notes.txt\"")
}

/// No frame links a file name: the tree row, the working-tree diff title,
/// the commit-files row, and the commit diff title of `my notes.txt` paint
/// with no `file://` OSC 8 link.
///
/// Positive control: a commit hash link (graph row or commit-files footer)
/// to the forge commit page. One shows up after the commit-files list
/// opens, and one after a frame that follows the commit diff title frame
/// (the diff title is gone), so the OSC 8 writer is live on this PTY and
/// those frames were flushed.
#[test]
fn pty_emits_no_osc8_file_link() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    git(&app, &["remote", "add", "origin", APP_REMOTE]);
    fs::write(app.join("my notes.txt"), "note\n").unwrap();
    git(&app, &["add", "my notes.txt"]);
    git(&app, &["commit", "-q", "-m", NOTES_COMMIT]);
    let sha = git_stdout(&app, &["rev-parse", "HEAD"]).trim().to_string();
    let commit_url = format!("https://github.invalid/octo/demo/commit/{sha}");
    // Dirty again so the tree lists the file too.
    fs::write(app.join("my notes.txt"), "note\nmore\n").unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("my notes.txt", WAIT);

    // Tree row and working-tree diff title.
    tui.search("my notes");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "my notes.txt") && notes_diff_title(screen),
        "working-tree diff of app/my notes.txt",
        GIT_WAIT,
    );

    // Commit-files list of the notes commit (file row + footer hash).
    // The app row sits right above its first file row.
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && screen.contains(NOTES_COMMIT),
        "k moves onto app and loads its graph",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        panes_tree_unfocused_graph_focused,
        "Enter focuses the app graph",
        WAIT,
    );
    tui.search(NOTES_COMMIT);
    tui.wait_pred(
        |screen| graph_cursor_on(screen, NOTES_COMMIT),
        "search selects the notes commit",
        GIT_WAIT,
    );
    let before_files = tui.raw_output().len();
    tui.enter();
    tui.wait_pred(
        |screen| {
            title_has_files(screen)
                && screen.contains("my notes.txt")
                && links_since(&tui, before_files)
                    .iter()
                    .any(|(url, _)| *url == commit_url)
        },
        "commit-files list with my notes.txt and a commit hash link",
        GIT_WAIT,
    );

    // Commit diff title.
    tui.enter();
    tui.wait_pred(
        |screen| title_has_diff(screen) && title_has_files(screen) && notes_diff_title(screen),
        "commit diff of my notes.txt",
        GIT_WAIT,
    );
    // Leave it in steps, so the control link comes from a frame after the
    // one that painted the diff title: Esc clears the armed graph search,
    // Esc focuses the file list (the title frame is flushed by now), and a
    // last Esc pops the diff.
    tui.esc();
    tui.wait_pred(
        |screen| crumb_line(screen).contains("search cleared"),
        "Esc clears the armed graph search",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            let crumb = crumb_line(screen);
            crumb.contains("› my notes.txt") && !crumb.contains('[')
        },
        "Esc focuses the commit file list",
        WAIT,
    );
    let files_focused = tui.raw_output().len();
    tui.esc();
    tui.wait_pred(
        |screen| {
            !notes_diff_title(screen)
                && links_since(&tui, files_focused)
                    .iter()
                    .any(|(url, _)| *url == commit_url)
        },
        "Esc pops the commit diff; a commit hash link paints after it",
        WAIT,
    );

    let links = links_since(&tui, 0);
    let files: Vec<_> = links
        .iter()
        .filter(|(url, _)| url.starts_with("file://"))
        .collect();
    assert!(files.is_empty(), "file links: {files:?}");
    let hash: Vec<_> = links.iter().filter(|(url, _)| *url == commit_url).collect();
    assert!(
        hash.iter().all(|(_, text)| sha.starts_with(text.trim())),
        "commit link wraps the short hash: {hash:?}"
    );
}
