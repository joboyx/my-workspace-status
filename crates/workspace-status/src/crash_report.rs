//! Crash report footer: the facts a bug report needs, printed after a panic.
//!
//! A release build prints only the panic message and the location in the
//! standard library (for example `alloc/src/slice.rs`), which does not name
//! the `ws` code that failed. The footer adds the version, the platform, the
//! terminal size and a backtrace, so one paste of the crash output is enough
//! to start a bug report.

use std::backtrace::Backtrace;
use std::io::Write;

/// Issue tracker that the crash footer points to.
pub(crate) const ISSUES_URL: &str = "https://github.com/joboyx/my-workspace-status/issues";

/// Chain a panic hook that prints [`crash_footer`] after the previous hook.
///
/// Install it first in `cli_main`, before the TUI chains its own hook. The
/// TUI hook restores the terminal and then calls this one, so the footer
/// lands on the normal screen. A worker-job panic that the TUI hook keeps
/// off the screen never reaches this hook.
///
/// With `RUST_BACKTRACE` set, the default hook already prints a backtrace,
/// so the footer leaves it out.
pub(crate) fn install_crash_report_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        let backtrace = std::env::var_os("RUST_BACKTRACE")
            .is_none()
            .then(|| Backtrace::force_capture().to_string());
        let footer = crash_footer(
            &crate::version_label(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            crossterm::terminal::size().ok(),
            backtrace.as_deref(),
        );
        let _ = std::io::stderr().write_all(footer.as_bytes());
    }));
}

/// Text printed after a panic message: version, platform, terminal size,
/// where to report, and the backtrace when one is given.
///
/// `terminal` is `(columns, rows)`; `None` prints `unknown`.
pub(crate) fn crash_footer(
    version: &str,
    os: &str,
    arch: &str,
    terminal: Option<(u16, u16)>,
    backtrace: Option<&str>,
) -> String {
    let size = terminal.map_or_else(|| "unknown".to_string(), |(c, r)| format!("{c}x{r}"));
    let mut out = format!(
        "\nws {version} crashed ({os} {arch}, terminal {size}).\n\
         Please report it at {ISSUES_URL} with this whole output.\n"
    );
    if let Some(trace) = backtrace {
        out.push_str("\nstack backtrace:\n");
        out.push_str(trace.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
