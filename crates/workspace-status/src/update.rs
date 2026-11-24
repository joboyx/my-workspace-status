//! cargo-dist sidecar (`workspace-status-update`) used by `--update` and the
//! TUI-startup prompt.
//!
//! Before the sidecar runs, the update prints GitHub Release notes for
//! published versions newer than [`crate::APP_VERSION`]. Notes are the
//! git-cliff changelog that the Release workflow prepends to cargo-dist's
//! installer copy. A failed fetch stays quiet and still runs the sidecar.
//!
//! A dev build ([`crate::DEV_BUILD`] set by `scripts/install-dev.sh`) skips
//! the notes and runs `workspace-status-update-dev` instead. That shim
//! rebuilds and reinstalls the source checkout.
//!
//! The sidecar runs as a child. The app path ([`env::current_exe`]) is read
//! before it starts, and the file at that path is fingerprinted before and
//! after. A new version was installed when the sidecar exits with success and
//! the fingerprint changed. Then the app starts again from that path with the
//! original arguments minus `--update` (Unix: `exec`; elsewhere: spawn, wait,
//! and forward the exit code). With no new install nothing starts again.

use std::cmp::Ordering;
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

use serde::Deserialize;

use crate::update_check::{github_get, is_newer_release};

/// GitHub Releases list (newest first), first page only (`per_page=100`, the
/// API maximum). This project has more than 100 releases, so an install more
/// than 100 releases behind the latest sees only the newest 100 in the
/// changelog it prints. The updater itself is not affected.
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

/// Updater command name on PATH: the cargo-dist sidecar for a release build,
/// the `scripts/install-dev.sh` shim for a dev build.
pub(crate) fn updater_name(dev_build: bool) -> &'static str {
    if dev_build {
        "workspace-status-update-dev"
    } else {
        "workspace-status-update"
    }
}

/// File name of the updater next to `ws` / `workspace-status` (or the `-dev`
/// pair when `dev_build`).
pub(crate) fn updater_file_name(dev_build: bool) -> String {
    let name = updater_name(dev_build);
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn sibling_updater(current_exe: &Path, dev_build: bool) -> Option<PathBuf> {
    let path = current_exe.parent()?.join(updater_file_name(dev_build));
    path.is_file().then_some(path)
}

fn updater_command(current_exe: &Path, dev_build: bool) -> Command {
    if let Some(path) = sibling_updater(current_exe, dev_build) {
        Command::new(path)
    } else {
        Command::new(updater_name(dev_build))
    }
}

/// `ws --update`: print newer GitHub Release notes, run the updater, and
/// start the new version when one was installed.
///
/// `args` is the full process argv ([`env::args_os`]). A dev build skips the
/// notes and runs `workspace-status-update-dev`. With a new install, Unix
/// replaces this process with the new version; on other platforms the new
/// version's exit code is returned. A failed relaunch returns 1. With no new
/// install the updater's exit code is returned (1 for a signal, an
/// out-of-range code, or a failed spawn).
pub(crate) fn run_self_update(args: &[OsString]) -> ExitCode {
    let code = match run_updater() {
        UpdaterRun::Installed(app) => relaunch(&app, &relaunch_args(args)).unwrap_or(1),
        UpdaterRun::NotInstalled(code) => code,
    };
    ExitCode::from(code)
}

/// The startup prompt's `y`: the same update as [`run_self_update`], for a
/// caller that opens the TUI when nothing new starts.
///
/// `Some(code)`: the new version ran as a child and exited with `code`
/// (non-Unix only; on Unix a relaunch replaces this process). `None`: no new
/// install, or the relaunch failed; the caller opens the current TUI.
pub(crate) fn run_startup_update(args: &[OsString]) -> Option<u8> {
    match run_updater() {
        UpdaterRun::Installed(app) => relaunch(&app, &relaunch_args(args)),
        UpdaterRun::NotInstalled(_) => None,
    }
}

/// Result of one updater run.
#[derive(Debug, PartialEq, Eq)]
enum UpdaterRun {
    /// A new version was installed at this app path.
    Installed(PathBuf),
    /// Nothing new was installed; the exit code to report.
    NotInstalled(u8),
}

/// Print the release notes (release build only), then run the updater as a
/// child and tell whether it installed a new version.
fn run_updater() -> UpdaterRun {
    let dev_build = crate::DEV_BUILD.is_some();
    if !dev_build {
        print_changes_since_installed();
    }
    // Resolve the app path before the updater runs: after a replace, Linux
    // `/proc/self/exe` reads `<path> (deleted)`.
    let app = match env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("failed to resolve current executable: {err}");
            return UpdaterRun::NotInstalled(1);
        }
    };
    let before = fingerprint(&app);
    let name = updater_name(dev_build);
    let status = match updater_command(&app, dev_build).status() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("failed to run {name}: {err}");
            return UpdaterRun::NotInstalled(1);
        }
    };
    let after = fingerprint(&app);
    if installed(status.success(), before, after) {
        UpdaterRun::Installed(app)
    } else {
        UpdaterRun::NotInstalled(exit_code(status))
    }
}

/// Length and content hash of a file, to tell whether the updater replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    hash: u64,
}

/// Fingerprint of the file at `path`: its length and a streaming hash of its
/// bytes. `None` when the file cannot be read.
fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let mut file = File::open(path).ok()?;
    let mut hasher = DefaultHasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.write(&buf[..n]);
        len += n as u64;
    }
    Some(Fingerprint {
        len,
        hash: hasher.finish(),
    })
}

/// A new version was installed: the updater succeeded and the app file's
/// fingerprint was read both times and changed.
fn installed(success: bool, before: Option<Fingerprint>, after: Option<Fingerprint>) -> bool {
    success && matches!((before, after), (Some(a), Some(b)) if a != b)
}

/// The arguments for the relaunch: `args[1..]` with every `--update` before
/// a `--` terminator removed. Tokens after `--` stay.
fn relaunch_args(args: &[OsString]) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut after_terminator = false;
    for arg in args.iter().skip(1) {
        if !after_terminator && arg == "--update" {
            continue;
        }
        if arg == "--" {
            after_terminator = true;
        }
        out.push(arg.clone());
    }
    out
}

/// Exit code to report for a child: its code when it fits in a `u8`, else 1
/// (a signal or an out-of-range code).
fn exit_code(status: ExitStatus) -> u8 {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

/// Start the new version at `app` with `args`; cwd and environment are
/// inherited. Unix replaces this process, so it returns only on failure.
/// `None` after a failure, with `failed to run <app path>: <err>` on stderr.
#[cfg(unix)]
fn relaunch(app: &Path, args: &[OsString]) -> Option<u8> {
    use std::os::unix::process::CommandExt;
    let err = Command::new(app).args(args).exec();
    eprintln!("failed to run {}: {err}", app.display());
    None
}

/// Start the new version at `app` with `args` as a child; cwd and
/// environment are inherited. Returns its exit code, or `None` after a spawn
/// failure, with `failed to run <app path>: <err>` on stderr.
#[cfg(not(unix))]
fn relaunch(app: &Path, args: &[OsString]) -> Option<u8> {
    match Command::new(app).args(args).status() {
        Ok(status) => Some(exit_code(status)),
        Err(err) => {
            eprintln!("failed to run {}: {err}", app.display());
            None
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> PathBuf {
        let path = crate::testutil::unique_dir("ws-update-sidecar");
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
        let sidecar = dir.join(updater_file_name(false));
        fs::write(&sidecar, b"").unwrap();
        assert_eq!(
            sibling_updater(&exe, false).as_deref(),
            Some(sidecar.as_path())
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sibling_updater_none_without_sidecar() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        assert_eq!(sibling_updater(&exe, false), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dev_build_picks_dev_updater_name() {
        assert_eq!(updater_name(false), "workspace-status-update");
        assert_eq!(updater_name(true), "workspace-status-update-dev");
        let ext = if cfg!(windows) { ".exe" } else { "" };
        assert_eq!(
            updater_file_name(false),
            format!("workspace-status-update{ext}")
        );
        assert_eq!(
            updater_file_name(true),
            format!("workspace-status-update-dev{ext}")
        );
    }

    #[test]
    fn dev_sibling_updater_ignores_release_sidecar() {
        let dir = temp_dir();
        let exe = dir.join("ws-dev");
        fs::write(&exe, b"").unwrap();
        fs::write(dir.join(updater_file_name(false)), b"").unwrap();
        assert_eq!(sibling_updater(&exe, true), None);
        let dev = dir.join(updater_file_name(true));
        fs::write(&dev, b"").unwrap();
        assert_eq!(sibling_updater(&exe, true).as_deref(), Some(dev.as_path()));
        let _ = fs::remove_dir_all(dir);
    }

    fn os_args(args: &[&str]) -> Vec<OsString> {
        std::iter::once("ws")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect()
    }

    #[test]
    fn relaunch_args_drop_update_before_terminator() {
        assert_eq!(relaunch_args(&os_args(&["--update"])), os_args(&[])[1..]);
        assert_eq!(
            relaunch_args(&os_args(&["--update", "-a", "repo"])),
            os_args(&["-a", "repo"])[1..]
        );
        assert_eq!(
            relaunch_args(&os_args(&["-C", "dir", "--update"])),
            os_args(&["-C", "dir"])[1..]
        );
        assert_eq!(
            relaunch_args(&os_args(&["--update", "--", "--update"])),
            os_args(&["--", "--update"])[1..]
        );
        assert_eq!(
            relaunch_args(&os_args(&["--update", "--plain", "--update"])),
            os_args(&["--plain"])[1..]
        );
    }

    #[test]
    fn installed_needs_success_and_changed_fingerprint() {
        let a = Some(Fingerprint { len: 3, hash: 1 });
        let b = Some(Fingerprint { len: 3, hash: 2 });
        assert!(installed(true, a, b));
        assert!(!installed(true, a, a));
        assert!(!installed(false, a, b));
        assert!(!installed(true, None, b));
        assert!(!installed(true, a, None));
        assert!(!installed(true, None, None));
    }

    #[test]
    fn fingerprint_follows_file_bytes() {
        let dir = temp_dir();
        let one = dir.join("one");
        let same = dir.join("same");
        let other = dir.join("other");
        fs::write(&one, b"abc").unwrap();
        fs::write(&same, b"abc").unwrap();
        fs::write(&other, b"abd").unwrap();
        let fp = fingerprint(&one).expect("readable file");
        assert_eq!(fp.len, 3);
        assert_eq!(fingerprint(&same), Some(fp));
        assert_ne!(fingerprint(&other), Some(fp));
        assert_eq!(fingerprint(&dir.join("missing")), None);
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
