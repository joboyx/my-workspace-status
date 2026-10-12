//! Terminal hyperlinks (OSC 8) painted over the finished frame.
//!
//! Ratatui has no hyperlink API, and an escape sequence inside a cell symbol
//! breaks its buffer diff. So the event loop draws the frame as usual, then
//! prints the cells of each [`LinkHit`] again from the finished buffer,
//! wrapped in OSC 8. The terminal stores the link per cell: the same glyphs
//! come back with the link attached. A later diff that writes those cells
//! without OSC 8 clears the link, and the next frame adds it again.
//!
//! [`link_cells`] reads the cells to print; [`write_links`] prints them. A
//! target URL can come from a forge API (PR badge), a git remote (commit
//! hash), or a file name (path rows, [`file_url`]), so a URL with a byte
//! outside printable ASCII (`0x21..=0x7E`) is dropped and can never inject
//! an escape. [`file_url`] percent-encodes so a path link always passes.

use std::io::{self, Write};

use crossterm::cursor::{RestorePosition, SavePosition};
use crossterm::queue;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::{Buffer, Cell};
use unicode_width::UnicodeWidthStr;

/// A screen span that links to a URL.
///
/// The span covers the painted glyphs only. The URL is the full target, even
/// when the visible text is shortened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkHit {
    /// 0-based screen row of the span.
    pub y: u16,
    /// 0-based first screen column of the span.
    pub x: u16,
    /// Span width in display columns.
    pub width: u16,
    /// Link target.
    pub url: String,
}

/// The cells of one [`LinkHit`], read from a finished frame buffer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinkCells {
    /// Link target, already checked by [`is_safe_url`].
    url: String,
    /// `(x, y, cell)` to print again, wide-glyph continuations left out.
    cells: Vec<(u16, u16, Cell)>,
}

/// True when `url` can go inside an OSC 8 sequence: not empty, and every
/// byte printable ASCII with no space.
fn is_safe_url(url: &str) -> bool {
    !url.is_empty() && url.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

/// Append `bytes` to `out`, each byte outside RFC 3986 unreserved
/// (`A-Z a-z 0-9 - . _ ~`) as `%XX`. `/` stays as is when `keep_slash`.
fn push_percent_encoded(out: &mut String, bytes: &[u8], keep_slash: bool) {
    for &byte in bytes {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && byte == b'/')
        {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
}

/// `file://` URL of the local file at absolute path `abs` on `host`.
///
/// `file://{host}{path}`. Every path byte outside RFC 3986 unreserved
/// characters and `/` is percent-encoded (spaces, `#`, `?`, `%`, non-ASCII
/// bytes), so the URL always passes the OSC 8 safety check. An empty
/// `host` gives `file:///…`. On Windows, `\` becomes `/` and the drive
/// gets a leading `/`.
pub fn file_url(host: &str, abs: &std::path::Path) -> String {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        abs.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let path = {
        let text = abs.to_string_lossy().replace('\\', "/");
        if text.starts_with('/') {
            text.into_bytes()
        } else {
            format!("/{text}").into_bytes()
        }
    };
    let mut url = String::from("file://");
    push_percent_encoded(&mut url, host.as_bytes(), false);
    push_percent_encoded(&mut url, &path, true);
    url
}

/// Host name for [`file_url`], read once per process.
///
/// `gethostname` on unix; `""` when it fails and on other systems, which
/// gives a `file:///…` URL.
pub(crate) fn local_host() -> &'static str {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(read_host_name)
}

#[cfg(unix)]
fn read_host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for `buf.len()` bytes; the call writes at most
    // that many.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[cfg(not(unix))]
fn read_host_name() -> String {
    String::new()
}

/// Read the cells of each link in `links` from `buffer`.
///
/// A link with an unsafe URL, or whose span is off the buffer, is left out.
/// The span is clipped to the buffer. A wide glyph is read once: the cells it
/// covers after the first are skipped, as is a cell with `skip` set, the
/// same way ratatui's diff skips them.
pub(crate) fn link_cells(buffer: &Buffer, links: &[LinkHit]) -> Vec<LinkCells> {
    let area = buffer.area;
    links
        .iter()
        .filter(|link| is_safe_url(&link.url))
        .filter_map(|link| {
            if link.y < area.top() || link.y >= area.bottom() {
                return None;
            }
            let start = link.x.max(area.left());
            let end = link.x.saturating_add(link.width).min(area.right());
            let mut cells = Vec::new();
            let mut to_skip = 0usize;
            for x in start..end {
                let cell = &buffer[(x, link.y)];
                if to_skip == 0 && !cell.skip {
                    cells.push((x, link.y, cell.clone()));
                }
                to_skip = if to_skip > 0 {
                    to_skip - 1
                } else {
                    cell.symbol().width().saturating_sub(1)
                };
            }
            (!cells.is_empty()).then(|| LinkCells {
                url: link.url.clone(),
                cells,
            })
        })
        .collect()
}

/// Print each link's cells again inside an OSC 8 hyperlink, then flush.
///
/// The cursor goes back to where it was, so the cursor ratatui placed does
/// not move. Writes nothing when `links` is empty.
pub(crate) fn write_links<W: Write>(
    backend: &mut CrosstermBackend<W>,
    links: &[LinkCells],
) -> io::Result<()> {
    if links.is_empty() {
        return Ok(());
    }
    queue!(backend, SavePosition)?;
    for link in links {
        write!(backend, "\x1b]8;;{}\x1b\\", link.url)?;
        Backend::draw(
            backend,
            link.cells.iter().map(|(x, y, cell)| (*x, *y, cell)),
        )?;
        write!(backend, "\x1b]8;;\x1b\\")?;
    }
    queue!(backend, RestorePosition)?;
    Backend::flush(backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    const FULL: &str = "https://github.com/acme/widgets/pull/12345";

    fn open(url: &str) -> String {
        format!("\x1b]8;;{url}\x1b\\")
    }

    const CLOSE: &str = "\x1b]8;;\x1b\\";

    fn link(y: u16, x: u16, width: u16, url: &str) -> LinkHit {
        LinkHit {
            y,
            x,
            width,
            url: url.into(),
        }
    }

    /// Bytes [`write_links`] prints for `links` over `buffer`.
    fn painted(buffer: &Buffer, links: &[LinkHit]) -> String {
        let mut out = Vec::new();
        write_links(
            &mut CrosstermBackend::new(&mut out),
            &link_cells(buffer, links),
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn link_targets_the_full_url_over_the_truncated_text() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 3));
        let shown = "https://github.com/acme/widgets/pu…";
        buffer.set_string(2, 1, shown, Style::default());
        let width = u16::try_from(shown.width()).unwrap();

        let out = painted(&buffer, &[link(1, 2, width, FULL)]);

        let open_at = out.find(&open(FULL)).unwrap_or_else(|| panic!("{out:?}"));
        let text_at = out[open_at..].find(shown).unwrap() + open_at;
        let close_at = out[text_at..].find(CLOSE).unwrap() + text_at;
        assert!(open_at < text_at && text_at < close_at, "{out:?}");
        assert!(!out[open_at + open(FULL).len()..].contains("/pull/12345"));
        assert!(out.starts_with("\x1b7"), "save cursor first: {out:?}");
        assert!(out.ends_with("\x1b8"), "restore cursor last: {out:?}");
    }

    #[test]
    fn unsafe_url_emits_nothing() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 1));
        buffer.set_string(0, 0, "P", Style::default());
        for url in [
            "",
            "https://x/\x1b]8;;evil",
            "https://x/a b",
            "https://x/\u{7f}",
            "https://x/é",
        ] {
            assert_eq!(painted(&buffer, &[link(0, 0, 1, url)]), "", "{url:?}");
        }
        // A safe link next to an unsafe one still paints alone.
        let out = painted(
            &buffer,
            &[link(0, 0, 1, "https://x/\n"), link(0, 0, 1, FULL)],
        );
        assert_eq!(out.matches("\x1b]8;;h").count(), 1, "{out:?}");
    }

    #[test]
    fn wide_glyph_prints_once() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        buffer.set_string(0, 0, "a界b", Style::default());
        let cells = link_cells(&buffer, &[link(0, 0, 4, FULL)]);
        let xs: Vec<u16> = cells[0].cells.iter().map(|(x, _, _)| *x).collect();
        assert_eq!(xs, vec![0, 1, 3]);

        let out = painted(&buffer, &[link(0, 0, 4, FULL)]);
        assert_eq!(out.matches('界').count(), 1, "{out:?}");
        assert!(out.contains('a') && out.contains('b'), "{out:?}");
    }

    #[test]
    fn span_off_the_buffer_is_clipped_or_dropped() {
        let buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        assert!(link_cells(&buffer, &[link(5, 0, 2, FULL)]).is_empty());
        assert!(link_cells(&buffer, &[link(0, 9, 2, FULL)]).is_empty());
        let clipped = link_cells(&buffer, &[link(1, 2, 9, FULL)]);
        assert_eq!(clipped[0].cells.len(), 2);
    }

    /// Decode `%XX` escapes back to bytes.
    fn percent_decode(text: &str) -> Vec<u8> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        out
    }

    #[test]
    fn file_url_percent_encodes_spaces_hash_and_non_ascii() {
        let path = std::path::Path::new("/work/a b#c/é.rs");
        let url = file_url("box-1", path);
        assert_eq!(url, "file://box-1/work/a%20b%23c/%C3%A9.rs");
        let rest = url.strip_prefix("file://box-1").unwrap();
        assert!(
            rest.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~/%".contains(&b)),
            "{url}"
        );
        assert_eq!(percent_decode(rest), "/work/a b#c/é.rs".as_bytes());
        // No host: `file:///…`.
        assert_eq!(
            file_url("", std::path::Path::new("/x/?%.rs")),
            "file:///x/%3F%25.rs"
        );
    }

    #[test]
    fn file_url_passes_is_safe_url() {
        for (host, path) in [
            ("box-1", "/work/a b#c/é.rs"),
            ("", "/tmp/tab\there/new\nline"),
            ("host with space", "/界/\u{7f}/x"),
        ] {
            let url = file_url(host, std::path::Path::new(path));
            assert!(is_safe_url(&url), "{url:?}");
        }
        assert!(is_safe_url(&file_url(
            local_host(),
            std::path::Path::new("/a b")
        )));
    }

    #[test]
    fn no_links_writes_nothing() {
        let buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        assert_eq!(painted(&buffer, &[]), "");
    }
}
