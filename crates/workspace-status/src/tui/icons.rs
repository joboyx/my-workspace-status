//! Nerd Font glyph registry for the ratatui tree. Port of Ink `src/tui/icons.ts`.
//!
//! A patched Nerd Font is a hard requirement. `WS_STATUS_GLYPHS=ascii` falls
//! back to plain markers. Every glyph occupies one terminal column: Nerd icons
//! live in the private-use area. Do not add emoji or CJK codepoints.

use crate::helpers::visible_width;
use crate::snapshot::{FileChange, SyncStatus};

/// Pick the Nerd Font glyph unless ASCII fallback is active.
pub fn glyph(ascii: bool, nerd: &'static str, fallback: &'static str) -> &'static str {
    if ascii {
        fallback
    } else {
        nerd
    }
}

/* ── Structure ──────────────────────────────────────────────────────────── */

/// Fold chevron — expanded. Width 1 in both modes (Ink does not ASCII-gate this).
pub const FOLD_EXPANDED: &str = "▾";
/// Fold chevron — collapsed.
pub const FOLD_COLLAPSED: &str = "▸";
/// ASCII fold expanded (Rust extra when `WS_STATUS_GLYPHS=ascii`).
pub const FOLD_EXPANDED_ASCII: &str = "v";
/// ASCII fold collapsed (Rust extra when `WS_STATUS_GLYPHS=ascii`).
pub const FOLD_COLLAPSED_ASCII: &str = ">";

/// Cursor accent bar painted in the left-most tree column.
pub const CURSOR_BAR: &str = "▌";

/// Vertical rule between panes and inside the diff gutter.
#[allow(dead_code)]
pub const RULE: &str = "│";

pub fn icon_workspace(ascii: bool) -> &'static str {
    glyph(ascii, "", "#")
}
pub fn icon_repo(ascii: bool) -> &'static str {
    glyph(ascii, "", "@")
}
/// Linked `git worktree` checkout. Nerd: nf-oct-link; ASCII: `L`.
pub fn icon_linked_worktree(ascii: bool) -> &'static str {
    glyph(ascii, "", "L")
}
pub fn icon_branch(ascii: bool) -> &'static str {
    glyph(ascii, "", "&")
}
/// Help MOVE column. Nerd: nf-dev-terminal_badge; ASCII: `+`.
pub fn icon_move(ascii: bool) -> &'static str {
    glyph(ascii, "", "+")
}
/// Help VIEW column. Nerd: nf-oct-diff; ASCII: `%`.
pub fn icon_diff(ascii: bool) -> &'static str {
    glyph(ascii, "", "%")
}
/// Font named in the Ink help footer (`REQUIRED_FONT`).
pub const REQUIRED_FONT: &str = "MesloLGM Nerd Font Mono";
pub fn icon_folder(ascii: bool) -> &'static str {
    glyph(ascii, "", "/")
}
#[allow(dead_code)]
pub fn icon_folder_open(ascii: bool) -> &'static str {
    glyph(ascii, "", "/")
}
pub fn icon_clean(ascii: bool) -> &'static str {
    glyph(ascii, "", ".")
}
pub fn icon_ignored(ascii: bool) -> &'static str {
    glyph(ascii, "", "~")
}
pub fn icon_ahead(ascii: bool) -> &'static str {
    glyph(ascii, "", "^")
}
pub fn icon_behind(ascii: bool) -> &'static str {
    glyph(ascii, "", "v")
}
pub fn icon_diverged(ascii: bool) -> &'static str {
    glyph(ascii, "", "Y")
}
pub fn icon_no_upstream(ascii: bool) -> &'static str {
    glyph(ascii, "", "?")
}
pub fn icon_synced(ascii: bool) -> &'static str {
    glyph(ascii, "", "=")
}
/// HEAD is an ancestor of the default-branch tip. Nerd: nf-fa-check-circle; ASCII: `M`.
pub fn icon_merged_into_default(ascii: bool) -> &'static str {
    glyph(ascii, "", "M")
}
/// HEAD is not merged into default. Nerd: nf-fa-tree; ASCII: `o`.
pub fn icon_open_vs_default(ascii: bool) -> &'static str {
    glyph(ascii, "", "o")
}
/// Ink `ICON_VIEWED` nerd glyph: nf-fa-eye (`U+F06E`).
///
/// Same pairing as `src/tui/icons.ts`. Do not substitute `◉` or another PUA eye.
pub const ICON_VIEWED_NERD: &str = "\u{f06e}";
/// Ink `ICON_VIEWED` ASCII fallback.
pub const ICON_VIEWED_ASCII: &str = "*";

/// Reviewed mark on a dirty file row. Nerd: nf-fa-eye (`U+F06E`); ASCII: `*`.
/// Distinct from `ICON_CLEAN` / `ICON_SYNCED`.
pub fn icon_viewed(ascii: bool) -> &'static str {
    glyph(ascii, ICON_VIEWED_NERD, ICON_VIEWED_ASCII)
}

const DEFAULT_FILE_GLYPH: &str = "";
const ASCII_FILE_GLYPH: &str = "·";

/// Devicon for a repo-relative file path. Exact filename wins, then extension.
pub fn file_icon(ascii: bool, file_path: &str) -> FileIcon {
    if ascii {
        return FileIcon {
            glyph: ASCII_FILE_GLYPH,
            color: None,
        };
    }
    let name = file_path
        .rsplit('/')
        .next()
        .unwrap_or(file_path)
        .to_ascii_lowercase();
    if let Some(icon) = filename_icon(&name) {
        return icon;
    }
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if let Some(icon) = extension_icon(ext) {
        return icon;
    }
    FileIcon {
        glyph: DEFAULT_FILE_GLYPH,
        color: None,
    }
}

/// Nerd file glyph plus optional hex colour (theme `file` when `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIcon {
    pub glyph: &'static str,
    pub color: Option<&'static str>,
}

fn filename_icon(name: &str) -> Option<FileIcon> {
    Some(match name {
        ".gitignore" | ".gitattributes" | ".gitmodules" => FileIcon {
            glyph: "",
            color: Some("#e24329"),
        },
        "package.json" => FileIcon {
            glyph: "",
            color: Some("#e8274b"),
        },
        "package-lock.json" => FileIcon {
            glyph: "",
            color: Some("#7a0d21"),
        },
        "dockerfile" => FileIcon {
            glyph: "",
            color: Some("#458ee6"),
        },
        "makefile" => FileIcon {
            glyph: "",
            color: Some("#6d8086"),
        },
        "readme.md" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        ".envrc" | ".env" => FileIcon {
            glyph: "",
            color: Some("#faf743"),
        },
        _ => return None,
    })
}

fn extension_icon(ext: &str) -> Option<FileIcon> {
    Some(match ext {
        "ts" | "mts" | "cts" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "tsx" | "jsx" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "js" | "mjs" | "cjs" => FileIcon {
            glyph: "",
            color: Some("#cbcb41"),
        },
        "json" => FileIcon {
            glyph: "",
            color: Some("#cbcb41"),
        },
        "md" | "mdx" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "py" => FileIcon {
            glyph: "",
            color: Some("#ffbc03"),
        },
        "sh" | "bash" | "zsh" => FileIcon {
            glyph: "",
            color: Some("#89e051"),
        },
        "html" => FileIcon {
            glyph: "",
            color: Some("#e34c26"),
        },
        "css" => FileIcon {
            glyph: "",
            color: Some("#563d7c"),
        },
        "scss" => FileIcon {
            glyph: "",
            color: Some("#f55385"),
        },
        "yml" | "yaml" | "toml" => FileIcon {
            glyph: "",
            color: Some("#6d8086"),
        },
        "java" => FileIcon {
            glyph: "",
            color: Some("#cc3e44"),
        },
        "cs" => FileIcon {
            glyph: "",
            color: Some("#596706"),
        },
        "go" => FileIcon {
            glyph: "",
            color: Some("#519aba"),
        },
        "rs" => FileIcon {
            glyph: "",
            color: Some("#dea584"),
        },
        "sql" => FileIcon {
            glyph: "",
            color: Some("#dad8d8"),
        },
        "png" | "jpg" | "jpeg" | "gif" => FileIcon {
            glyph: "",
            color: Some("#a074c4"),
        },
        "svg" => FileIcon {
            glyph: "",
            color: Some("#ffb13b"),
        },
        "lock" => FileIcon {
            glyph: "",
            color: Some("#bbbbbb"),
        },
        "txt" => FileIcon {
            glyph: "",
            color: None,
        },
        _ => return None,
    })
}

/* ── File status ────────────────────────────────────────────────────────── */

/// Letter code aligned with Ink `FileStatusLetter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatusLetter {
    A,
    M,
    S,
    Ms,
    D,
    R,
    U,
    C,
}

impl FileStatusLetter {
    /// Ink letter token (`A` / `S` / `MS` / `M` / `D` / `R` / `U` / `C`).
    ///
    /// Watch signatures and tests use this, not the 2-column badge.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::M => "M",
            Self::S => "S",
            Self::Ms => "MS",
            Self::D => "D",
            Self::R => "R",
            Self::U => "U",
            Self::C => "C",
        }
    }

    /// Exactly 2 display columns — right-aligned like the VS Code SCM gutter.
    pub fn badge(self) -> &'static str {
        match self {
            Self::A => "A ",
            Self::M => "M ",
            Self::S => "S ",
            Self::Ms => "MS",
            Self::D => "D ",
            Self::R => "R ",
            Self::U => "U ",
            Self::C => "C ",
        }
    }

    /// Ink `statusColor` token name.
    pub fn color_role(self) -> StatusColorRole {
        match self {
            Self::A | Self::S => StatusColorRole::Added,
            Self::M | Self::Ms => StatusColorRole::Modified,
            Self::D | Self::U => StatusColorRole::Deleted,
            Self::R | Self::C => StatusColorRole::Renamed,
        }
    }
}

/// Semantic colour for a status letter / sync mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusColorRole {
    Added,
    Modified,
    Deleted,
    Renamed,
    Muted,
    #[allow(dead_code)]
    File,
}

/// Map a FileChange to the same letter vocabulary as Ink `statusLetterFromChange`.
pub fn status_letter_from_change(change: &FileChange) -> FileStatusLetter {
    // Conflict before MS — staged+unstaged both set must not swallow U.
    let unstaged = change.unstaged_status.as_deref();
    let staged = change.staged_status.as_deref();
    if unstaged == Some("U") || staged == Some("U") {
        return FileStatusLetter::U;
    }
    if staged.is_some() && unstaged.is_some() {
        return FileStatusLetter::Ms;
    }
    let status = unstaged.or(staged);
    if status == Some("R") {
        return FileStatusLetter::R;
    }
    if status == Some("D") {
        return FileStatusLetter::D;
    }
    if change.untracked || status == Some("A") {
        return FileStatusLetter::A;
    }
    if staged.is_some() && unstaged.is_none() {
        return FileStatusLetter::S;
    }
    if status == Some("C") {
        return FileStatusLetter::C;
    }
    FileStatusLetter::M
}

/// Exactly 2 display columns for a file-change badge.
pub fn tui_file_badge(change: &FileChange) -> &'static str {
    status_letter_from_change(change).badge()
}

/* ── Branch / sync ──────────────────────────────────────────────────────── */

/// Merge-into-default mark for TUI branch chrome. Never emoji.
pub fn tui_merge_mark(ascii: bool, merged: Option<bool>) -> &'static str {
    match merged {
        Some(true) => icon_merged_into_default(ascii),
        Some(false) => icon_open_vs_default(ascii),
        None => "",
    }
}

/// Sync mark: glyph plus commit count, e.g. ahead-by-2 or behind-by-3.
pub fn tui_sync_mark(ascii: bool, status: SyncStatus, note: &str) -> String {
    match status {
        SyncStatus::NoUpstream => icon_no_upstream(ascii).to_string(),
        SyncStatus::Behind => {
            let count = capture_count(note, "behind by ");
            format!("{}{count}", icon_behind(ascii))
        }
        SyncStatus::Ahead => {
            let count = capture_count(note, "ahead by ");
            format!("{}{count}", icon_ahead(ascii))
        }
        SyncStatus::Diverged => icon_diverged(ascii).to_string(),
        SyncStatus::UpToDate => icon_synced(ascii).to_string(),
    }
}

fn capture_count<'a>(note: &'a str, prefix: &str) -> &'a str {
    let Some(idx) = note.find(prefix) else {
        return "";
    };
    let start = idx + prefix.len();
    let end = note[start..]
        .find(|c: char| !c.is_ascii_digit())
        .map(|n| start + n)
        .unwrap_or(note.len());
    &note[start..end]
}

/// Colour matching `tui_sync_mark` semantics.
pub fn sync_color_role(status: SyncStatus) -> StatusColorRole {
    match status {
        SyncStatus::Behind => StatusColorRole::Deleted,
        SyncStatus::Ahead => StatusColorRole::Added,
        SyncStatus::Diverged => StatusColorRole::Modified,
        SyncStatus::UpToDate | SyncStatus::NoUpstream => StatusColorRole::Muted,
    }
}

/// Truncate a string to at most `width` terminal columns.
pub fn truncate_visible(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if visible_width(value) <= width {
        return value.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in value.chars() {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let cw = visible_width(s);
        if w + cw > width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// True if any codepoint is in the emoji / pictograph range (≥ U+1F300).
#[allow(dead_code)]
pub fn has_wide_emoji(value: &str) -> bool {
    value.chars().any(|ch| (ch as u32) >= 0x1f300)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(untracked: bool, staged: Option<&str>, unstaged: Option<&str>) -> FileChange {
        FileChange {
            path: "a".into(),
            staged_status: staged.map(str::to_string),
            unstaged_status: unstaged.map(str::to_string),
            untracked,
            old_path: None,
        }
    }

    #[test]
    fn badges_are_two_columns_and_match_ink() {
        let letters = [
            FileStatusLetter::A,
            FileStatusLetter::M,
            FileStatusLetter::S,
            FileStatusLetter::Ms,
            FileStatusLetter::D,
            FileStatusLetter::R,
            FileStatusLetter::U,
            FileStatusLetter::C,
        ];
        let expected = ["A ", "M ", "S ", "MS", "D ", "R ", "U ", "C "];
        for (letter, want) in letters.into_iter().zip(expected) {
            assert_eq!(letter.badge(), want);
            assert_eq!(visible_width(letter.badge()), 2);
        }
        assert_eq!(
            status_letter_from_change(&change(true, None, None)),
            FileStatusLetter::A
        );
        assert_eq!(
            status_letter_from_change(&change(false, Some("M"), Some("M"))),
            FileStatusLetter::Ms
        );
        assert_eq!(
            status_letter_from_change(&change(false, Some("M"), None)),
            FileStatusLetter::S
        );
        assert_eq!(
            status_letter_from_change(&change(false, None, Some("M"))),
            FileStatusLetter::M
        );
        assert_eq!(
            status_letter_from_change(&change(false, Some("U"), Some("M"))),
            FileStatusLetter::U
        );
        assert_eq!(tui_file_badge(&change(true, None, None)), "A ");
    }

    #[test]
    fn structure_glyphs_are_width_one() {
        let ascii = [
            icon_workspace(true),
            icon_repo(true),
            icon_linked_worktree(true),
            icon_branch(true),
            icon_folder(true),
            icon_clean(true),
            icon_ignored(true),
            icon_ahead(true),
            icon_behind(true),
            icon_diverged(true),
            icon_no_upstream(true),
            icon_synced(true),
            icon_merged_into_default(true),
            icon_open_vs_default(true),
            icon_viewed(true),
            ASCII_FILE_GLYPH,
        ];
        let nerd = [
            icon_workspace(false),
            icon_repo(false),
            icon_linked_worktree(false),
            icon_branch(false),
            icon_folder(false),
            icon_clean(false),
            icon_ignored(false),
            icon_ahead(false),
            icon_behind(false),
            icon_diverged(false),
            icon_no_upstream(false),
            icon_synced(false),
            icon_merged_into_default(false),
            icon_open_vs_default(false),
            icon_viewed(false),
            CURSOR_BAR,
            FOLD_EXPANDED,
            FOLD_COLLAPSED,
            RULE,
        ];
        for g in ascii.into_iter().chain(nerd) {
            assert_eq!(visible_width(g), 1, "{g:?}");
            assert!(!has_wide_emoji(g), "{g:?}");
        }
        assert_eq!(icon_workspace(true), "#");
        assert_eq!(icon_viewed(true), "*");
        assert_ne!(icon_viewed(false), icon_clean(false));
        assert_ne!(icon_viewed(false), icon_synced(false));
    }

    #[test]
    fn viewed_is_ink_nf_fa_eye_not_a_substitute() {
        assert_eq!(icon_viewed(false), "\u{f06e}");
        assert_eq!(icon_viewed(true), "*");
        assert_eq!(icon_viewed(false), ICON_VIEWED_NERD);
        assert_eq!(icon_viewed(true), ICON_VIEWED_ASCII);
        let nerd = icon_viewed(false).chars().next().expect("glyph");
        assert_eq!(u32::from(nerd), 0xf06e);
        assert_ne!(nerd, '\u{25c9}'); // ◉
        assert_ne!(nerd, '\u{f07a}'); // other PUA eye/search lookalike
        assert_eq!(visible_width(icon_viewed(false)), 1);
    }

    #[test]
    fn file_icons_and_ascii_fallback() {
        assert_eq!(file_icon(true, "a.ts").glyph, "·");
        assert_eq!(visible_width(file_icon(false, "a.ts").glyph), 1);
        assert_eq!(visible_width(file_icon(false, "a.wat").glyph), 1);
        assert_ne!(
            file_icon(false, "package.json").glyph,
            file_icon(false, "tsconfig.json").glyph
        );
        assert_eq!(
            file_icon(false, "README.md").glyph,
            file_icon(false, "readme.md").glyph
        );
    }

    #[test]
    fn sync_and_merge_marks() {
        assert_eq!(
            tui_sync_mark(false, SyncStatus::Behind, "behind by 3"),
            format!("{}3", icon_behind(false))
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::Ahead, "ahead by 2"),
            "^2".to_string()
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::NoUpstream, ""),
            "?".to_string()
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::Diverged, ""),
            "Y".to_string()
        );
        assert_eq!(
            tui_sync_mark(true, SyncStatus::UpToDate, ""),
            "=".to_string()
        );
        assert_eq!(tui_merge_mark(true, Some(true)), "M");
        assert_eq!(tui_merge_mark(true, Some(false)), "o");
        assert_eq!(tui_merge_mark(true, None), "");
    }
}
