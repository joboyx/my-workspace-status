//! cargo-dist sidecar (`workspace-status-update`) used by `--update`.
//!
//! Before exec, `--update` prints GitHub Release notes for published versions
//! newer than [`crate::APP_VERSION`]. Notes are the git-cliff changelog that
//! the Release workflow prepends to cargo-dist's installer copy. A failed
//! fetch stays quiet and still runs the sidecar.

use std::cmp::Ordering;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

use crate::update_check::{github_get, is_newer_release};

/// GitHub Releases list (newest first). `per_page=100` covers this project's history.
const RELEASES_LIST_URL: &str =
    "https://api.github.com/repos/joboyx/my-workspace-status/releases?per_page=100";

/// cargo-dist announcement always starts here; git-cliff notes sit above it.
const INSTALL_HEADING: &str = "## Install ";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GithubReleaseNotes {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// File name of the cargo-dist updater next to `ws` / `workspace-status`.
pub(crate) fn updater_file_name() -> &'static str {
    if cfg!(windows) {
        "workspace-status-update.exe"
    } else {
        "workspace-status-update"
    }
}

fn sibling_updater(current_exe: &Path) -> Option<PathBuf> {
    let path = current_exe.parent()?.join(updater_file_name());
    path.is_file().then_some(path)
}

fn updater_command(current_exe: &Path) -> Command {
    if let Some(path) = sibling_updater(current_exe) {
        Command::new(path)
    } else {
        Command::new("workspace-status-update")
    }
}

/// Print newer GitHub Release notes, then exec `workspace-status-update`.
///
/// On Unix this replaces the process. If exec fails, or on Windows after the
/// sidecar returns, the caller receives an [`ExitCode`].
pub(crate) fn run_self_update() -> ExitCode {
    print_changes_since_installed();
    let exe = match env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("failed to resolve current executable: {err}");
            return ExitCode::from(1);
        }
    };
    let mut cmd = updater_command(&exe);
    exec_updater(&mut cmd)
}

fn print_changes_since_installed() {
    let Ok(body) = github_get(RELEASES_LIST_URL, "8") else {
        return;
    };
    let Some(releases) = parse_releases_json(&body) else {
        return;
    };
    let Some(text) = format_update_changelog(crate::APP_VERSION, &releases) else {
        return;
    };
    let mut stdout = io::stdout();
    let _ = write!(stdout, "{text}");
    let _ = stdout.flush();
}

fn parse_releases_json(body: &str) -> Option<Vec<GithubReleaseNotes>> {
    serde_json::from_str(body).ok()
}

/// git-cliff section of a Release body; empty when the body is installer-only.
fn changelog_from_release_body(body: &str) -> &str {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "";
    }
    match trimmed.find(INSTALL_HEADING) {
        Some(0) => "",
        Some(i) => trimmed[..i].trim_end(),
        None => trimmed,
    }
}

fn display_version(raw: &str) -> &str {
    raw.trim().trim_start_matches(['v', 'V'])
}

/// Human text of published releases newer than `current`, newest first.
///
/// `None` when every published release is current or older.
fn format_update_changelog(current: &str, releases: &[GithubReleaseNotes]) -> Option<String> {
    let mut newer: Vec<&GithubReleaseNotes> = releases
        .iter()
        .filter(|rel| !rel.draft && !rel.prerelease)
        .filter(|rel| is_newer_release(current, &rel.tag_name))
        .collect();
    if newer.is_empty() {
        return None;
    }
    newer.sort_by(|a, b| {
        match (
            is_newer_release(&a.tag_name, &b.tag_name),
            is_newer_release(&b.tag_name, &a.tag_name),
        ) {
            (true, _) => Ordering::Greater,
            (_, true) => Ordering::Less,
            _ => b.tag_name.cmp(&a.tag_name),
        }
    });
    let latest = display_version(&newer[0].tag_name);
    let current_disp = display_version(current);
    let mut out = format!("Updating {current_disp} -> {latest}\n");
    for rel in newer {
        let notes = changelog_from_release_body(rel.body.as_deref().unwrap_or(""));
        if notes.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(notes);
        if !notes.ends_with('\n') {
            out.push('\n');
        }
    }
    Some(out)
}

#[cfg(unix)]
fn exec_updater(cmd: &mut Command) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    eprintln!("failed to run workspace-status-update: {err}");
    ExitCode::from(1)
}

#[cfg(not(unix))]
fn exec_updater(cmd: &mut Command) -> ExitCode {
    match cmd.status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map(ExitCode::from)
            .unwrap_or(ExitCode::from(1)),
        Err(err) => {
            eprintln!("failed to run workspace-status-update: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "ws-update-sidecar-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn rel(tag: &str, body: &str, draft: bool, prerelease: bool) -> GithubReleaseNotes {
        GithubReleaseNotes {
            tag_name: tag.into(),
            body: Some(body.into()),
            draft,
            prerelease,
        }
    }

    #[test]
    fn sibling_updater_finds_file_next_to_exe() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        let sidecar = dir.join(updater_file_name());
        fs::write(&sidecar, b"").unwrap();
        assert_eq!(sibling_updater(&exe).as_deref(), Some(sidecar.as_path()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sibling_updater_none_without_sidecar() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        assert_eq!(sibling_updater(&exe), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn changelog_strips_cargo_dist_install_section() {
        let body = "## [1.2.3] - 2026-08-28\n\n### Features\n\n- Add notes\n\n## Install workspace-status 1.2.3\n\ncurl install\n";
        assert_eq!(
            changelog_from_release_body(body),
            "## [1.2.3] - 2026-08-28\n\n### Features\n\n- Add notes"
        );
        assert_eq!(
            changelog_from_release_body("## Install workspace-status 1.2.3\n\ncurl\n"),
            ""
        );
        assert_eq!(changelog_from_release_body("   \n"), "");
        assert_eq!(
            changelog_from_release_body("## [1.0.0]\n\n- Only notes\n"),
            "## [1.0.0]\n\n- Only notes"
        );
    }

    #[test]
    fn format_skips_current_and_older() {
        let releases = [
            rel(
                "v0.1.50",
                "## Install workspace-status 0.1.50\n",
                false,
                false,
            ),
            rel("v0.1.49", "## [0.1.49]\n\n- old\n", false, false),
        ];
        assert_eq!(format_update_changelog("0.1.50", &releases), None);
        assert_eq!(format_update_changelog("0.1.51", &releases), None);
    }

    #[test]
    fn format_concatenates_newer_notes_newest_first() {
        let releases = [
            rel(
                "v0.1.48",
                "## [0.1.48]\n\n### Features\n\n- Older feat\n\n## Install workspace-status 0.1.48\n",
                false,
                false,
            ),
            rel(
                "v0.1.50",
                "## [0.1.50] - 2026-08-28\n\n### Bug Fixes\n\n- Newer fix\n\n## Install workspace-status 0.1.50\n",
                false,
                false,
            ),
            rel(
                "v0.1.49",
                "## [0.1.49]\n\n### Features\n\n- Mid feat\n\n## Install workspace-status 0.1.49\n",
                false,
                false,
            ),
        ];
        let text = format_update_changelog("0.1.48", &releases).expect("newer");
        assert_eq!(
            text,
            "Updating 0.1.48 -> 0.1.50\n\n\
             ## [0.1.50] - 2026-08-28\n\n### Bug Fixes\n\n- Newer fix\n\n\
             ## [0.1.49]\n\n### Features\n\n- Mid feat\n"
        );
    }

    #[test]
    fn format_skips_drafts_prereleases_and_installer_only_bodies() {
        let releases = [
            rel(
                "v9.0.0",
                "## Install workspace-status 9.0.0\n",
                false,
                false,
            ),
            rel("v8.0.0", "## [8.0.0]\n\n- Draft notes\n", true, false),
            rel("v7.0.0", "## [7.0.0]\n\n- Pre notes\n", false, true),
        ];
        assert_eq!(
            format_update_changelog("0.1.0", &releases).as_deref(),
            Some("Updating 0.1.0 -> 9.0.0\n")
        );
    }

    #[test]
    fn parse_releases_json_reads_github_array() {
        let json = r#"[{"tag_name":"v1.0.0","body":"notes","draft":false,"prerelease":false}]"#;
        let parsed = parse_releases_json(json).expect("json");
        assert_eq!(parsed[0].tag_name, "v1.0.0");
        assert_eq!(parsed[0].body.as_deref(), Some("notes"));
        assert_eq!(parse_releases_json("not json"), None);
        assert_eq!(parse_releases_json("{}"), None);
    }
}
