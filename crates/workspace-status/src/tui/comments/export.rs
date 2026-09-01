//! Markdown export and clipboard copy.

use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};

use super::store::{CommentKey, CommentStore};

/// Markdown marker for a resolved comment. Open comments have no tag.
pub const RESOLVED_MARKDOWN_TAG: &str = "[resolved]";

fn line_span_label(line: u32, end_line: u32) -> String {
    if end_line == line {
        line.to_string()
    } else {
        format!("{line}-{end_line}")
    }
}

fn resolved_suffix(resolved: bool) -> &'static str {
    if resolved {
        " [resolved]"
    } else {
        ""
    }
}

/// Quote every line after the first so a blank line, ATX heading, or list
/// marker cannot leave the current markdown block.
fn quote_continuation_lines(body: &str) -> String {
    let mut lines = body.split('\n');
    let first = lines.next().unwrap_or("");
    let mut out = String::from(first);
    for line in lines {
        out.push_str("\n  >");
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
    }
    out
}

/// Object-comment body under a `##` heading. One line stays a paragraph.
/// Several lines are a blockquote so `#` / `-` in the body cannot escape.
fn object_comment_body(body: &str) -> String {
    if !body.contains('\n') {
        return format!("{body}\n");
    }
    let mut out = String::new();
    for (i, line) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push('>');
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
    }
    out.push('\n');
    out
}

/// Markdown for the comments in `store`. Empty store → a short empty notice.
///
/// Resolved comments stay in the list. The heading or bullet carries
/// [`RESOLVED_MARKDOWN_TAG`] so a copy can tell resolved from open.
/// A line comment with a newline stays one bullet. Continuation lines
/// are quoted.
pub fn export_markdown(store: &CommentStore) -> String {
    if store.is_empty() {
        return "# Comments\n\nNo comments.\n".to_string();
    }
    let mut out = String::from("# Comments\n");
    for (key, entry) in store {
        let tag = resolved_suffix(entry.resolved);
        let body = entry.body.trim_end();
        out.push('\n');
        match key {
            CommentKey::Branch { repo, branch } => {
                out.push_str(&format!("## {repo} — branch `{branch}`{tag}\n\n"));
                out.push_str(&object_comment_body(body));
            }
            CommentKey::Commit { repo, sha } => {
                out.push_str(&format!("## {repo} — commit `{sha}`{tag}\n\n"));
                out.push_str(&object_comment_body(body));
            }
            CommentKey::Worktree { path } => {
                out.push_str(&format!("## {path} — worktree{tag}\n\n"));
                out.push_str(&object_comment_body(body));
            }
            CommentKey::WorktreeLine {
                repo,
                branch,
                path,
                line,
                end_line,
            } => {
                out.push_str(&format!("## {repo} — branch `{branch}`\n\n"));
                out.push_str(&format!(
                    "- `{path}`:{}{tag} — {}\n",
                    line_span_label(*line, *end_line),
                    quote_continuation_lines(body)
                ));
            }
            CommentKey::CommitLine {
                repo,
                sha,
                path,
                line,
                end_line,
            } => {
                out.push_str(&format!("## {repo} — commit `{sha}`\n\n"));
                out.push_str(&format!(
                    "- `{path}`:{}{tag} — {}\n",
                    line_span_label(*line, *end_line),
                    quote_continuation_lines(body)
                ));
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
                end_line: 2,
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
        assert!(
            !md.contains(RESOLVED_MARKDOWN_TAG),
            "open comments must not carry the resolved tag: {md}"
        );
        store = super::super::store::put_comment_entry(
            &store,
            CommentKey::Commit {
                repo: "merger".into(),
                sha: "deadbeef".into(),
            },
            "commit note",
            true,
        );
        let md = export_markdown(&store);
        assert!(md.contains("commit note"));
        assert!(
            md.contains(&format!("commit `deadbeef` {RESOLVED_MARKDOWN_TAG}")),
            "resolved object comments must tag the heading: {md}"
        );
        store = super::super::store::put_comment_entry(
            &store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 2,
                end_line: 2,
            },
            "dirty line",
            true,
        );
        let md = export_markdown(&store);
        assert!(md.contains("dirty line"));
        assert!(
            md.contains(&format!(
                "`README.md`:2 {RESOLVED_MARKDOWN_TAG} — dirty line"
            )),
            "resolved line comments stay in the copy with a tag: {md}"
        );
    }

    #[test]
    fn export_markdown_keeps_multiline_line_comment_in_one_bullet() {
        let store = super::super::store::put_comment_entry(
            &CommentStore::new(),
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 2,
                end_line: 2,
            },
            "one\n# heading\n- list\n\nmore",
            true,
        );
        let md = export_markdown(&store);
        assert!(
            md.contains(&format!(
                "`README.md`:2 {RESOLVED_MARKDOWN_TAG} — one\n  > # heading\n  > - list\n  >\n  > more\n"
            )),
            "continuation lines must stay quoted inside the bullet:\n{md}"
        );
        assert_eq!(
            md.matches("- `").count(),
            1,
            "multiline body must not open extra bullets:\n{md}"
        );
        assert!(
            !md.lines()
                .any(|line| line == "# heading" || line == "- list" || line == "more"),
            "heading / list / blank-line body must not start a new block:\n{md}"
        );
    }

    #[test]
    fn base64_encode_known_vector() {
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b"hi!"), "aGkh");
    }
}
