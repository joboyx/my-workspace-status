use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::harness::PtySession;
use crate::pr_support::{forge_env, prs_headers, seed_prs, total_is, PRS_BOARD};
use crate::support::{GIT_WAIT, WAIT};

/// Headers once the title command has run: a title only where the command
/// exits 0 with a non-empty first line, and never for `other`.
const WITH_TITLES: [&str; 5] = [
    "ABC-130 · Upload limits · > fix CI !31",
    "ABC-120 · Totals export · > merge #43",
    "ABC-125 · > wait for CI !32",
    "ABC-110 · m all 2 merged",
    "other",
];

/// `WS_STATUS_PR_TITLE_CMD` names a program that the PRs tab runs once per
/// ticket key, and its first output line becomes the group title
/// (`docs/configuration.md`, `WS_STATUS_PR_TITLE_CMD`).
///
/// Seed: [`PRS_BOARD`] and a script that logs its argument, then prints
/// `Upload limits` for `ABC-130`, two lines starting `Totals export` for
/// `ABC-120`, a line and exit 3 for `ABC-125`, and nothing for `ABC-110`.
///
/// Proof: the headers read `ABC-130 · Upload limits · > fix CI !31` and
/// `ABC-120 · Totals export · > merge #43` (first line only); `ABC-125`
/// (failed run) and `ABC-110` (no output) show no title; `other` stays
/// bare. The log names exactly the four ticket keys, each as the only
/// argument: never `other` or the keyless branch. A title from any exit
/// code shows `ignored` on `ABC-125`; a command run for every group logs
/// `other`.
#[test]
fn pty_prs_tab_title_command_names_the_ticket_groups() {
    let (root, workspace, forge) = seed_prs("ws-tui-tty-prs-titles", &PRS_BOARD);
    let log = root.join("title-calls.log");
    let script = root.join("pr-titles");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             printf '%s|%s\\n' \"$#\" \"$1\" >> '{}'\n\
             case \"$1\" in\n\
             ABC-130) echo 'Upload limits' ;;\n\
             ABC-120) printf 'Totals export\\nsecond line\\n' ;;\n\
             ABC-125) echo 'ignored'; exit 3 ;;\n\
             esac\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let script_env = script.display().to_string();

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        40,
        &forge_env(&forge, &[("WS_STATUS_PR_TITLE_CMD", &script_env)]),
    );
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "9 PRs · 2 need you"),
        "every lookup lands: the workspace row counts nine PRs",
        GIT_WAIT,
    );
    tui.send_bytes(b"gp");
    let want: Vec<(bool, String)> = WITH_TITLES
        .iter()
        .map(|text| (false, text.to_string()))
        .collect();
    tui.wait_pred(
        |_| prs_headers(&tui) == want,
        "the title command names the ticket groups",
        WAIT,
    );

    let calls: BTreeSet<String> = fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let keys = ["1|ABC-110", "1|ABC-120", "1|ABC-125", "1|ABC-130"].map(str::to_string);
    assert_eq!(
        calls,
        BTreeSet::from(keys),
        "one argument per run, the four ticket keys only"
    );
}
