//! Markdown export and clipboard copy.

use std::io::{self, IsTerminal, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::store::{CommentKey, CommentStore};
use crate::tui::pull_request::wait_until;

#[cfg(test)]
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

/// Longest the host clipboard tools of one copy may run together before
/// [`copy_with_host_tool`] kills the one that still runs.
pub const HOST_CLIPBOARD_TIME_LIMIT: Duration = Duration::from_secs(2);

/// Write `text` to the terminal clipboard with OSC 52. True when stdout is a
/// TTY and the write and flush succeed; false (nothing written) otherwise.
///
/// A terminal write only, so the TTY loop calls it between frames.
pub fn write_osc52(text: &str) -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }
    let payload = base64_encode(text.as_bytes());
    let seq = format!("\x1b]52;c;{payload}\x07");
    let mut out = io::stdout().lock();
    out.write_all(seq.as_bytes()).is_ok() && out.flush().is_ok()
}

/// Copy `text` with the first host clipboard tool that succeeds: `wl-copy`,
/// then `xclip -selection clipboard`, then `pbcopy`. False when none starts
/// and succeeds before `deadline`.
///
/// Blocking: it spawns processes and waits for them, so the TTY loop runs
/// it on a worker. A tool that still runs at `deadline` is killed.
pub fn copy_with_host_tool(text: &str, deadline: Instant) -> bool {
    [
        &["wl-copy"][..],
        &["xclip", "-selection", "clipboard"][..],
        &["pbcopy"][..],
    ]
    .into_iter()
    .any(|argv| Instant::now() < deadline && pipe_to(argv, text, deadline))
}

/// Run `argv` with `text` on its stdin. True when it reads all of `text` and
/// exits 0 before `deadline`; past `deadline` it is killed and reaped.
///
/// The stdin write runs on its own thread: a tool that never reads must not
/// stall this thread past `deadline` on a full pipe.
fn pipe_to(argv: &[&str], text: &str, deadline: Instant) -> bool {
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
    let Some(mut stdin) = child.stdin.take() else {
        kill_and_reap(&mut child);
        return false;
    };
    let bytes = text.as_bytes().to_vec();
    let (tx, written) = mpsc::channel();
    std::thread::spawn(move || {
        // The pipe drops at the end of this thread, so the tool sees EOF.
        let _ = tx.send(stdin.write_all(&bytes).is_ok());
    });
    let Some(status) = wait_until(&mut child, deadline, &AtomicBool::new(false)) else {
        // The kill closes the pipe's read end, so a blocked write ends too.
        kill_and_reap(&mut child);
        return false;
    };
    // The tool exited. A child of the tool can still hold the pipe open, so
    // wait for the writer only until the deadline; a blocked writer thread is
    // detached and ends when the last reader closes the pipe.
    let wrote = written
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(false);
    status.success() && wrote
}

fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Standard base64 (RFC 4648, `+/` alphabet, `=` padded) for OSC 52.
///
/// Hand-rolled to keep the dependency list small; the payload is a comment
/// export, so arbitrary UTF-8 bytes have to round-trip.
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4)) as usize] as char);
        match b1 {
            Some(b1) => {
                out.push(TABLE[(((b1 & 0x0f) << 2) | (b2.unwrap_or(0) >> 6)) as usize] as char)
            }
            None => out.push('='),
        }
        match b2 {
            Some(b2) => out.push(TABLE[(b2 & 0x3f) as usize] as char),
            None => out.push('='),
        }
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
        // RFC 4648 section 10 vectors: every padding case.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b"hi!"), "aGkh");
    }

    #[test]
    fn base64_encode_handles_multibyte_and_high_bytes() {
        // Comments carry arbitrary UTF-8; `+` and `/` only appear for
        // high bytes, so an ASCII-only vector set never exercises them.
        assert_eq!(base64_encode("é".as_bytes()), "w6k=");
        assert_eq!(base64_encode("🎉".as_bytes()), "8J+OiQ==");
        assert_eq!(base64_encode(&[0xff, 0xef, 0xbe]), "/+++");
        assert_eq!(base64_encode(&[0x00, 0x00, 0x00]), "AAAA");
    }

    #[cfg(unix)]
    #[test]
    fn a_host_tool_that_reads_stdin_and_exits_zero_copies() {
        let dir = crate::testutil::unique_dir("ws-clipboard-ok");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("clipboard.txt");
        let out_arg = out.to_string_lossy().into_owned();
        let deadline = Instant::now() + Duration::from_secs(10);
        let ok = pipe_to(
            &["sh", "-c", "cat > \"$0\"", &out_arg],
            "copied text",
            deadline,
        );
        assert!(ok, "the tool read stdin and exited 0");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "copied text");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_host_tool_that_hangs_is_killed_at_the_deadline() {
        let dir = crate::testutil::unique_dir("ws-clipboard-hang");
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("tool.pid");
        let pid_arg = pid_file.to_string_lossy().into_owned();
        // More than a pipe buffer, and the tool never reads it: the write
        // must not hold the caller past the deadline.
        let text = "x".repeat(1 << 20);
        let started = Instant::now();
        let ok = pipe_to(
            &["sh", "-c", "echo $$ > \"$0\"; exec sleep 5", &pid_arg],
            &text,
            started + Duration::from_millis(500),
        );
        let took = started.elapsed();
        assert!(!ok, "a tool that does not exit in time is a failed copy");
        assert!(
            took < Duration::from_secs(3),
            "returned at the deadline, took {took:?}"
        );
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: signal 0 only checks that the process exists.
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "the hung tool {pid} was killed and reaped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_host_tool_runs_once_the_deadline_has_passed() {
        assert!(!copy_with_host_tool("text", Instant::now()));
    }

    #[test]
    fn osc52_writes_nothing_when_stdout_is_not_a_terminal() {
        // A run from a terminal would get the escape sequence: check only
        // the headless case (CI, piped output).
        if !io::stdout().is_terminal() {
            assert!(!write_osc52("text"));
        }
    }
}
