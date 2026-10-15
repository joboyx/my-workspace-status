//! Doctor setup checkup: a report of the setup the TUI needs to run as
//! expected.
//!
//! [`collect`] reads the environment, `PATH`, a few read-only commands
//! (`<git> --version`, `rustc --version` on a dev build, `tmux show-options
//! -gv <opt>` inside tmux), and the config and store files into a [`Probe`].
//! [`checks`] turns a [`Probe`] into one [`Check`] row per setup item; it is
//! pure, so tests build a [`Probe`] by hand for every [`Os`].
//!
//! The Doctor only reports. Nothing here writes a file, a setting, the
//! environment, the clipboard, or the terminal. [`collect`] blocks (it spawns
//! commands and reads files), so call it on a blocking worker thread, never on
//! the TTY draw / event thread.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::comments::check_comment_store_bytes;
use super::diff_tool::resolve_diff_tool;
use super::editor::{parse_editor_argv, resolve_editor};
use super::fetch_claim::check_background_fetch_store_bytes;
use super::icons::{IconKind, REQUIRED_FONT};
use super::pull_request::{browser_command, parse_remote, Forge};
use super::search_files::SEARCH_PREVIEW_MIN_COLS;
use super::split::{MIN_TERM_COLS, MIN_TERM_ROWS};
use super::ui_state_store::check_ui_state_bytes;
use super::viewed::check_viewed_store_bytes;
use crate::config::{load_config_files, CONFIG_FILENAME};
use crate::git::resolve_git_binary;
use crate::settings::Settings;
use crate::update_check::check_update_check_store_bytes;

/// Operating system family the checks target.
///
/// Data, not only `cfg`, so a Linux test run covers the macOS and Windows
/// rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Linux, WSL, and any other Unix that is not macOS.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl Os {
    /// The OS this binary was built for.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    /// The `std::env::consts::OS` value [`browser_command`] matches on.
    fn consts_name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }
}

/// Outcome of one check. Ordered by severity: `Ok < Warn < Fail`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckStatus {
    /// Set up as expected (or an intentional fallback).
    Ok,
    /// Works, but a feature degrades or the probe cannot confirm it.
    Warn,
    /// A feature the TUI needs does not work.
    Fail,
}

/// Section a check belongs to, in paint order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckGroup {
    /// Terminal size, locale, colour, font, hyperlinks, clipboard, mouse.
    Terminal,
    /// External programs: git, editor, diff tool, browser, forge CLIs.
    Tools,
    /// Config and state files.
    Files,
}

/// What the user can do about a check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fix {
    /// Short advice in words.
    pub hint: Option<String>,
    /// One line to paste: a shell command, a tmux.conf line, or a JSON
    /// config line.
    pub copy: Option<String>,
}

/// One row of the checkup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Section the row paints under.
    pub group: CheckGroup,
    /// Short name, for example `Terminal size`.
    pub name: &'static str,
    /// Outcome.
    pub status: CheckStatus,
    /// What was found and what it affects (one or two short sentences).
    pub why: String,
    /// Advice, when there is any.
    pub fix: Option<Fix>,
}

/// The git binary the app resolved, and how it answered `--version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitProbe {
    /// Resolved binary (`WORKSPACE_STATUS_GIT` / config `git` /
    /// `/usr/bin/git` / `git`).
    pub path: String,
    /// True when `WORKSPACE_STATUS_GIT` or config `git` set it.
    pub explicit: bool,
    /// Trimmed stdout of `<git> --version`, or why it did not run.
    pub version: Result<String, String>,
}

/// tmux option values read with `tmux show-options -gv <opt>`. `None` is an
/// option that could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TmuxOptions {
    /// `terminal-features` (one entry per line).
    pub terminal_features: Option<String>,
    /// `set-clipboard`: `on`, `external`, or `off`.
    pub set_clipboard: Option<String>,
    /// `mouse`: `on` or `off`.
    pub mouse: Option<String>,
}

/// One config file path and whether something is there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileProbe {
    /// Absolute path.
    pub path: PathBuf,
    /// True when the path exists (a dangling symlink counts).
    pub present: bool,
}

/// The config files, re-read with the app's loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigProbe {
    /// User config file; `None` when neither `XDG_CONFIG_HOME` nor `HOME`
    /// gives a path.
    pub user: Option<FileProbe>,
    /// Workspace config file ([`CONFIG_FILENAME`] under the workspace root).
    pub workspace: FileProbe,
    /// The loader's error text, when it refused the files.
    pub error: Option<String>,
}

/// One of the persisted state files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    /// Comment store (`WS_STATUS_COMMENT_STORE` / `commentStore`).
    Comments,
    /// Viewed-marks store (`WS_STATUS_VIEWED_STORE` / `viewedStore`).
    Viewed,
    /// UI-state store (`WS_STATUS_UI_STORE` / `uiStore`).
    UiState,
    /// Update-check store (`WS_STATUS_UPDATE_CHECK_STORE` / `updateCheckStore`).
    UpdateCheck,
    /// Shared background-fetch claim and outcome store (no env or config key).
    BackgroundFetch,
}

impl StoreKind {
    /// Label in the row text.
    fn label(self) -> &'static str {
        match self {
            StoreKind::Comments => "comments",
            StoreKind::Viewed => "viewed",
            StoreKind::UiState => "UI state",
            StoreKind::UpdateCheck => "update check",
            StoreKind::BackgroundFetch => "background fetch",
        }
    }

    /// The store's own read-only check of a present file's bytes.
    fn check_bytes(self, bytes: &[u8]) -> Result<(), String> {
        match self {
            StoreKind::Comments => check_comment_store_bytes(bytes),
            StoreKind::Viewed => check_viewed_store_bytes(bytes),
            StoreKind::UiState => check_ui_state_bytes(bytes),
            StoreKind::UpdateCheck => check_update_check_store_bytes(bytes),
            StoreKind::BackgroundFetch => check_background_fetch_store_bytes(bytes),
        }
    }
}

/// What a read-only look at a store file found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreRead {
    /// No file yet; the first save creates it.
    Missing,
    /// A file the store's save accepts.
    Valid,
    /// A file the store's save refuses (and leaves unchanged), or one that
    /// cannot be read; the text says why.
    Rejected(String),
}

/// One state file and what a read-only look found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreProbe {
    /// Which store.
    pub kind: StoreKind,
    /// Resolved path.
    pub path: PathBuf,
    /// What the read found.
    pub read: StoreRead,
}

/// Every input the checks need, as plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// OS family.
    pub os: Os,
    /// The environment variables the checks read ([`PROBE_ENV`]), as set.
    pub env: BTreeMap<String, String>,
    /// Commands found on `PATH` (or as a path), by the name the app runs.
    pub on_path: BTreeSet<String>,
    /// Terminal width in columns.
    pub term_cols: u16,
    /// Terminal height in rows.
    pub term_rows: u16,
    /// ASCII glyphs instead of Nerd Font glyphs (resolved setting).
    pub ascii: bool,
    /// App mouse capture is on.
    pub mouse_enabled: bool,
    /// This is a dev build ([`crate::DEV_BUILD`] is set).
    pub dev_build: bool,
    /// The TUI-startup release check setting (`updateCheck`).
    pub update_check: bool,
    /// The resolved git binary and its `--version`.
    pub git: GitProbe,
    /// `rustc --version` on a dev build; `None` on a release build.
    pub rustc: Option<Result<String, String>>,
    /// tmux options; `None` outside tmux (`TMUX` unset).
    pub tmux: Option<TmuxOptions>,
    /// Config `editor`.
    pub editor_config: Option<String>,
    /// Config `diffTool`.
    pub diff_tool_config: Option<String>,
    /// Forges of the loaded repos' remotes, each once.
    pub forges: Vec<Forge>,
    /// Config files.
    pub config: ConfigProbe,
    /// State files, one per [`StoreKind`].
    pub stores: Vec<StoreProbe>,
}

/// Environment variables [`collect`] copies into [`Probe::env`].
pub const PROBE_ENV: &[&str] = &[
    "LC_ALL",
    "LC_CTYPE",
    "LANG",
    "NO_COLOR",
    "COLORTERM",
    "TERM_PROGRAM",
    "TMUX",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "WORKSPACE_STATUS_GIT",
    "WSL_DISTRO_NAME",
    "VISUAL",
    "EDITOR",
    "BROWSER",
    "HOME",
];

impl Probe {
    /// Non-empty value of env var `key`.
    fn var(&self, key: &str) -> Option<&str> {
        self.env
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// True when `cmd` was found on `PATH`.
    fn has(&self, cmd: &str) -> bool {
        self.on_path.contains(cmd)
    }

    /// True on WSL (`WSL_DISTRO_NAME` set on Linux).
    fn wsl(&self) -> bool {
        self.os == Os::Linux && self.var("WSL_DISTRO_NAME").is_some()
    }

    /// `path` with `$HOME` shown as `~`.
    fn tilde(&self, path: &Path) -> String {
        if let Some(home) = self.var("HOME") {
            if let Ok(rest) = path.strip_prefix(home) {
                if rest.as_os_str().is_empty() {
                    return "~".to_string();
                }
                return format!("~/{}", rest.display());
            }
        }
        path.display().to_string()
    }

    /// `export NAME=value` (Unix) or `$env:NAME = "value"` (Windows).
    fn set_env_line(&self, name: &str, value: &str) -> String {
        if self.os == Os::Windows {
            format!("$env:{name} = \"{value}\"")
        } else {
            format!("export {name}={value}")
        }
    }
}

/// Row counts by status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Rows with [`CheckStatus::Ok`].
    pub ok: usize,
    /// Rows with [`CheckStatus::Warn`].
    pub warn: usize,
    /// Rows with [`CheckStatus::Fail`].
    pub fail: usize,
}

/// Count `checks` by status.
pub fn summary(checks: &[Check]) -> Summary {
    let mut out = Summary::default();
    for check in checks {
        match check.status {
            CheckStatus::Ok => out.ok += 1,
            CheckStatus::Warn => out.warn += 1,
            CheckStatus::Fail => out.fail += 1,
        }
    }
    out
}

/// Every check for `probe`, in group order (TERMINAL, TOOLS, FILES). Pure.
///
/// The UTF-8 row is left out on Windows, the curl row on a dev build, and
/// the Rust toolchain row on a release build.
pub fn checks(probe: &Probe) -> Vec<Check> {
    [
        Some(terminal_size(probe)),
        utf8_locale(probe),
        Some(colour(probe)),
        Some(nerd_font(probe)),
        Some(hyperlinks(probe)),
        Some(clipboard(probe)),
        Some(mouse(probe)),
        Some(git(probe)),
        Some(editor(probe)),
        Some(diff_tool(probe)),
        Some(browser(probe)),
        Some(forge_clis(probe)),
        curl(probe),
        rust_toolchain(probe),
        Some(config_files(probe)),
        Some(state_files(probe)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn row(
    group: CheckGroup,
    name: &'static str,
    status: CheckStatus,
    why: impl Into<String>,
    fix: Option<Fix>,
) -> Check {
    Check {
        group,
        name,
        status,
        why: why.into(),
        fix,
    }
}

fn hint(text: impl Into<String>) -> Option<Fix> {
    Some(Fix {
        hint: Some(text.into()),
        copy: None,
    })
}

fn hint_copy(text: impl Into<String>, copy: impl Into<String>) -> Option<Fix> {
    Some(Fix {
        hint: Some(text.into()),
        copy: Some(copy.into()),
    })
}

/* ── TERMINAL ───────────────────────────────────────────────────────────── */

fn terminal_size(p: &Probe) -> Check {
    const NAME: &str = "Terminal size";
    let (cols, rows) = (p.term_cols, p.term_rows);
    if cols < MIN_TERM_COLS || rows < MIN_TERM_ROWS {
        return row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Fail,
            format!(
                "{cols}×{rows}: need ≥ {MIN_TERM_COLS}×{MIN_TERM_ROWS}. Smaller, the panes do \
                 not paint and a resize notice shows instead."
            ),
            hint(format!(
                "Enlarge the terminal to at least {MIN_TERM_COLS}×{MIN_TERM_ROWS} \
                 ({SEARCH_PREVIEW_MIN_COLS}+ columns for the search preview)."
            )),
        );
    }
    if cols < SEARCH_PREVIEW_MIN_COLS {
        return row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Warn,
            format!(
                "{cols}×{rows}: search in files hides its file preview below \
                 {SEARCH_PREVIEW_MIN_COLS} columns."
            ),
            hint(format!(
                "Widen the terminal to {SEARCH_PREVIEW_MIN_COLS}+ columns."
            )),
        );
    }
    row(
        CheckGroup::Terminal,
        NAME,
        CheckStatus::Ok,
        format!("{cols}×{rows}"),
        None,
    )
}

/// Locale variables in the order the C library reads them for `LC_CTYPE`.
const LOCALE_VARS: [&str; 3] = ["LC_ALL", "LC_CTYPE", "LANG"];

fn utf8_locale(p: &Probe) -> Option<Check> {
    const NAME: &str = "UTF-8 locale";
    if p.os == Os::Windows {
        return None;
    }
    let effective = LOCALE_VARS
        .iter()
        .find_map(|key| p.var(key).map(|value| (*key, value)));
    if let Some((key, value)) = effective {
        let lower = value.to_ascii_lowercase();
        if lower.contains("utf-8") || lower.contains("utf8") {
            return Some(row(
                CheckGroup::Terminal,
                NAME,
                CheckStatus::Ok,
                format!("{key}={value}"),
                None,
            ));
        }
    }
    let found = match effective {
        Some((key, value)) => format!("{key}={value} is not UTF-8"),
        None => "LC_ALL, LC_CTYPE and LANG are unset".to_string(),
    };
    // The variable that decides the locale is the one to change: LANG does
    // nothing while LC_ALL or LC_CTYPE is set.
    let var = effective.map_or("LANG", |(key, _)| key);
    let value = if p.os == Os::Macos {
        "en_US.UTF-8"
    } else {
        "C.UTF-8"
    };
    Some(row(
        CheckGroup::Terminal,
        NAME,
        CheckStatus::Warn,
        format!(
            "{found}. The TUI draws box-drawing and block characters in both glyph modes \
             (dialog borders, the │ tab separator and diff rule, the ▌ cursor bar); tmux \
             swaps them for _ under a non-UTF-8 locale."
        ),
        hint_copy(
            "Set a UTF-8 locale in the shell profile, then restart the terminal.",
            p.set_env_line(var, value),
        ),
    ))
}

/// True when `COLORTERM` advertises 24-bit colour (same test as
/// [`crate::tui::undercurl::undercurl_enabled`]).
fn truecolor(colorterm: Option<&str>) -> bool {
    colorterm.is_some_and(|value| {
        let value = value.trim();
        value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
    })
}

fn colour(p: &Probe) -> Check {
    const NAME: &str = "Colour";
    if p.var("NO_COLOR").is_some() {
        return row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Ok,
            "NO_COLOR set: spinner, sparkline, toast, dialog backdrop and comment underline \
             paint plain.",
            None,
        );
    }
    let colorterm = p.var("COLORTERM");
    if truecolor(colorterm) {
        let value = colorterm.unwrap_or_default().trim();
        let why = if p.ascii {
            format!("COLORTERM={value}: RGB theme colours show as designed.")
        } else {
            format!(
                "COLORTERM={value}: RGB theme colours show as designed; commented lines get a \
                 curly underline."
            )
        };
        return row(CheckGroup::Terminal, NAME, CheckStatus::Ok, why, None);
    }
    let found = match colorterm {
        Some(value) => format!("COLORTERM={value}"),
        None => "COLORTERM is unset".to_string(),
    };
    let mut why =
        format!("{found}: the terminal did not advertise 24-bit colour. Theme colours are RGB");
    if p.ascii {
        why.push('.');
    } else {
        why.push_str(", and commented lines keep a plain underline instead of a curly one.");
    }
    let mut advice = "Set it only when the terminal really shows 24-bit colour.".to_string();
    if p.tmux.is_some() {
        advice.push_str(" Inside tmux also add set -as terminal-features \",*:RGB\" to tmux.conf.");
    }
    row(
        CheckGroup::Terminal,
        NAME,
        CheckStatus::Warn,
        why,
        hint_copy(advice, p.set_env_line("COLORTERM", "truecolor")),
    )
}

fn nerd_font(p: &Probe) -> Check {
    const NAME: &str = "Nerd Font glyphs";
    if p.ascii {
        return row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Ok,
            "ASCII markers on (WS_STATUS_GLYPHS or config glyphs = ascii): no Nerd Font needed.",
            None,
        );
    }
    let sample = [
        IconKind::Workspace,
        IconKind::Repo,
        IconKind::Branch,
        IconKind::Folder,
    ]
    .map(|kind| kind.glyph(false))
    .join(" ");
    let why = format!(
        "Nerd Font glyphs on; ws cannot see the terminal font. Check these: {sample} — boxes or \
         ? mean no Nerd Font."
    );
    let fix = if p.var("TERM_PROGRAM") == Some("vscode") {
        hint_copy(
            "VS Code / Cursor: add this to the User Settings (settings.json).",
            format!("\"terminal.integrated.fontFamily\": \"'{REQUIRED_FONT}', monospace\""),
        )
    } else {
        hint_copy(
            format!(
                "Install {REQUIRED_FONT} (romkatv/powerlevel10k-media) and select it in the \
                 terminal profile. Do not use MesloLGM Nerd Font Mono in VTE terminals \
                 (xfce4-terminal). No Nerd Font: use ASCII markers."
            ),
            p.set_env_line("WS_STATUS_GLYPHS", "ascii"),
        )
    };
    row(CheckGroup::Terminal, NAME, CheckStatus::Warn, why, fix)
}

fn hyperlinks(p: &Probe) -> Check {
    const NAME: &str = "Hyperlinks (OSC 8)";
    const COPY: &str = "set -as terminal-features \",*:hyperlinks\"";
    const ADD: &str = "Add to tmux.conf, then reload the tmux config.";
    let Some(tmux) = &p.tmux else {
        return row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Ok,
            "Sent on PR badges and commit hashes; the terminal cannot be asked whether \
             it honours OSC 8 — a terminal that ignores them shows plain text.",
            None,
        );
    };
    match &tmux.terminal_features {
        None => row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Warn,
            "Inside tmux, but could not read tmux options; tmux drops OSC 8 links unless \
             terminal-features has hyperlinks.",
            hint_copy(ADD, COPY),
        ),
        Some(features) if features.contains("hyperlinks") => row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Ok,
            "tmux terminal-features has a hyperlinks entry: tmux passes the OSC 8 links on PR \
             badges and commit hashes when that entry matches this terminal.",
            None,
        ),
        Some(_) => row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Warn,
            "tmux drops OSC 8 links (PR badges, commit hashes): terminal-features has \
             no hyperlinks entry.",
            hint_copy(ADD, COPY),
        ),
    }
}

fn clipboard(p: &Probe) -> Check {
    const NAME: &str = "Clipboard";
    let (status, mut why, fix) = match p.os {
        Os::Windows => (
            CheckStatus::Ok,
            "OSC 52; the app uses no host clipboard tool on Windows.".to_string(),
            None,
        ),
        Os::Macos if p.has("pbcopy") => (CheckStatus::Ok, "OSC 52 + pbcopy.".to_string(), None),
        Os::Macos => (
            CheckStatus::Warn,
            "Only OSC 52 (pbcopy is not on PATH); works when the terminal honours OSC 52."
                .to_string(),
            Some(Fix {
                hint: Some("pbcopy ships with macOS in /usr/bin; put /usr/bin on PATH.".into()),
                copy: None,
            }),
        ),
        Os::Linux => {
            let wayland = p.var("WAYLAND_DISPLAY").is_some();
            let x11 = p.var("DISPLAY").is_some();
            // Same order as `copy_to_clipboard`: wl-copy, then xclip.
            let tool = if wayland && p.has("wl-copy") {
                Some("wl-copy")
            } else if x11 && p.has("xclip") {
                Some("xclip")
            } else {
                None
            };
            match tool {
                Some(tool) => (CheckStatus::Ok, format!("OSC 52 + {tool}."), None),
                None => {
                    let why = "Only OSC 52; works when the terminal honours OSC 52.".to_string();
                    let fix = if wayland {
                        hint_copy(
                            "Install wl-clipboard for a host clipboard fallback.",
                            "sudo apt install wl-clipboard",
                        )
                    } else if x11 {
                        hint_copy(
                            "Install xclip for a host clipboard fallback.",
                            "sudo apt install xclip",
                        )
                    } else {
                        hint("No display: OSC 52 is the only path; use a terminal that honours OSC 52.")
                    };
                    (CheckStatus::Warn, why, fix)
                }
            }
        }
    };
    let Some(tmux) = &p.tmux else {
        return row(CheckGroup::Terminal, NAME, status, why, fix);
    };
    if tmux.set_clipboard.as_deref().map(str::trim) == Some("on") {
        why.push_str(" tmux set-clipboard is on.");
        return row(CheckGroup::Terminal, NAME, status, why, fix);
    }
    match tmux.set_clipboard.as_deref().map(str::trim) {
        Some(value) => why.push_str(&format!(
            " tmux set-clipboard is {value}: tmux does not pass the app's OSC 52 to the \
             terminal."
        )),
        None => why.push_str(
            " Could not read tmux set-clipboard; tmux passes OSC 52 on only when it is on.",
        ),
    }
    let mut advice = "Add to tmux.conf, then reload the tmux config.".to_string();
    if let Some(base) = fix {
        if let Some(text) = base.hint {
            advice.push(' ');
            advice.push_str(&text);
        }
        if let Some(copy) = base.copy {
            advice.push_str(&format!(" ({copy})"));
        }
    }
    row(
        CheckGroup::Terminal,
        NAME,
        status.max(CheckStatus::Warn),
        why,
        hint_copy(advice, "set -g set-clipboard on"),
    )
}

fn mouse(p: &Probe) -> Check {
    const NAME: &str = "Mouse";
    if !p.mouse_enabled {
        return row(
            CheckGroup::Terminal,
            NAME,
            CheckStatus::Ok,
            "Off; m turns it on.",
            None,
        );
    }
    if let Some(tmux) = &p.tmux {
        let value = tmux.mouse.as_deref().map(str::trim);
        if value != Some("on") {
            let found = match value {
                Some(value) => format!("tmux mouse is {value}"),
                None => "could not read tmux mouse".to_string(),
            };
            return row(
                CheckGroup::Terminal,
                NAME,
                CheckStatus::Warn,
                format!("On, but {found}: tmux does not forward clicks and the wheel."),
                hint_copy(
                    "Add to tmux.conf, then reload the tmux config.",
                    "set -g mouse on",
                ),
            );
        }
    }
    row(CheckGroup::Terminal, NAME, CheckStatus::Ok, "On.", None)
}

/* ── TOOLS ──────────────────────────────────────────────────────────────── */

fn git(p: &Probe) -> Check {
    const NAME: &str = "git";
    let path = &p.git.path;
    let version = match &p.git.version {
        Ok(version) => version,
        Err(err) => {
            let why = format!("{path} --version failed ({err}); every pane needs git.");
            let fix = if p.git.explicit {
                let value = if p.os == Os::Windows {
                    "git"
                } else {
                    "/usr/bin/git"
                };
                hint_copy(
                    format!(
                        "WORKSPACE_STATUS_GIT or config git names {path}; point it at a \
                         working git."
                    ),
                    p.set_env_line("WORKSPACE_STATUS_GIT", value),
                )
            } else {
                match p.os {
                    Os::Linux => hint_copy(
                        "Install git (or use the distribution's package manager).",
                        "sudo apt install git",
                    ),
                    Os::Macos => hint_copy(
                        "Install the Xcode command line tools (they include git).",
                        "xcode-select --install",
                    ),
                    Os::Windows => {
                        hint_copy("Install Git for Windows.", "winget install --id Git.Git -e")
                    }
                }
            };
            return row(CheckGroup::Tools, NAME, CheckStatus::Fail, why, fix);
        }
    };
    if p.wsl() && path.to_ascii_lowercase().ends_with(".exe") {
        return row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Warn,
            format!("{path} is a Windows git.exe under WSL; ws should run the Linux git."),
            hint_copy(
                "Point ws at the Linux git (install it first with sudo apt install git if \
                 /usr/bin/git is missing).",
                "export WORKSPACE_STATUS_GIT=/usr/bin/git",
            ),
        );
    }
    row(
        CheckGroup::Tools,
        NAME,
        CheckStatus::Ok,
        format!("{version} ({path})"),
        None,
    )
}

/// First argv token of a command string (the program the app runs).
fn program_of(command: &str) -> String {
    parse_editor_argv(command)
        .into_iter()
        .next()
        .unwrap_or_else(|| command.to_string())
}

/// Editors suggested when the resolved one is missing, in order.
const EDITOR_CANDIDATES: &[&str] = &["nvim", "vim", "nano", "code", "cursor"];

fn editor(p: &Probe) -> Check {
    const NAME: &str = "Editor (e)";
    let config = p
        .editor_config
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let resolved = resolve_editor(config, p.var("VISUAL"), p.var("EDITOR"));
    let program = program_of(&resolved);
    if p.has(&program) {
        return row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Ok,
            format!("e opens {program}."),
            None,
        );
    }
    let why = format!("e cannot start {program}: not found on PATH.");
    let windows_extra: &[&str] = if p.os == Os::Windows {
        &["notepad"]
    } else {
        &[]
    };
    let found = EDITOR_CANDIDATES
        .iter()
        .chain(windows_extra)
        .find(|cmd| p.has(cmd));
    let base = "Install it, or set editor in the user config or EDITOR.";
    // The copy line sets the source `resolve_editor` reads first, so it wins.
    let fix = match found {
        None => hint(base),
        Some(cmd) if config.is_some() => hint_copy(
            format!("{base} The user config editor key wins over EDITOR."),
            format!("\"editor\": \"{cmd}\""),
        ),
        Some(cmd) if p.var("VISUAL").is_some() => hint_copy(base, p.set_env_line("VISUAL", cmd)),
        Some(cmd) => hint_copy(base, p.set_env_line("EDITOR", cmd)),
    };
    row(CheckGroup::Tools, NAME, CheckStatus::Warn, why, fix)
}

fn diff_tool(p: &Probe) -> Check {
    const NAME: &str = "Diff tool (E)";
    let resolved = resolve_diff_tool(p.diff_tool_config.as_deref());
    let program = program_of(&resolved);
    if p.has(&program) {
        return row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Ok,
            format!("E opens {program}."),
            None,
        );
    }
    let why = format!("E cannot start {program}: not found on PATH.");
    let gui = ["code", "cursor"].into_iter().find(|cmd| p.has(cmd));
    let fix = match gui {
        Some(cmd) => hint_copy(
            "Add to the user config file.",
            format!("\"diffTool\": \"{cmd} --diff --wait\""),
        ),
        None => hint("Install vim (for vimdiff), or set diffTool in the user config."),
    };
    row(CheckGroup::Tools, NAME, CheckStatus::Warn, why, fix)
}

fn browser(p: &Probe) -> Check {
    const NAME: &str = "Browser opener (gx, Open PR)";
    let program = browser_program(p.os, p.var("BROWSER"));
    if p.has(&program) {
        return row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Ok,
            format!("gx and Open PR use {program}."),
            None,
        );
    }
    let why = format!("gx and Open PR cannot start {program}: not found on PATH.");
    let fix = if p.wsl() && p.has("wslview") {
        hint_copy(
            "On WSL, wslview opens the Windows browser.",
            "export BROWSER=wslview",
        )
    } else if p.var("BROWSER").is_some() {
        hint("BROWSER names a missing command; point it at a browser or unset it.")
    } else if p.os == Os::Linux {
        hint_copy(
            "Install xdg-utils (xdg-open), or set BROWSER.",
            "sudo apt install xdg-utils",
        )
    } else {
        hint(format!("{program} is the system opener; check PATH."))
    };
    row(CheckGroup::Tools, NAME, CheckStatus::Warn, why, fix)
}

/// Program [`browser_command`] runs for `BROWSER` on `os`.
fn browser_program(os: Os, browser_env: Option<&str>) -> String {
    browser_command("https://example.com", browser_env, os.consts_name())
        .argv
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// The CLI the PR badges run for `forge`.
fn forge_cli(forge: Forge) -> &'static str {
    match forge {
        Forge::GitHub => "gh",
        Forge::GitLab => "glab",
    }
}

fn forge_name(forge: Forge) -> &'static str {
    match forge {
        Forge::GitHub => "GitHub",
        Forge::GitLab => "GitLab",
    }
}

fn forge_clis(p: &Probe) -> Check {
    const NAME: &str = "Forge CLIs (PR badges)";
    if p.forges.is_empty() {
        return row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Ok,
            "No GitHub or GitLab remotes.",
            None,
        );
    }
    let missing: Vec<Forge> = p
        .forges
        .iter()
        .copied()
        .filter(|forge| !p.has(forge_cli(*forge)))
        .collect();
    if missing.is_empty() {
        let clis: Vec<&str> = p.forges.iter().map(|forge| forge_cli(*forge)).collect();
        return row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Ok,
            format!(
                "PR badges use {} (login is not checked).",
                clis.join(" and ")
            ),
            None,
        );
    }
    let why = missing
        .iter()
        .map(|forge| {
            format!(
                "PR badges for {} remotes need {}.",
                forge_name(*forge),
                forge_cli(*forge)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let fix = match p.os {
        Os::Macos => hint_copy(
            "Install with Homebrew, then log in.",
            format!(
                "brew install {}",
                missing
                    .iter()
                    .map(|forge| forge_cli(*forge))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ),
        Os::Windows => hint_copy(
            "Install with winget, then log in.",
            missing
                .iter()
                .map(|forge| match forge {
                    Forge::GitHub => "winget install --id GitHub.cli",
                    Forge::GitLab => "winget install glab.glab",
                })
                .collect::<Vec<_>>()
                .join("; "),
        ),
        Os::Linux => hint(format!(
            "Install from {}, then log in.",
            missing
                .iter()
                .map(|forge| match forge {
                    Forge::GitHub => "https://cli.github.com",
                    Forge::GitLab => "https://gitlab.com/gitlab-org/cli",
                })
                .collect::<Vec<_>>()
                .join(" and ")
        )),
    };
    row(CheckGroup::Tools, NAME, CheckStatus::Warn, why, fix)
}

fn curl(p: &Probe) -> Option<Check> {
    const NAME: &str = "curl (release check)";
    if p.dev_build {
        return None;
    }
    if p.has("curl") {
        return Some(row(
            CheckGroup::Tools,
            NAME,
            CheckStatus::Ok,
            "Found.",
            None,
        ));
    }
    let fix = match p.os {
        Os::Linux => hint_copy("Install curl.", "sudo apt install curl"),
        Os::Macos => hint("macOS ships curl in /usr/bin; check PATH."),
        Os::Windows => hint("Windows 10 and later ship curl.exe in System32; check PATH."),
    };
    Some(row(
        CheckGroup::Tools,
        NAME,
        CheckStatus::Warn,
        "The startup release check and ws --update notes need curl.",
        fix,
    ))
}

/// Oldest rustc that builds the crate (`rust-version` in Cargo.toml).
const MIN_RUSTC: (u64, u64) = (1, 85);

/// `(major, minor)` from `rustc 1.85.0 (…)`.
fn rustc_version(text: &str) -> Option<(u64, u64)> {
    let version = text.split_whitespace().nth(1)?;
    let mut parts = version.split(['.', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

fn rust_toolchain(p: &Probe) -> Option<Check> {
    const NAME: &str = "Rust toolchain (dev build)";
    if !p.dev_build {
        return None;
    }
    let found = match &p.rustc {
        Some(Ok(text)) => match rustc_version(text) {
            Some(version) if version >= MIN_RUSTC => {
                return Some(row(
                    CheckGroup::Tools,
                    NAME,
                    CheckStatus::Ok,
                    text.clone(),
                    None,
                ))
            }
            _ => format!("found {text}"),
        },
        Some(Err(err)) => format!("rustc did not run ({err})"),
        None => "rustc was not checked".to_string(),
    };
    Some(row(
        CheckGroup::Tools,
        NAME,
        CheckStatus::Warn,
        format!(
            "ws-dev --update rebuilds from source and needs rustc {}.{}+; {found}.",
            MIN_RUSTC.0, MIN_RUSTC.1
        ),
        hint_copy("Update the stable toolchain.", "rustup update stable"),
    ))
}

/* ── FILES ──────────────────────────────────────────────────────────────── */

/// A character that can continue a path component.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '~')
}

/// `text` with each whole-path occurrence of `home` shown as `~`.
///
/// An occurrence counts only when it starts a path (not after a name
/// character or a separator) and ends at a separator or where the path
/// ends, so `HOME=/home/u` leaves `/home/user2/x` alone.
fn tilde_text(text: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    for (at, _) in text.match_indices(home) {
        if at < rest {
            continue;
        }
        let before = text[..at].chars().next_back();
        let after = text[at + home.len()..].chars().next();
        let starts = before.is_none_or(|c| !is_name_char(c) && c != '/' && c != '\\');
        let ends = after.is_none_or(|c| c == '/' || c == '\\' || !is_name_char(c));
        if starts && ends {
            out.push_str(&text[rest..at]);
            out.push('~');
            rest = at + home.len();
        }
    }
    out.push_str(&text[rest..]);
    out
}

fn config_files(p: &Probe) -> Check {
    const NAME: &str = "Config files";
    let describe = |file: &FileProbe| {
        let state = if file.present {
            "loaded"
        } else {
            "not present"
        };
        format!("{} {state}", p.tilde(&file.path))
    };
    if let Some(error) = &p.config.error {
        let text = match p.var("HOME") {
            Some(home) => tilde_text(error, home),
            None => error.clone(),
        };
        return row(
            CheckGroup::Files,
            NAME,
            CheckStatus::Fail,
            format!("{text}."),
            hint("Fix the key the error names."),
        );
    }
    let user = match &p.config.user {
        Some(file) => describe(file),
        None => "user config: no path (XDG_CONFIG_HOME and HOME unset)".to_string(),
    };
    row(
        CheckGroup::Files,
        NAME,
        CheckStatus::Ok,
        format!("{user}; {}.", describe(&p.config.workspace)),
        None,
    )
}

/// `'text'` for a POSIX shell.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// `'text'` for PowerShell.
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn state_files(p: &Probe) -> Check {
    const NAME: &str = "State files";
    // The update-check store is read and saved only by the startup release
    // check, which a dev build or `updateCheck` off never runs.
    let in_use = |store: &&StoreProbe| {
        store.kind != StoreKind::UpdateCheck || (p.update_check && !p.dev_build)
    };
    let stores: Vec<&StoreProbe> = p.stores.iter().filter(in_use).collect();
    let rejected: Vec<(&StoreProbe, &str)> = stores
        .iter()
        .filter_map(|store| match &store.read {
            StoreRead::Rejected(err) => Some((*store, err.as_str())),
            _ => None,
        })
        .collect();
    if let Some((first, _)) = rejected.first() {
        let why = rejected
            .iter()
            .map(|(store, err)| {
                format!(
                    "{} ({}): {err}; {} saves will fail, the file is left unchanged.",
                    store.kind.label(),
                    p.tilde(&store.path),
                    store.kind.label()
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let path = first.path.display().to_string();
        let backup = format!("{path}.bak");
        let copy = if p.os == Os::Windows {
            format!("Move-Item {} {}", ps_quote(&path), ps_quote(&backup))
        } else {
            format!("mv {} {}", sh_quote(&path), sh_quote(&backup))
        };
        let mut advice = "Move the file aside; the next save starts a new one.".to_string();
        if rejected.len() > 1 {
            advice.push_str(" Do the same for each store named above.");
        }
        return row(
            CheckGroup::Files,
            NAME,
            CheckStatus::Fail,
            why,
            hint_copy(advice, copy),
        );
    }
    let why = stores
        .iter()
        .map(|store| {
            let state = match store.read {
                StoreRead::Missing => "created on first save",
                _ => "ok",
            };
            format!("{} {state} ({})", store.kind.label(), p.tilde(&store.path))
        })
        .collect::<Vec<_>>()
        .join("; ");
    row(
        CheckGroup::Files,
        NAME,
        CheckStatus::Ok,
        format!("{why}."),
        None,
    )
}

/* ── collect ────────────────────────────────────────────────────────────── */

/// What only the app knows, for [`collect`].
#[derive(Clone, Copy, Debug)]
pub struct CollectInputs<'a> {
    /// Terminal width in columns.
    pub term_cols: u16,
    /// Terminal height in rows.
    pub term_rows: u16,
    /// The resolved settings (glyphs, git binary, store paths).
    pub settings: &'a Settings,
    /// App mouse capture is on.
    pub mouse_enabled: bool,
    /// Config `editor`.
    pub editor_config: Option<&'a str>,
    /// Config `diffTool`.
    pub diff_tool_config: Option<&'a str>,
    /// Remote URLs of the loaded repos (any order, duplicates allowed).
    pub remote_urls: &'a [String],
    /// Workspace root, where [`CONFIG_FILENAME`] lives.
    pub workspace_root: &'a Path,
    /// User config path ([`crate::config::user_config_path`]).
    pub user_config: Option<&'a Path>,
}

/// Commands the checks look up on `PATH` besides the editor, diff tool, and
/// browser programs.
const LOOKUP: &[&str] = &[
    "wl-copy", "xclip", "pbcopy", "nvim", "vim", "nano", "code", "cursor", "notepad", "gh", "glab",
    "curl", "wslview",
];

/// Build the [`Probe`] from the real environment, `PATH`, read-only
/// commands, and the config and store files.
///
/// Read-only: it never writes a file or the terminal. Spawned commands get
/// a null stdin and captured stdout / stderr. It blocks, so call it on a
/// blocking worker thread.
pub fn collect(inputs: &CollectInputs<'_>) -> Probe {
    collect_with(inputs, |key| std::env::var_os(key))
}

/// [`collect`] with an injected env lookup.
fn collect_with<F>(inputs: &CollectInputs<'_>, get: F) -> Probe
where
    F: Fn(&str) -> Option<OsString>,
{
    let os = Os::current();
    let env: BTreeMap<String, String> = PROBE_ENV
        .iter()
        .filter_map(|key| {
            get(key).map(|value| (key.to_string(), value.to_string_lossy().into_owned()))
        })
        .collect();
    let var = |key: &str| env.get(key).map(String::as_str).filter(|v| !v.is_empty());

    let path_var = get("PATH");
    let editor_program = program_of(&resolve_editor(
        inputs.editor_config.filter(|v| !v.trim().is_empty()),
        var("VISUAL"),
        var("EDITOR"),
    ));
    let diff_program = program_of(&resolve_diff_tool(inputs.diff_tool_config));
    let browser = browser_program(os, var("BROWSER"));
    let on_path: BTreeSet<String> = LOOKUP
        .iter()
        .map(|cmd| cmd.to_string())
        .chain([editor_program, diff_program, browser])
        .filter(|cmd| find_on_path(cmd, path_var.as_deref(), os).is_some())
        .collect();

    let settings = inputs.settings;
    let config_result = load_config_files(inputs.user_config, inputs.workspace_root);
    let config_git = config_result
        .as_ref()
        .ok()
        .is_some_and(|config| config.runtime.git.is_some());
    let explicit = var("WORKSPACE_STATUS_GIT").is_some()
        || config_git
        // The reload failed: a git that is not the default came from config.
        || (config_result.is_err() && settings.git != resolve_git_binary(None, None));
    let git_path = settings.git.display().to_string();
    let git = GitProbe {
        version: run_read_only(settings.git.as_os_str(), &["--version"]),
        path: git_path,
        explicit,
    };
    let dev_build = crate::DEV_BUILD.is_some();
    let rustc = dev_build.then(|| run_read_only("rustc".as_ref(), &["--version"]));
    let tmux = var("TMUX").map(|_| TmuxOptions {
        terminal_features: tmux_option("terminal-features"),
        set_clipboard: tmux_option("set-clipboard"),
        mouse: tmux_option("mouse"),
    });

    let mut forges = Vec::new();
    for url in inputs.remote_urls {
        if let Some(repo) = parse_remote(url) {
            if !forges.contains(&repo.forge) {
                forges.push(repo.forge);
            }
        }
    }

    let file = |path: &Path| FileProbe {
        path: path.to_path_buf(),
        present: fs::symlink_metadata(path).is_ok(),
    };
    let config = ConfigProbe {
        user: inputs.user_config.map(file),
        workspace: file(&inputs.workspace_root.join(CONFIG_FILENAME)),
        error: config_result.err(),
    };
    let stores = [
        (StoreKind::Comments, &settings.comment_store),
        (StoreKind::Viewed, &settings.viewed_store),
        (StoreKind::UiState, &settings.ui_store),
        (StoreKind::UpdateCheck, &settings.update_check_store),
        (StoreKind::BackgroundFetch, &settings.fetch_store),
    ]
    .into_iter()
    .map(|(kind, path)| StoreProbe {
        kind,
        path: path.clone(),
        read: read_store(kind, path),
    })
    .collect();

    Probe {
        os,
        env,
        on_path,
        term_cols: inputs.term_cols,
        term_rows: inputs.term_rows,
        ascii: settings.ascii,
        mouse_enabled: inputs.mouse_enabled,
        dev_build,
        update_check: settings.update_check,
        git,
        rustc,
        tmux,
        editor_config: inputs.editor_config.map(str::to_string),
        diff_tool_config: inputs.diff_tool_config.map(str::to_string),
        forges,
        config,
        stores,
    }
}

/// Read a store file without changing it and run the store's own check.
fn read_store(kind: StoreKind, path: &Path) -> StoreRead {
    match fs::read(path) {
        Ok(bytes) => match kind.check_bytes(&bytes) {
            Ok(()) => StoreRead::Valid,
            Err(err) => StoreRead::Rejected(err),
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => StoreRead::Missing,
        Err(err) => StoreRead::Rejected(err.to_string()),
    }
}

/// `tmux show-options -gv <name>`; `None` when it fails.
fn tmux_option(name: &str) -> Option<String> {
    run_read_only("tmux".as_ref(), &["show-options", "-gv", name]).ok()
}

/// Run a read-only command with a null stdin and captured output.
///
/// `Ok` is the trimmed stdout of a successful run (the first line for a
/// one-line answer, every line for a list). `Err` is the first stderr line,
/// the exit status, or the spawn error.
fn run_read_only(program: &std::ffi::OsStr, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(|| output.status.to_string(), str::to_string))
}

/// True when the file name of `cmd` (after the last separator) has an
/// extension, as `Path::extension` reads it: a `.` that is not the first
/// character.
fn has_extension(cmd: &str, os: Os) -> bool {
    let name = if os == Os::Windows {
        cmd.rsplit(['/', '\\']).next()
    } else {
        cmd.rsplit('/').next()
    };
    name.and_then(|name| name.rfind('.'))
        .is_some_and(|dot| dot > 0)
}

/// Find `cmd` the way Rust std `Command::new` starts it, so a found
/// program is one the app can really spawn.
///
/// A name with a path separator is checked as a path; a bare name is looked
/// up in each `PATH` entry. On Windows, std does not use `PATHEXT`: a bare
/// name with no extension is tried only as `<name>.exe` (so a `code.cmd`
/// shim does not count), a name with an extension is used as is, and a path
/// with no extension gets `.exe` only when the bare path is not there.
fn find_on_path(cmd: &str, path_var: Option<&std::ffi::OsStr>, os: Os) -> Option<PathBuf> {
    if cmd.is_empty() {
        return None;
    }
    let windows = os == Os::Windows;
    let has_separator = cmd.contains('/') || (windows && cmd.contains('\\'));
    let exe_name = (windows && !has_extension(cmd, os)).then(|| format!("{cmd}.exe"));
    if has_separator {
        let bare = PathBuf::from(cmd);
        if is_executable(&bare) {
            return Some(bare);
        }
        return exe_name
            .map(PathBuf::from)
            .filter(|candidate| is_executable(candidate));
    }
    let name = exe_name.unwrap_or_else(|| cmd.to_string());
    std::env::split_paths(path_var?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(&name))
        .find(|candidate| is_executable(candidate))
}

/// A regular file with an execute bit (Unix), or a regular file (Windows).
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeKeys;
    use crate::testutil::unique_dir;
    use std::time::SystemTime;

    /// A probe where every row is Ok on Linux outside tmux.
    fn base(os: Os) -> Probe {
        let mut env = BTreeMap::new();
        env.insert("LANG".to_string(), "C.UTF-8".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("HOME".to_string(), "/home/u".to_string());
        env.insert("DISPLAY".to_string(), ":0".to_string());
        Probe {
            os,
            env,
            on_path: [
                "vim", "vimdiff", "xdg-open", "open", "cmd", "xclip", "pbcopy", "curl",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            term_cols: 120,
            term_rows: 40,
            ascii: true,
            mouse_enabled: true,
            dev_build: false,
            update_check: true,
            git: GitProbe {
                path: "/usr/bin/git".to_string(),
                explicit: false,
                version: Ok("git version 2.43.0".to_string()),
            },
            rustc: None,
            tmux: None,
            editor_config: None,
            diff_tool_config: None,
            forges: Vec::new(),
            config: ConfigProbe {
                user: Some(FileProbe {
                    path: PathBuf::from("/home/u/.config/my-workspace-status/config.json"),
                    present: true,
                }),
                workspace: FileProbe {
                    path: PathBuf::from("/w/.workspace-status-config.json"),
                    present: false,
                },
                error: None,
            },
            stores: [
                (StoreKind::Comments, "comments.json"),
                (StoreKind::Viewed, "viewed-files.json"),
                (StoreKind::UiState, "ui-state.json"),
                (StoreKind::UpdateCheck, "update-check.json"),
                (StoreKind::BackgroundFetch, "background-fetch.json"),
            ]
            .into_iter()
            .map(|(kind, name)| StoreProbe {
                kind,
                path: PathBuf::from("/home/u/.local/state/my-workspace-status").join(name),
                read: StoreRead::Missing,
            })
            .collect(),
        }
    }

    fn set(p: &mut Probe, key: &str, value: &str) {
        p.env.insert(key.to_string(), value.to_string());
    }

    fn unset(p: &mut Probe, key: &str) {
        p.env.remove(key);
    }

    fn find<'a>(checks: &'a [Check], name: &str) -> Option<&'a Check> {
        checks.iter().find(|check| check.name == name)
    }

    fn get(p: &Probe, name: &str) -> Check {
        find(&checks(p), name)
            .unwrap_or_else(|| panic!("no {name} row"))
            .clone()
    }

    fn copy(check: &Check) -> Option<&str> {
        check.fix.as_ref().and_then(|fix| fix.copy.as_deref())
    }

    fn hint_of(check: &Check) -> &str {
        check
            .fix
            .as_ref()
            .and_then(|fix| fix.hint.as_deref())
            .unwrap_or_default()
    }

    fn in_tmux(p: &mut Probe, features: Option<&str>, clip: Option<&str>, mouse: Option<&str>) {
        set(p, "TMUX", "/tmp/tmux-1000/default,1,0");
        p.tmux = Some(TmuxOptions {
            terminal_features: features.map(str::to_string),
            set_clipboard: clip.map(str::to_string),
            mouse: mouse.map(str::to_string),
        });
    }

    #[test]
    fn base_probe_is_all_ok_in_group_order() {
        let got = checks(&base(Os::Linux));
        assert!(
            got.iter().all(|check| check.status == CheckStatus::Ok),
            "{got:#?}"
        );
        let names: Vec<&str> = got.iter().map(|check| check.name).collect();
        assert_eq!(
            names,
            [
                "Terminal size",
                "UTF-8 locale",
                "Colour",
                "Nerd Font glyphs",
                "Hyperlinks (OSC 8)",
                "Clipboard",
                "Mouse",
                "git",
                "Editor (e)",
                "Diff tool (E)",
                "Browser opener (gx, Open PR)",
                "Forge CLIs (PR badges)",
                "curl (release check)",
                "Config files",
                "State files",
            ]
        );
        let groups: Vec<CheckGroup> = got.iter().map(|check| check.group).collect();
        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "rows paint in group order");
    }

    #[test]
    fn terminal_size_fail_warn_ok() {
        let mut p = base(Os::Linux);
        p.term_cols = MIN_TERM_COLS - 1;
        let check = get(&p, "Terminal size");
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.why.contains("need ≥ 46×12"), "{}", check.why);
        p.term_cols = 80;
        p.term_rows = MIN_TERM_ROWS - 1;
        assert_eq!(get(&p, "Terminal size").status, CheckStatus::Fail);
        p.term_rows = MIN_TERM_ROWS;
        let check = get(&p, "Terminal size");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.why.contains("search in files"), "{}", check.why);
        assert!(hint_of(&check).contains("100+ columns"));
        p.term_cols = SEARCH_PREVIEW_MIN_COLS;
        assert_eq!(get(&p, "Terminal size").status, CheckStatus::Ok);
    }

    #[test]
    fn locale_row_is_absent_on_windows() {
        assert!(find(&checks(&base(Os::Windows)), "UTF-8 locale").is_none());
        assert!(find(&checks(&base(Os::Macos)), "UTF-8 locale").is_some());
    }

    #[test]
    fn locale_utf8_ok_else_warn_with_os_line() {
        let mut p = base(Os::Linux);
        set(&mut p, "LANG", "en_US.utf8");
        assert_eq!(get(&p, "UTF-8 locale").status, CheckStatus::Ok);
        set(&mut p, "LANG", "POSIX");
        let check = get(&p, "UTF-8 locale");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.why.contains("tmux"), "{}", check.why);
        assert_eq!(copy(&check), Some("export LANG=C.UTF-8"));
        unset(&mut p, "LANG");
        let check = get(&p, "UTF-8 locale");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.why.contains("unset"), "{}", check.why);
        let mut mac = base(Os::Macos);
        unset(&mut mac, "LANG");
        assert_eq!(
            copy(&get(&mac, "UTF-8 locale")),
            Some("export LANG=en_US.UTF-8")
        );
    }

    #[test]
    fn locale_lc_all_wins_and_is_the_var_to_fix() {
        let mut p = base(Os::Linux);
        set(&mut p, "LC_ALL", "C");
        let check = get(&p, "UTF-8 locale");
        assert_eq!(check.status, CheckStatus::Warn, "LC_ALL beats a UTF-8 LANG");
        assert_eq!(copy(&check), Some("export LC_ALL=C.UTF-8"));
        set(&mut p, "LC_ALL", "");
        assert_eq!(
            get(&p, "UTF-8 locale").status,
            CheckStatus::Ok,
            "empty skips"
        );
    }

    #[test]
    fn colour_no_color_is_intentional_ok() {
        let mut p = base(Os::Linux);
        unset(&mut p, "COLORTERM");
        set(&mut p, "NO_COLOR", "1");
        let check = get(&p, "Colour");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.why.starts_with("NO_COLOR set"), "{}", check.why);
    }

    #[test]
    fn colour_truecolor_ok_else_warn_with_os_line() {
        let mut p = base(Os::Linux);
        set(&mut p, "COLORTERM", "24BIT");
        assert_eq!(get(&p, "Colour").status, CheckStatus::Ok);
        set(&mut p, "NO_COLOR", "");
        set(&mut p, "COLORTERM", "256");
        let check = get(&p, "Colour");
        assert_eq!(check.status, CheckStatus::Warn, "empty NO_COLOR is unset");
        assert_eq!(copy(&check), Some("export COLORTERM=truecolor"));
        assert!(!hint_of(&check).contains("tmux"));
        in_tmux(&mut p, Some("hyperlinks"), Some("on"), Some("on"));
        assert!(hint_of(&get(&p, "Colour")).contains("terminal-features \",*:RGB\""));
        let mut win = base(Os::Windows);
        unset(&mut win, "COLORTERM");
        assert_eq!(
            copy(&get(&win, "Colour")),
            Some("$env:COLORTERM = \"truecolor\"")
        );
        p.ascii = false;
        assert!(get(&p, "Colour").why.contains("curly"));
    }

    #[test]
    fn nerd_font_ascii_ok_nerd_always_warn() {
        let mut p = base(Os::Linux);
        let check = get(&p, "Nerd Font glyphs");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.why.contains("ASCII markers on"));
        p.ascii = false;
        let check = get(&p, "Nerd Font glyphs");
        assert_eq!(check.status, CheckStatus::Warn, "Nerd mode is never Ok");
        assert!(check.why.contains(IconKind::Repo.glyph(false)));
        assert!(check.why.contains("boxes or ? mean no Nerd Font"));
        assert_eq!(copy(&check), Some("export WS_STATUS_GLYPHS=ascii"));
        assert!(hint_of(&check).contains("MesloLGM Nerd Font Mono"));
        set(&mut p, "TERM_PROGRAM", "vscode");
        let check = get(&p, "Nerd Font glyphs");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(
            copy(&check),
            Some("\"terminal.integrated.fontFamily\": \"'MesloLGS NF', monospace\"")
        );
        let mut win = base(Os::Windows);
        win.ascii = false;
        assert_eq!(
            copy(&get(&win, "Nerd Font glyphs")),
            Some("$env:WS_STATUS_GLYPHS = \"ascii\"")
        );
    }

    #[test]
    fn hyperlinks_outside_tmux_ok_inside_by_option() {
        let mut p = base(Os::Linux);
        let check = get(&p, "Hyperlinks (OSC 8)");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.why.contains("cannot be asked"));
        in_tmux(
            &mut p,
            Some("xterm*:clipboard:hyperlinks"),
            Some("on"),
            Some("on"),
        );
        assert_eq!(get(&p, "Hyperlinks (OSC 8)").status, CheckStatus::Ok);
        in_tmux(&mut p, Some("xterm*:clipboard"), Some("on"), Some("on"));
        let check = get(&p, "Hyperlinks (OSC 8)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(
            copy(&check),
            Some("set -as terminal-features \",*:hyperlinks\"")
        );
        in_tmux(&mut p, None, Some("on"), Some("on"));
        let check = get(&p, "Hyperlinks (OSC 8)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.why.contains("could not read tmux options"));
    }

    #[test]
    fn clipboard_windows_ok_without_tools() {
        let mut p = base(Os::Windows);
        p.on_path.clear();
        let check = get(&p, "Clipboard");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.why.contains("no host clipboard tool"));
    }

    #[test]
    fn clipboard_linux_tool_or_install_line_by_display() {
        let mut p = base(Os::Linux);
        assert!(get(&p, "Clipboard").why.contains("xclip"));
        set(&mut p, "WAYLAND_DISPLAY", "wayland-0");
        p.on_path.insert("wl-copy".to_string());
        assert!(get(&p, "Clipboard").why.contains("wl-copy"));
        p.on_path.remove("wl-copy");
        p.on_path.remove("xclip");
        let check = get(&p, "Clipboard");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), Some("sudo apt install wl-clipboard"));
        unset(&mut p, "WAYLAND_DISPLAY");
        assert_eq!(copy(&get(&p, "Clipboard")), Some("sudo apt install xclip"));
        unset(&mut p, "DISPLAY");
        p.on_path.insert("xclip".to_string());
        let check = get(&p, "Clipboard");
        assert_eq!(check.status, CheckStatus::Warn, "xclip needs DISPLAY");
        assert_eq!(copy(&check), None);
        assert!(hint_of(&check).contains("No display"));
        assert!(!check.why.contains("pbcopy"));
    }

    #[test]
    fn clipboard_macos_pbcopy() {
        let mut p = base(Os::Macos);
        assert_eq!(get(&p, "Clipboard").status, CheckStatus::Ok);
        p.on_path.remove("pbcopy");
        let check = get(&p, "Clipboard");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(!hint_of(&check).contains("apt"));
    }

    #[test]
    fn clipboard_tmux_set_clipboard() {
        let mut p = base(Os::Windows);
        in_tmux(&mut p, Some("hyperlinks"), Some("on"), Some("on"));
        assert_eq!(get(&p, "Clipboard").status, CheckStatus::Ok);
        for value in [Some("external"), Some("off"), None] {
            in_tmux(&mut p, Some("hyperlinks"), value, Some("on"));
            let check = get(&p, "Clipboard");
            assert_eq!(check.status, CheckStatus::Warn, "{value:?}");
            assert_eq!(copy(&check), Some("set -g set-clipboard on"));
        }
        let mut linux = base(Os::Linux);
        unset(&mut linux, "DISPLAY");
        in_tmux(&mut linux, Some("hyperlinks"), Some("off"), Some("on"));
        let check = get(&linux, "Clipboard");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(hint_of(&check).contains("No display"));
    }

    #[test]
    fn mouse_off_on_and_tmux() {
        let mut p = base(Os::Linux);
        p.mouse_enabled = false;
        in_tmux(&mut p, Some("hyperlinks"), Some("on"), Some("off"));
        let check = get(&p, "Mouse");
        assert_eq!(check.status, CheckStatus::Ok, "app mouse off");
        assert!(check.why.contains("m turns it on"));
        p.mouse_enabled = true;
        let check = get(&p, "Mouse");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), Some("set -g mouse on"));
        in_tmux(&mut p, Some("hyperlinks"), Some("on"), None);
        assert_eq!(get(&p, "Mouse").status, CheckStatus::Warn);
        in_tmux(&mut p, Some("hyperlinks"), Some("on"), Some("on"));
        assert_eq!(get(&p, "Mouse").status, CheckStatus::Ok);
    }

    #[test]
    fn git_missing_fails_with_per_os_install_line() {
        for (os, line) in [
            (Os::Linux, "sudo apt install git"),
            (Os::Macos, "xcode-select --install"),
            (Os::Windows, "winget install --id Git.Git -e"),
        ] {
            let mut p = base(os);
            p.git.version = Err("No such file or directory".to_string());
            let check = get(&p, "git");
            assert_eq!(check.status, CheckStatus::Fail, "{os:?}");
            assert_eq!(copy(&check), Some(line), "{os:?}");
        }
    }

    #[test]
    fn git_explicit_path_points_at_working_git() {
        let mut p = base(Os::Linux);
        p.git = GitProbe {
            path: "/opt/broken/git".to_string(),
            explicit: true,
            version: Err("permission denied".to_string()),
        };
        let check = get(&p, "git");
        assert_eq!(check.status, CheckStatus::Fail);
        assert_eq!(
            copy(&check),
            Some("export WORKSPACE_STATUS_GIT=/usr/bin/git")
        );
        assert!(hint_of(&check).contains("point it at a working git"));
        p.os = Os::Windows;
        assert_eq!(
            copy(&get(&p, "git")),
            Some("$env:WORKSPACE_STATUS_GIT = \"git\"")
        );
    }

    #[test]
    fn git_wsl_exe_warns_and_version_ok() {
        let mut p = base(Os::Linux);
        let check = get(&p, "git");
        assert!(check.why.contains("git version 2.43.0"));
        p.git.path = "/mnt/c/Program Files/Git/cmd/git.exe".to_string();
        assert_eq!(get(&p, "git").status, CheckStatus::Ok, "not WSL");
        set(&mut p, "WSL_DISTRO_NAME", "Ubuntu");
        let check = get(&p, "git");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(
            copy(&check),
            Some("export WORKSPACE_STATUS_GIT=/usr/bin/git")
        );
    }

    #[test]
    fn editor_found_or_warn_with_first_candidate() {
        let mut p = base(Os::Linux);
        let check = get(&p, "Editor (e)");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.why.contains("vim"));
        set(&mut p, "EDITOR", "hx --vsplit");
        p.on_path.insert("nano".to_string());
        p.on_path.insert("code".to_string());
        let check = get(&p, "Editor (e)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.why.contains("cannot start hx"), "{}", check.why);
        assert_eq!(copy(&check), Some("export EDITOR=vim"));
        set(&mut p, "VISUAL", "hx");
        assert_eq!(copy(&get(&p, "Editor (e)")), Some("export VISUAL=vim"));
        p.editor_config = Some("hx".to_string());
        assert_eq!(copy(&get(&p, "Editor (e)")), Some("\"editor\": \"vim\""));
        p.on_path.clear();
        let check = get(&p, "Editor (e)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), None);
    }

    #[test]
    fn editor_windows_offers_notepad_in_powershell_form() {
        let mut p = base(Os::Windows);
        p.on_path.clear();
        p.on_path.insert("notepad".to_string());
        assert_eq!(
            copy(&get(&p, "Editor (e)")),
            Some("$env:EDITOR = \"notepad\"")
        );
        let mut linux = base(Os::Linux);
        linux.on_path.clear();
        linux.on_path.insert("notepad".to_string());
        assert_eq!(copy(&get(&linux, "Editor (e)")), None);
    }

    #[test]
    fn diff_tool_found_or_warn_with_gui_line() {
        let mut p = base(Os::Linux);
        assert_eq!(get(&p, "Diff tool (E)").status, CheckStatus::Ok);
        p.on_path.remove("vimdiff");
        let check = get(&p, "Diff tool (E)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), None);
        assert!(hint_of(&check).contains("vim"));
        p.on_path.insert("cursor".to_string());
        assert_eq!(
            copy(&get(&p, "Diff tool (E)")),
            Some("\"diffTool\": \"cursor --diff --wait\"")
        );
        p.diff_tool_config = Some("cursor --diff --wait".to_string());
        assert_eq!(get(&p, "Diff tool (E)").status, CheckStatus::Ok);
    }

    #[test]
    fn browser_opener_per_os_and_fixes() {
        let mut p = base(Os::Linux);
        assert!(get(&p, "Browser opener (gx, Open PR)")
            .why
            .contains("xdg-open"));
        p.on_path.remove("xdg-open");
        let check = get(&p, "Browser opener (gx, Open PR)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), Some("sudo apt install xdg-utils"));
        set(&mut p, "WSL_DISTRO_NAME", "Ubuntu");
        p.on_path.insert("wslview".to_string());
        assert_eq!(
            copy(&get(&p, "Browser opener (gx, Open PR)")),
            Some("export BROWSER=wslview")
        );
        set(&mut p, "BROWSER", "wslview");
        assert_eq!(
            get(&p, "Browser opener (gx, Open PR)").status,
            CheckStatus::Ok
        );
        let mac = base(Os::Macos);
        assert!(get(&mac, "Browser opener (gx, Open PR)")
            .why
            .contains("open"));
        let mut win = base(Os::Windows);
        assert!(get(&win, "Browser opener (gx, Open PR)")
            .why
            .contains("cmd"));
        win.on_path.remove("cmd");
        let check = get(&win, "Browser opener (gx, Open PR)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), None, "no apt on Windows");
    }

    #[test]
    fn forge_clis_by_remote_and_os() {
        let mut p = base(Os::Linux);
        assert!(get(&p, "Forge CLIs (PR badges)").why.contains("No GitHub"));
        p.forges = vec![Forge::GitHub, Forge::GitLab];
        let check = get(&p, "Forge CLIs (PR badges)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.why.contains("PR badges for GitHub remotes need gh"));
        assert!(check.why.contains("PR badges for GitLab remotes need glab"));
        assert!(hint_of(&check).contains("https://cli.github.com"));
        assert_eq!(copy(&check), None);
        p.os = Os::Macos;
        assert_eq!(
            copy(&get(&p, "Forge CLIs (PR badges)")),
            Some("brew install gh glab")
        );
        p.os = Os::Windows;
        p.on_path.insert("gh".to_string());
        assert_eq!(
            copy(&get(&p, "Forge CLIs (PR badges)")),
            Some("winget install glab.glab")
        );
        p.on_path.insert("glab".to_string());
        assert_eq!(get(&p, "Forge CLIs (PR badges)").status, CheckStatus::Ok);
    }

    #[test]
    fn curl_row_release_only() {
        let mut p = base(Os::Linux);
        p.on_path.remove("curl");
        let check = get(&p, "curl (release check)");
        assert_eq!(check.status, CheckStatus::Warn);
        assert_eq!(copy(&check), Some("sudo apt install curl"));
        p.os = Os::Macos;
        assert_eq!(copy(&get(&p, "curl (release check)")), None);
        p.dev_build = true;
        assert!(find(&checks(&p), "curl (release check)").is_none());
    }

    #[test]
    fn rust_toolchain_row_dev_only_never_fails() {
        let mut p = base(Os::Linux);
        p.rustc = Some(Ok("rustc 1.90.0 (abc 2025-09-14)".to_string()));
        assert!(find(&checks(&p), "Rust toolchain (dev build)").is_none());
        p.dev_build = true;
        assert_eq!(
            get(&p, "Rust toolchain (dev build)").status,
            CheckStatus::Ok
        );
        p.rustc = Some(Ok("rustc 1.85.0 (4d91de4e4 2025-02-17)".to_string()));
        assert_eq!(
            get(&p, "Rust toolchain (dev build)").status,
            CheckStatus::Ok
        );
        for rustc in [
            Some(Ok("rustc 1.84.1 (e71f9a9a9 2025-01-27)".to_string())),
            Some(Ok("garbage".to_string())),
            Some(Err("not found".to_string())),
        ] {
            p.rustc = rustc.clone();
            let check = get(&p, "Rust toolchain (dev build)");
            assert_eq!(check.status, CheckStatus::Warn, "{rustc:?}");
            assert_eq!(copy(&check), Some("rustup update stable"));
        }
    }

    #[test]
    fn config_files_ok_names_paths_with_tilde_and_fail_on_error() {
        let mut p = base(Os::Linux);
        let check = get(&p, "Config files");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(
            check
                .why
                .contains("~/.config/my-workspace-status/config.json loaded"),
            "{}",
            check.why
        );
        assert!(check
            .why
            .contains("/w/.workspace-status-config.json not present"));
        p.config.error = Some(
            "/home/u/.config/my-workspace-status/config.json glyphs must be \"nerd\" or \"ascii\""
                .to_string(),
        );
        let check = get(&p, "Config files");
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.why.starts_with("~/.config"), "{}", check.why);
        assert!(check.why.contains("glyphs must be"));
    }

    #[test]
    fn state_files_corrupt_store_fails_with_move_line() {
        let mut p = base(Os::Linux);
        let check = get(&p, "State files");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(check.why.contains("created on first save"));
        p.stores[1].read = StoreRead::Rejected("expected value".to_string());
        let check = get(&p, "State files");
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check
            .why
            .contains("viewed saves will fail, the file is left unchanged"));
        assert_eq!(
            copy(&check),
            Some(
                "mv '/home/u/.local/state/my-workspace-status/viewed-files.json' \
                 '/home/u/.local/state/my-workspace-status/viewed-files.json.bak'"
            )
        );
        p.os = Os::Windows;
        p.stores[1].path = PathBuf::from("C:/Users/o'neil/viewed.json");
        assert_eq!(
            copy(&get(&p, "State files")),
            Some("Move-Item 'C:/Users/o''neil/viewed.json' 'C:/Users/o''neil/viewed.json.bak'")
        );
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn state_files_skip_update_check_store_when_the_check_never_runs() {
        let mut p = base(Os::Linux);
        p.stores[3].read = StoreRead::Rejected("not an update-check store".to_string());
        assert_eq!(get(&p, "State files").status, CheckStatus::Fail);
        p.update_check = false;
        let check = get(&p, "State files");
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(!check.why.contains("update check"));
        p.update_check = true;
        p.dev_build = true;
        assert_eq!(get(&p, "State files").status, CheckStatus::Ok);
    }

    #[test]
    fn windows_fixes_use_powershell_env_form() {
        let mut p = base(Os::Windows);
        unset(&mut p, "COLORTERM");
        p.ascii = false;
        p.git = GitProbe {
            path: "C:/bad/git.exe".to_string(),
            explicit: true,
            version: Err("not found".to_string()),
        };
        for check in checks(&p) {
            if let Some(line) = copy(&check) {
                assert!(!line.starts_with("export "), "{}: {line}", check.name);
                assert!(!line.contains("apt "), "{}: {line}", check.name);
            }
        }
    }

    #[test]
    fn summary_counts_by_status() {
        let mut p = base(Os::Linux);
        p.term_cols = 10;
        p.ascii = false;
        let got = summary(&checks(&p));
        assert_eq!(
            got,
            Summary {
                ok: 13,
                warn: 1,
                fail: 1
            }
        );
    }

    #[test]
    fn rustc_version_parses_major_minor() {
        assert_eq!(
            rustc_version("rustc 1.85.0 (4d91de4e4 2025-02-17)"),
            Some((1, 85))
        );
        assert_eq!(rustc_version("rustc 1.92.0-nightly (x)"), Some((1, 92)));
        assert_eq!(rustc_version("rustc"), None);
    }

    fn make_exec(path: &Path) {
        fs::write(path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// File name of a fake executable `name` that the host's std
    /// `Command::new(name)` would start (`name.exe` on Windows).
    fn host_exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    #[test]
    fn find_on_path_uses_path_entries_and_paths() {
        let os = Os::current();
        let dir = unique_dir("ws-doctor-path");
        let a = dir.join("a");
        let b = dir.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let tool = b.join(host_exe("fake-tool"));
        make_exec(&tool);
        fs::write(a.join("plain-file"), b"").unwrap();
        fs::create_dir_all(a.join(host_exe("a-dir"))).unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(
            find_on_path("fake-tool", Some(&path), os),
            Some(tool.clone())
        );
        assert_eq!(find_on_path("missing", Some(&path), os), None);
        assert_eq!(find_on_path("a-dir", Some(&path), os), None);
        #[cfg(unix)]
        assert_eq!(
            find_on_path("plain-file", Some(&path), os),
            None,
            "no execute bit"
        );
        assert_eq!(find_on_path("fake-tool", None, os), None);
        let direct = tool.display().to_string();
        assert_eq!(find_on_path(&direct, None, os), Some(tool.clone()));
        assert_eq!(find_on_path("", Some(&path), os), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_on_path_mirrors_std_command_on_windows() {
        let dir = unique_dir("ws-doctor-winexe");
        fs::create_dir_all(&dir).unwrap();
        make_exec(&dir.join("tool.exe"));
        make_exec(&dir.join("shim.cmd"));
        make_exec(&dir.join("bare"));
        let path = std::env::join_paths([&dir]).unwrap();
        let win = Os::Windows;
        assert_eq!(
            find_on_path("tool", Some(&path), win),
            Some(dir.join("tool.exe")),
            "a bare name is tried as <name>.exe"
        );
        assert_eq!(
            find_on_path("shim", Some(&path), win),
            None,
            "no PATHEXT: a .cmd shim is not what Command::new starts"
        );
        assert_eq!(
            find_on_path("shim.cmd", Some(&path), win),
            Some(dir.join("shim.cmd")),
            "a name with an extension is used as is"
        );
        assert_eq!(
            find_on_path("bare", Some(&path), win),
            None,
            "a bare name never matches a file without .exe"
        );
        // A path: as is when it is there, else `.exe` when it has no extension.
        let as_path = |name: &str| dir.join(name).display().to_string();
        assert_eq!(
            find_on_path(&as_path("bare"), None, win),
            Some(dir.join("bare"))
        );
        assert_eq!(
            find_on_path(&as_path("tool"), None, win),
            Some(dir.join("tool.exe"))
        );
        assert_eq!(find_on_path(&as_path("shim"), None, win), None);
        assert_eq!(find_on_path(&as_path("tool.dll"), None, win), None);
        // Unix never appends `.exe`.
        assert_eq!(find_on_path("tool", Some(&path), Os::Linux), None);
        assert!(has_extension("C:\\bin\\tool.exe", win));
        assert!(!has_extension("C:\\bin.d\\tool", win));
        assert!(!has_extension(".hidden", win));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tilde_text_replaces_only_whole_home_paths() {
        let home = "/home/user";
        assert_eq!(
            tilde_text("/home/user/.config/x.json glyphs must be", home),
            "~/.config/x.json glyphs must be"
        );
        assert_eq!(tilde_text("/home/user2/x", home), "/home/user2/x");
        assert_eq!(tilde_text("/srv/home/user/x", home), "/srv/home/user/x");
        assert_eq!(tilde_text("/home/user: denied", home), "~: denied");
        assert_eq!(tilde_text("/home/user", "/home/user/"), "~");
        assert_eq!(
            tilde_text("a /home/user/x and /home/user/y", home),
            "a ~/x and ~/y"
        );
        assert_eq!(tilde_text("/x", ""), "/x");
        let mut p = base(Os::Linux);
        set(&mut p, "HOME", "/home/u");
        p.config.error = Some("/home/user2/config.json is not valid".to_string());
        assert!(get(&p, "Config files")
            .why
            .starts_with("/home/user2/config.json"));
    }

    #[test]
    fn store_checks_accept_own_files_and_reject_foreign_ones() {
        let dir = unique_dir("ws-doctor-stores");
        fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.json");
        for kind in [
            StoreKind::Comments,
            StoreKind::Viewed,
            StoreKind::UiState,
            StoreKind::UpdateCheck,
            StoreKind::BackgroundFetch,
        ] {
            assert_eq!(read_store(kind, &missing), StoreRead::Missing);
            let bad = dir.join("bad.json");
            fs::write(&bad, b"{ not json").unwrap();
            assert!(
                matches!(read_store(kind, &bad), StoreRead::Rejected(_)),
                "{kind:?}"
            );
            fs::write(&bad, [0xff, 0xfe]).unwrap();
            assert!(
                matches!(read_store(kind, &bad), StoreRead::Rejected(_)),
                "{kind:?} non-UTF-8"
            );
            assert!(
                matches!(read_store(kind, &dir), StoreRead::Rejected(_)),
                "{kind:?} a directory cannot be read"
            );
        }
        let good = [
            (StoreKind::Comments, r#"{"version":2,"workspaces":{}}"#),
            (StoreKind::Viewed, r#"{"version":2,"workspaces":{}}"#),
            (StoreKind::UiState, r#"{"version":1,"theme":"slate"}"#),
            (StoreKind::UpdateCheck, r#"{"version":1,"lastCheckUnix":0}"#),
            (
                StoreKind::BackgroundFetch,
                r#"{"version":1,"workspaces":{}}"#,
            ),
        ];
        for (kind, text) in good {
            let path = dir.join("good.json");
            fs::write(&path, text).unwrap();
            assert_eq!(read_store(kind, &path), StoreRead::Valid, "{kind:?}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Bytes and mtime of every entry in `dir` (a directory has no bytes;
    /// its mtime moves when an entry is added or removed).
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let meta = fs::metadata(&path).unwrap();
                let bytes = if meta.is_file() {
                    fs::read(&path).unwrap()
                } else {
                    Vec::new()
                };
                (path, (bytes, meta.modified().unwrap()))
            })
            .collect()
    }

    #[test]
    fn collect_is_read_only() {
        let dir = unique_dir("ws-doctor-collect");
        let workspace = dir.join("workspace");
        let state = dir.join("state");
        let bin = dir.join("bin");
        for sub in [&workspace, &state, &bin] {
            fs::create_dir_all(sub).unwrap();
        }
        make_exec(&bin.join(host_exe("gh")));
        let user = dir.join("config.json");
        fs::write(&user, br#"{"glyphs":"ascii"}"#).unwrap();
        fs::write(workspace.join(CONFIG_FILENAME), br#"{"ignoredRepos":[]}"#).unwrap();
        // A corrupt comment store and a valid UI store; viewed and update
        // check stay missing and must not be created.
        fs::write(state.join("comments.json"), b"{ corrupt").unwrap();
        fs::write(state.join("ui-state.json"), br#"{"version":1}"#).unwrap();
        let keys = RuntimeKeys {
            comment_store: Some(state.join("comments.json")),
            viewed_store: Some(state.join("viewed.json")),
            ui_store: Some(state.join("ui-state.json")),
            update_check_store: Some(state.join("update-check.json")),
            glyphs: Some(crate::config::GlyphSet::Ascii),
            ..RuntimeKeys::default()
        };
        let mut settings = Settings::resolve(&keys, |_| None);
        settings.fetch_store = state.join("background-fetch.json");
        let before = [snapshot(&dir), snapshot(&workspace), snapshot(&state)];
        let remotes = vec![
            "git@github.com:octo/demo.git".to_string(),
            "https://github.com/octo/other".to_string(),
            "https://example.com/x/y".to_string(),
        ];
        let inputs = CollectInputs {
            term_cols: 120,
            term_rows: 40,
            settings: &settings,
            mouse_enabled: true,
            editor_config: Some("gh"),
            diff_tool_config: None,
            remote_urls: &remotes,
            workspace_root: &workspace,
            user_config: Some(&user),
        };
        let probe = collect_with(&inputs, |key| match key {
            "PATH" => Some(bin.clone().into_os_string()),
            "HOME" => Some(dir.clone().into_os_string()),
            _ => None,
        });
        let after = [snapshot(&dir), snapshot(&workspace), snapshot(&state)];
        assert_eq!(before, after, "collect changed or created a file");

        assert_eq!(probe.os, Os::current());
        assert!(probe.tmux.is_none(), "no TMUX in the injected env");
        assert!(probe.ascii);
        assert_eq!(probe.forges, vec![Forge::GitHub]);
        assert!(probe.has("gh"));
        assert!(!probe.has("curl"), "PATH is the temp bin dir only");
        assert!(probe.config.error.is_none(), "{:?}", probe.config.error);
        assert!(probe.config.user.as_ref().unwrap().present);
        assert!(probe.config.workspace.present);
        let reads: Vec<(StoreKind, StoreRead)> = probe
            .stores
            .iter()
            .map(|store| (store.kind, store.read.clone()))
            .collect();
        assert!(matches!(
            reads[0],
            (StoreKind::Comments, StoreRead::Rejected(_))
        ));
        assert_eq!(reads[1], (StoreKind::Viewed, StoreRead::Missing));
        assert_eq!(reads[2], (StoreKind::UiState, StoreRead::Valid));
        assert_eq!(reads[3], (StoreKind::UpdateCheck, StoreRead::Missing));
        assert_eq!(reads[4], (StoreKind::BackgroundFetch, StoreRead::Missing));
        let rows = checks(&probe);
        assert_eq!(
            find(&rows, "State files").unwrap().status,
            CheckStatus::Fail
        );
        assert_eq!(find(&rows, "Editor (e)").unwrap().status, CheckStatus::Ok);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collect_reports_a_config_load_error() {
        let dir = unique_dir("ws-doctor-config-err");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(CONFIG_FILENAME), b"[]").unwrap();
        let settings = Settings::resolve(&RuntimeKeys::default(), |key| {
            (key == "XDG_STATE_HOME").then(|| dir.join("state").display().to_string())
        });
        let inputs = CollectInputs {
            term_cols: 120,
            term_rows: 40,
            settings: &settings,
            mouse_enabled: false,
            editor_config: None,
            diff_tool_config: None,
            remote_urls: &[],
            workspace_root: &dir,
            user_config: None,
        };
        let probe = collect_with(&inputs, |_| None);
        assert!(probe.config.error.is_some());
        assert!(probe.config.user.is_none());
        let rows = checks(&probe);
        assert_eq!(
            find(&rows, "Config files").unwrap().status,
            CheckStatus::Fail
        );
        assert!(!dir.join("state").exists(), "collect created the state dir");
        fs::remove_dir_all(&dir).unwrap();
    }
}
