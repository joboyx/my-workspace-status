//! Markdown export and clipboard copy.

use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};

use super::store::{CommentKey, CommentStore};

/// Markdown for the comments in `store`. Empty store → a short empty notice.
pub fn export_markdown(store: &CommentStore) -> String {
    if store.is_empty() {
        return "# Comments\n\nNo comments.\n".to_string();
    }
    let mut out = String::from("# Comments\n");
    for (key, body) in store {
        out.push('\n');
        match key {
            CommentKey::Branch { repo, branch } => {
                out.push_str(&format!("## {repo} — branch `{branch}`\n\n"));
                out.push_str(&format!("{}\n", body.trim_end()));
            }
            CommentKey::Commit { repo, sha } => {
                out.push_str(&format!("## {repo} — commit `{sha}`\n\n"));
                out.push_str(&format!("{}\n", body.trim_end()));
            }
            CommentKey::Worktree { path } => {
                out.push_str(&format!("## {path} — worktree\n\n"));
                out.push_str(&format!("{}\n", body.trim_end()));
            }
            CommentKey::WorktreeLine {
                repo,
                branch,
                path,
                line,
            } => {
                out.push_str(&format!("## {repo} — branch `{branch}`\n\n"));
                out.push_str(&format!("- `{path}`:{line} — {}\n", body.trim_end()));
            }
            CommentKey::CommitLine {
                repo,
                sha,
                path,
                line,
            } => {
                out.push_str(&format!("## {repo} — commit `{sha}`\n\n"));
                out.push_str(&format!("- `{path}`:{line} — {}\n", body.trim_end()));
            }
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Copy `text` to the clipboard. OSC 52 when stdout is a TTY, then a host tool.
pub fn copy_to_clipboard(text: &str) -> bool {
    let mut ok = false;
    if io::stdout().is_terminal() {
        let payload = base64_encode(text.as_bytes());
        let seq = format!("\x1b]52;c;{payload}\x07");
        let mut out = io::stdout().lock();
        if out.write_all(seq.as_bytes()).is_ok() && out.flush().is_ok() {
            ok = true;
        }
    }
    for argv in [
        &["wl-copy"][..],
        &["xclip", "-selection", "clipboard"][..],
        &["pbcopy"][..],
    ] {
        if pipe_to(argv, text) {
            ok = true;
            break;
        }
    }
    ok
}

fn pipe_to(argv: &[&str], text: &str) -> bool {
    let Some((bin, args)) = argv.split_first() else {
        return false;
    };
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let ok = child
        .stdin
        .as_mut()
        .map(|stdin| stdin.write_all(text.as_bytes()).is_ok())
        .unwrap_or(false);
    child.wait().map(|s| s.success()).unwrap_or(false) && ok
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied();
        let b2 = data.get(i + 2).copied();
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4)) as usize] as char);
        if b1.is_some() {
            out.push(
                TABLE[(((b1.unwrap_or(0) & 0x0f) << 2) | (b2.unwrap_or(0) >> 6)) as usize] as char,
            );
        } else {
            out.push('=');
        }
        if b2.is_some() {
            out.push(TABLE[(b2.unwrap_or(0) & 0x3f) as usize] as char);
        } else if b1.is_some() {
            out.push('=');
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::store::put_comment;
    use super::*;

    #[test]
    fn export_markdown_lists_live_omits_chrome() {
        let mut store = CommentStore::new();
        store = put_comment(
            &store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 2,
            },
            "dirty line",
        );
        store = put_comment(
            &store,
            CommentKey::Commit {
                repo: "merger".into(),
                sha: "deadbeef".into(),
            },
            "commit note",
        );
        let md = export_markdown(&store);
        assert!(md.contains("# Comments"));
        assert!(md.contains("app"));
        assert!(md.contains("branch `main`"));
        assert!(md.contains("`README.md`:2"));
        assert!(md.contains("dirty line"));
        assert!(md.contains("merger"));
        assert!(md.contains("commit `deadbeef`"));
        assert!(md.contains("commit note"));
        assert!(!md.contains("tokyo-night"));
        assert!(!md.contains("\"kind\""));
    }

    #[test]
    fn base64_encode_known_vector() {
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b"hi!"), "aGkh");
    }
}
