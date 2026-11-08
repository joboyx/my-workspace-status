//! Resolve config.editor / `$VISUAL` / `$EDITOR` and build the spawn argv.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use super::pull_request::reap_in_background;

/// Editors that accept `+LINE` before the path.
const PLUS_LINE: &[&str] = &["vim", "nvim", "vi", "nano", "gvim"];
/// Editors that accept `-g path:LINE`.
const GOTO_FLAG: &[&str] = &["code", "code-insiders", "cursor", "codium"];
/// GUI editors that must not steal the TTY.
const DETACHED: &[&str] = &["code", "code-insiders", "cursor", "codium", "gvim"];

/// Split an editor string into argv tokens. Simple quotes only.
pub fn parse_editor_argv(editor: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in editor.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Config `editor`, then `$VISUAL`, then `$EDITOR`, then `vim` (git's order).
pub fn resolve_editor(
    config_editor: Option<&str>,
    env_visual: Option<&str>,
    env_editor: Option<&str>,
) -> String {
    for raw in [config_editor, env_visual, env_editor]
        .into_iter()
        .flatten()
    {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "vim".into()
}

/// True when `e` should spawn without leaving the alternate screen.
pub fn is_detached_editor(editor: &str) -> bool {
    let argv = parse_editor_argv(editor);
    let command = argv.first().map(String::as_str).unwrap_or(editor);
    let name = Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(command);
    DETACHED.contains(&name)
}

/// Start a detached GUI editor `cmd` with `args` in `cwd`, with null stdio,
/// and return its process id.
///
/// A background thread waits for it, so it does not stay a zombie after it
/// exits while the TUI runs on.
pub fn spawn_detached_editor(cmd: &str, args: &[String], cwd: &Path) -> io::Result<u32> {
    let child = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let pid = child.id();
    reap_in_background(child);
    Ok(pid)
}

/// Executable plus args to open `file_path` (optional 1-based `line`).
pub fn editor_command(editor: &str, file_path: &str, line: Option<u32>) -> (String, Vec<String>) {
    let argv = parse_editor_argv(editor);
    let command = argv.first().cloned().unwrap_or_else(|| editor.to_string());
    let mut args: Vec<String> = argv.into_iter().skip(1).collect();
    let name = Path::new(&command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&command)
        .to_string();
    if let Some(line) = line {
        if PLUS_LINE.contains(&name.as_str()) {
            args.push(format!("+{line}"));
            args.push(file_path.into());
            return (command, args);
        }
        if GOTO_FLAG.contains(&name.as_str()) {
            args.push("-g".into());
            args.push(format!("{file_path}:{line}"));
            return (command, args);
        }
    }
    args.push(file_path.into());
    (command, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_config_then_visual_then_editor() {
        assert_eq!(
            resolve_editor(Some("nvim"), Some("code --wait"), Some("vi")),
            "nvim"
        );
        assert_eq!(
            resolve_editor(Some("  "), Some("code --wait"), Some("vi")),
            "code --wait"
        );
        assert_eq!(resolve_editor(None, Some(" "), Some("nano")), "nano");
        assert_eq!(resolve_editor(None, None, None), "vim");
    }

    #[test]
    fn parse_quoted_and_flags() {
        assert_eq!(
            parse_editor_argv(r#"cursor --wait"#),
            vec!["cursor", "--wait"]
        );
        assert_eq!(
            parse_editor_argv(r#""/path with spaces/vim" -p"#),
            vec!["/path with spaces/vim", "-p"]
        );
    }

    #[test]
    fn detached_gui_vs_tty() {
        assert!(is_detached_editor("cursor"));
        assert!(is_detached_editor("code --wait"));
        assert!(!is_detached_editor("vim"));
        assert!(!is_detached_editor("nvim -p"));
    }

    #[test]
    fn plus_line_for_vim() {
        let (cmd, args) = editor_command("vim", "README.md", Some(12));
        assert_eq!(cmd, "vim");
        assert_eq!(args, vec!["+12", "README.md"]);
    }

    /// A detached editor that exits is waited for: `ps` stops listing it
    /// instead of showing a zombie (`Z`) for as long as the TUI runs.
    #[cfg(unix)]
    #[test]
    fn a_detached_editor_that_exits_is_reaped() {
        use std::time::{Duration, Instant};
        let git = crate::git::git_binary().to_string_lossy().into_owned();
        let pid = spawn_detached_editor(&git, &["--version".into()], &std::env::temp_dir())
            .expect("spawn git --version")
            .to_string();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut last = String::new();
        while Instant::now() < deadline {
            let out = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid])
                .output()
                .expect("run ps");
            last = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if last.is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("pid {pid} still listed with state {last:?}");
    }
}
