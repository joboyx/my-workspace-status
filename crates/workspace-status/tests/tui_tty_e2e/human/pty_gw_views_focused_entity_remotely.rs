use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{ahead_workspace, git, git_stdout};
use crate::support::{
    graph_cursor_on, graph_pane_focused, tree_cursor_on, GIT_WAIT, SETTLE_MS, WAIT,
};

/// Forge URL `origin` points at after the seed push. No test reaches it:
/// `gw` reads local refs, and the `gh` / `glab` shims answer badge lookups.
const ORIGIN_URL: &str = "https://github.com/octo/demo.git";
const REPO_PAGE: &str = "https://github.com/octo/demo";

/// Same gap as `pty_workspace_command_palette`: a same-letter nav burst
/// is dropped after the first press.
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

/// Footer of a pinned popover.
const PINNED_FOOTER: &str = "y copy line · Enter run · Esc close";

/// More than the field and action lines of the seed row popover, so `j`
/// that many times lands on its last line.
const POPOVER_LINES_MAX: usize = 30;

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn type_palette_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
        }
    }
}

fn open_commands_filter(tui: &mut PtySession, query: &str, title: &str) {
    tui.key(':');
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains(">▏"),
        "`:` opens Quick Open commands",
        WAIT,
    );
    type_palette_filter(tui, query);
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains(title) && screen.contains(query),
        &format!("palette filter `{query}` shows `{title}`"),
        WAIT,
    );
}

/// Write an executable `/bin/sh` script `name` with `body` in `dir`.
fn shim(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

/// URLs the browser shim was started with, one per open.
fn opened(marker: &Path) -> Vec<String> {
    fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// `gw` and palette View remotely open the focused entity's forge page in
/// `$BROWSER`, and refuse an entity that is not on the remote.
///
/// Docs: `gw` on a repo row opens the repository home; on a graph commit it
/// opens `/commit/<sha>` once the commit is on a branch of origin, else
/// the status line says `commit <short> is not on origin` and nothing
/// opens. A popover View remotely line opens the entity of its own
/// section: on a remote-tracking chip, that branch (`/tree/<b>`), not the
/// row commit. The check reads local refs only.
///
/// Seed: `syncbox` pushed its seed commit to a bare origin, then made one
/// more commit (`ahead-tip-commit`, not pushed). `origin` is then pointed
/// at a GitHub URL, so `refs/remotes/origin/main` stays on the seed and no
/// test touches the network. `$BROWSER` is a shim that appends its URL to
/// a marker file; `gh` / `glab` shims fail, so badge lookups find nothing.
#[test]
fn pty_gw_views_focused_entity_remotely() {
    let (root, workspace) = ahead_workspace();
    let repo = workspace.join("syncbox");
    git(&repo, &["remote", "set-url", "origin", ORIGIN_URL]);
    let seed_sha = git_stdout(&repo, &["rev-parse", "HEAD~1"]);
    let tip_sha = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let refused = format!("commit {} is not on origin", &tip_sha[..7]);

    let shim_dir = root.join("browser-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let marker = shim_dir.join("opened");
    shim(
        &shim_dir,
        "rec",
        &format!("printf '%s\\n' \"$1\" >> '{}'\n", marker.display()),
    );
    shim(&shim_dir, "gh", "exit 1\n");
    shim(&shim_dir, "glab", "exit 1\n");
    let browser = format!("{} %s", shim_dir.join("rec").display());
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[("BROWSER", browser.as_str()), ("PATH", path.as_str())],
    );
    tui.wait_contains("syncbox", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "syncbox")
                && screen.contains("ahead-tip-commit")
                && screen.contains("seed syncbox")
                && !graph_pane_focused(screen)
        },
        "first paint: tree cursor on the syncbox repo row, graph unfocused",
        GIT_WAIT,
    );
    assert!(opened(&marker).is_empty(), "nothing opened at launch");

    // Repo row: the repository home.
    tui.keys("gw");
    tui.wait_pred(
        |screen| opened(&marker) == [REPO_PAGE] && screen.contains("opened"),
        "gw on the repo row opens the repository home in $BROWSER",
        GIT_WAIT,
    );

    // An unpushed graph commit: refused on the status line, nothing opens.
    tui.tab();
    tui.wait_pred(graph_pane_focused, "Tab focuses the graph", WAIT);
    tui.search("ahead-tip-commit");
    tui.wait_pred(
        |screen| graph_cursor_on(screen, "ahead-tip-commit"),
        "graph cursor on the unpushed commit",
        WAIT,
    );
    tui.keys("gw");
    tui.wait_pred(
        |screen| screen.contains(&refused),
        "gw on an unpushed commit says it is not on origin",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert_eq!(
        opened(&marker),
        [REPO_PAGE],
        "a refused commit opens nothing"
    );

    // Palette View remotely on the pushed seed commit: its commit page.
    tui.search("seed syncbox");
    tui.wait_pred(
        |screen| graph_cursor_on(screen, "seed syncbox"),
        "graph cursor on the pushed seed commit",
        WAIT,
    );
    open_commands_filter(&mut tui, "view remotely", "View remotely");
    tui.enter();
    let commit_page = format!("{REPO_PAGE}/commit/{seed_sha}");
    tui.wait_pred(
        |screen| {
            !palette_open(screen) && opened(&marker) == [REPO_PAGE.to_string(), commit_page.clone()]
        },
        "palette View remotely opens the focused commit page",
        GIT_WAIT,
    );

    // Popover View remotely on the same row's `origin/main` chip: the
    // branch page, not the commit. `gh` pins the commit node section, then
    // the chip section; the last line of the box is the chip's View
    // remotely, and `j` stops there.
    tui.keys("gh");
    tui.wait_pred(
        |screen| screen.contains(PINNED_FOOTER) && screen.contains("remote-tracking branch"),
        "gh pins the seed row's popover with its origin/main chip section",
        WAIT,
    );
    for _ in 0..POPOVER_LINES_MAX {
        tui.key('j');
        tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
    }
    tui.enter();
    let branch_page = format!("{REPO_PAGE}/tree/main");
    tui.wait_pred(
        |screen| {
            !screen.contains(PINNED_FOOTER)
                && opened(&marker)
                    == [
                        REPO_PAGE.to_string(),
                        commit_page.clone(),
                        branch_page.clone(),
                    ]
        },
        "the chip's View remotely line opens the branch page and closes the popover",
        GIT_WAIT,
    );
}
