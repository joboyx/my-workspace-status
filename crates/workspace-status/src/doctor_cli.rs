//! `ws doctor`: the Doctor setup checkup as plain text to paste.
//!
//! The report has the rows of the TUI Doctor (`Doctor: setup checkup` in
//! the command palette), built by the same functions and in the same order
//! ([`doctor_rows`]). Only the inputs differ:
//!
//! - Terminal size: from the terminal when stdout is one, else unknown.
//! - Capability rows: [`query_capabilities`] asks the terminal only when
//!   stdin and stdout are both terminals, else the rows are unknown.
//! - Mouse wheel (live test): unknown, "TUI only" ([`WheelRow::TuiOnly`]).
//! - Mouse: the capture state the TUI starts with
//!   ([`crate::tui::MOUSE_CAPTURE_AT_START`]).
//!
//! The text has no colour and no escape codes. The exit code is `1` when a
//! row fails or the config does not load, else `0` ([`exit_code`]).

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::config::{load_workspace_status_config, user_config_path};
use crate::discovery::discover_checkouts;
use crate::git::current_branch;
use crate::settings::Settings;
use crate::tui::doctor::{self, Check, CheckStatus, CollectInputs, WheelRow, DOCTOR_TITLE};
use crate::tui::term_caps::{doctor_rows, query_capabilities, CAPS_QUERY_TIMEOUT};

/// Width the status word is padded to: the longest one, `unknown`.
const STATUS_WIDTH: usize = 7;

/// Indent of the why, `hint:`, and `fix:` lines: under the row name
/// (two spaces, the status word, one space).
const TEXT_INDENT: usize = 2 + STATUS_WIDTH + 1;

/// Run `ws doctor` on the workspace root `cwd`: probe the setup, print the
/// report on stdout, and return the exit code.
///
/// Loads the config and the settings as the other modes do. A config that
/// does not load prints its error on stderr and returns `1`. No release
/// check runs and no TUI opens.
pub(crate) fn run(cwd: &Path) -> u8 {
    let loaded = match load_workspace_status_config(cwd) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let settings = Settings::from_env(&loaded.runtime);
    crate::git::init_git_binary(settings.git.clone());

    // Ask the terminal before anything prints: the query holds raw mode
    // for its wait, and the report prints once raw mode is off again.
    let caps = query_capabilities(CAPS_QUERY_TIMEOUT);
    let term_size = if io::stdout().is_terminal() {
        crossterm::terminal::size().ok()
    } else {
        None
    };
    // The checkouts the TUI loads (hidden ones too), with the branch each
    // is on: no `git status`, only what the remote lookup needs.
    let checkouts: Vec<(PathBuf, String)> =
        discover_checkouts(cwd, &crate::tui::discover_config(&loaded), None)
            .into_iter()
            .map(|(repo, _, _)| {
                let dir = cwd.join(repo);
                let branch = current_branch(&dir);
                (dir, branch)
            })
            .collect();
    let remote_urls = doctor::remote_urls(&checkouts);
    let user_config = user_config_path();
    let probe = doctor::collect(&CollectInputs {
        term_size,
        settings: &settings,
        mouse_enabled: crate::tui::MOUSE_CAPTURE_AT_START,
        editor_config: loaded.editor.as_deref(),
        diff_tool_config: loaded.diff_tool.as_deref(),
        remote_urls: &remote_urls,
        workspace_root: cwd,
        user_config: user_config.as_deref(),
    });
    let rows = doctor_rows(
        &doctor::checks(&probe),
        WheelRow::TuiOnly,
        &caps,
        probe.os,
        probe.in_tmux(),
    );

    let mut out = io::stdout().lock();
    for line in report_lines(&rows, &crate::version_label()) {
        // A closed stdout (`ws doctor | head`) ends the report; the exit
        // code still comes from the rows.
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
    let _ = out.flush();
    exit_code(&rows)
}

/// The report lines for `rows`. Pure.
///
/// The head line `Doctor · setup checkup · workspace-status <version>`,
/// the summary line `<n> ok · <n> warn · <n> fail · <n> unknown`, then per
/// group a blank line and its title (`TERMINAL`, `TOOLS`, `FILES`), and per
/// row `  <status padded to 7> <name>` with the why, `hint: …`, and
/// `fix: …` (the line to paste) lines under the name.
pub(crate) fn report_lines(rows: &[Check], version: &str) -> Vec<String> {
    let summary = doctor::summary(rows)
        .counts()
        .iter()
        .map(|(status, n)| format!("{n} {}", status.label()))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut lines = vec![
        format!("{DOCTOR_TITLE} · workspace-status {version}"),
        summary,
    ];
    let mut group = None;
    for check in rows {
        if group != Some(check.group) {
            group = Some(check.group);
            lines.push(String::new());
            lines.push(check.group.title().to_string());
        }
        lines.push(format!(
            "  {:<STATUS_WIDTH$} {}",
            check.status.label(),
            check.name
        ));
        push_text(&mut lines, "", &check.why);
        if let Some(fix) = &check.fix {
            if let Some(hint) = &fix.hint {
                push_text(&mut lines, "hint: ", hint);
            }
            if let Some(copy) = &fix.copy {
                push_text(&mut lines, "fix: ", copy);
            }
        }
    }
    lines
}

/// Push `text` under a row name, each of its lines indented, the first one
/// after `label`. Empty text pushes nothing.
fn push_text(lines: &mut Vec<String>, label: &str, text: &str) {
    for (at, line) in text.lines().enumerate() {
        let label = if at == 0 { label } else { "" };
        lines.push(format!("{:TEXT_INDENT$}{label}{line}", ""));
    }
}

/// Exit code of a report: `1` when a row is [`CheckStatus::Fail`], else `0`
/// (warn and unknown rows are fine).
pub(crate) fn exit_code(rows: &[Check]) -> u8 {
    u8::from(rows.iter().any(|check| check.status == CheckStatus::Fail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::doctor::{CheckGroup, Fix};

    fn check(
        group: CheckGroup,
        name: &'static str,
        status: CheckStatus,
        why: &str,
        fix: Option<Fix>,
    ) -> Check {
        Check {
            group,
            name,
            status,
            why: why.to_string(),
            fix,
        }
    }

    #[test]
    fn report_prints_head_summary_groups_rows_hints_and_fix_lines() {
        let rows = vec![
            check(
                CheckGroup::Terminal,
                "Terminal size",
                CheckStatus::Unknown,
                "stdout is not a terminal.",
                None,
            ),
            check(
                CheckGroup::Terminal,
                "Mouse",
                CheckStatus::Warn,
                "tmux mouse is off.",
                Some(Fix {
                    hint: Some("Turn tmux mouse mode on.".to_string()),
                    copy: Some("set -g mouse on".to_string()),
                }),
            ),
            check(
                CheckGroup::Tools,
                "git",
                CheckStatus::Ok,
                "git version 2.43.0",
                None,
            ),
            check(
                CheckGroup::Tools,
                "Editor",
                CheckStatus::Warn,
                "No editor found.",
                Some(Fix {
                    hint: Some("Set EDITOR.".to_string()),
                    copy: None,
                }),
            ),
            check(
                CheckGroup::Files,
                "State files",
                CheckStatus::Fail,
                "UI state: bad JSON.",
                Some(Fix {
                    hint: None,
                    copy: Some("mv a a.bak".to_string()),
                }),
            ),
        ];
        assert_eq!(
            report_lines(&rows, "v0.1.2"),
            vec![
                "Doctor · setup checkup · workspace-status v0.1.2",
                "1 ok · 2 warn · 1 fail · 1 unknown",
                "",
                "TERMINAL",
                "  unknown Terminal size",
                "          stdout is not a terminal.",
                "  warn    Mouse",
                "          tmux mouse is off.",
                "          hint: Turn tmux mouse mode on.",
                "          fix: set -g mouse on",
                "",
                "TOOLS",
                "  ok      git",
                "          git version 2.43.0",
                "  warn    Editor",
                "          No editor found.",
                "          hint: Set EDITOR.",
                "",
                "FILES",
                "  fail    State files",
                "          UI state: bad JSON.",
                "          fix: mv a a.bak",
            ]
        );
    }

    #[test]
    fn report_indents_every_line_of_a_multi_line_text_and_skips_an_empty_one() {
        let rows = vec![check(
            CheckGroup::Files,
            "Config files",
            CheckStatus::Ok,
            "",
            Some(Fix {
                hint: Some("first\nsecond".to_string()),
                copy: None,
            }),
        )];
        assert_eq!(
            report_lines(&rows, "v1")[3..],
            [
                "FILES",
                "  ok      Config files",
                "          hint: first",
                "          second",
            ]
        );
    }

    #[test]
    fn report_with_no_rows_prints_the_head_and_zero_counts() {
        assert_eq!(
            report_lines(&[], "v1"),
            vec![
                "Doctor · setup checkup · workspace-status v1",
                "0 ok · 0 warn · 0 fail · 0 unknown",
            ]
        );
    }

    #[test]
    fn exit_code_is_one_only_for_a_fail_row() {
        let with = |status| {
            vec![
                check(CheckGroup::Terminal, "a", CheckStatus::Ok, "", None),
                check(CheckGroup::Tools, "b", status, "", None),
            ]
        };
        assert_eq!(exit_code(&[]), 0);
        assert_eq!(exit_code(&with(CheckStatus::Ok)), 0);
        assert_eq!(exit_code(&with(CheckStatus::Unknown)), 0);
        assert_eq!(exit_code(&with(CheckStatus::Warn)), 0);
        assert_eq!(exit_code(&with(CheckStatus::Fail)), 1);
    }

    #[test]
    fn cli_rows_keep_the_tui_order_with_the_tui_only_wheel_row() {
        use crate::tui::term_caps::TermCaps;
        let probe = vec![
            check(
                CheckGroup::Terminal,
                "Terminal size",
                CheckStatus::Unknown,
                "",
                None,
            ),
            check(CheckGroup::Terminal, "Mouse", CheckStatus::Ok, "On.", None),
            check(CheckGroup::Tools, "git", CheckStatus::Ok, "", None),
        ];
        let rows = doctor_rows(
            &probe,
            WheelRow::TuiOnly,
            &TermCaps::not_asked(),
            doctor::Os::Linux,
            false,
        );
        let lines = report_lines(&rows, "v1");
        let names: Vec<&str> = lines
            .iter()
            .filter_map(|line| line.strip_prefix("  "))
            .filter(|line| !line.starts_with(' '))
            .collect();
        assert_eq!(
            names,
            vec![
                "unknown Terminal size",
                "ok      Mouse",
                "unknown Mouse wheel (live test)",
                "unknown Terminal (XTVERSION)",
                "unknown SGR mouse (1006)",
                "unknown Bracketed paste (2004)",
                "unknown Synchronized output (2026)",
                "unknown Kitty keyboard (CSI u)",
                "unknown Background (OSC 11)",
                "ok      git",
            ]
        );
        assert!(
            lines.contains(
                &"          TUI only: open Doctor: setup checkup in the TUI and scroll once."
                    .to_string()
            ),
            "{lines:#?}"
        );
        assert!(
            lines.contains(&"          Not asked: stdin or stdout is not a terminal.".to_string()),
            "{lines:#?}"
        );
    }
}
