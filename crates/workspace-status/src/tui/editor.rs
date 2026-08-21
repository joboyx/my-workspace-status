//! Resolve `$EDITOR` / config.editor and build the spawn argv.

use std::path::Path;

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

/// Config `editor`, then `$EDITOR`, then `$VISUAL`, then `vim`.
pub fn resolve_editor(config_editor: Option<&str>, env_editor: Option<&str>, env_visual: Option<&str>) -> String {
    for candidate in [config_editor, env_editor, env_visual] {
        if let Some(raw) = candidate {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
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
    fn resolve_prefers_config_then_editor() {
        assert_eq!(resolve_editor(Some("nvim"), Some("vim"), Some("vi")), "nvim");
        assert_eq!(resolve_editor(Some("  "), Some("vim"), None), "vim");
        assert_eq!(resolve_editor(None, None, None), "vim");
        assert_eq!(resolve_editor(None, None, Some("nano")), "nano");
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
}
