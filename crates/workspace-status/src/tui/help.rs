//! Help overlay entries and `/` search (highlight only).
//!
//! Three columns match `HELP_GROUPS` (MOVE / GIT / VIEW). Extra keys
//! (`q`, Tab, graph `c`, stash `a p D`, Home/End) stay in those groups.
//! Each column stacks its own entries (no row alignment across columns);
//! [`help_column_widths`] splits the width so the tallest column is as
//! short as it can be. The footer shows [`crate::APP_VERSION`] in the
//! lower-right. On a compare tab [`help_groups`] swaps GIT for
//! [`HELP_COMPARE_GROUP`] (what acts on the compare diff and what needs the
//! Workspace tab), on a file tab for [`HELP_FILE_GROUP`];
//! [`help_status_lines`] sizes the help dialog for the columns that paint
//! plus the icon legend under them. A dialog shorter than that scrolls its
//! body.
//!
//! The legend ([`help_legend_layout`]) lists the icon catalog rows that
//! have a legend group (Tree / Graph / Chrome): glyph, muted name, meaning.
//! It flows into up to [`HELP_LEGEND_MAX_COLUMNS`] even columns inside the
//! same scroll body, so the three key columns stay three.

use std::sync::LazyLock;

use super::icons::{IconGroup, IconSpec, ICON_CATALOG};

/// One help row: key chips plus a short description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    pub keys: &'static str,
    pub desc: &'static str,
}

/// One help column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpGroup {
    pub title: &'static str,
    pub entries: &'static [HelpEntry],
}

/// Help overlay stays on three groups.
#[cfg(test)]
pub const HELP_COLUMN_COUNT: usize = 3;

/// Short key list shown in the `?` overlay, grouped the same.
pub const HELP_GROUPS: &[HelpGroup] = &[
    HelpGroup {
        title: "MOVE",
        entries: &[
            HelpEntry {
                keys: "j k",
                desc: "down / up",
            },
            HelpEntry {
                keys: "h l",
                desc: "fold · pan lists/diff · Shift-←→ tree",
            },
            HelpEntry {
                keys: "z",
                desc: "toggle fold (instant; no-op on graph/diff)",
            },
            HelpEntry {
                keys: "zz",
                desc: "toggle subtree (no-op on graph/diff)",
            },
            HelpEntry {
                keys: "gg G",
                desc: "top / bottom of focused pane",
            },
            HelpEntry {
                keys: "Home End",
                desc: "top / bottom",
            },
            HelpEntry {
                keys: "/",
                desc: "search focused pane (Enter arms)",
            },
            HelpEntry {
                keys: "n N",
                desc: "next / prev match (after Enter)",
            },
            HelpEntry {
                keys: "gt gT",
                desc: "next / prev tab",
            },
            HelpEntry {
                keys: "g1-g9",
                desc: "jump to tab (1=Workspace)",
            },
        ],
    },
    HelpGroup {
        title: "GIT",
        entries: &[
            HelpEntry {
                keys: "s",
                desc: "stage scope",
            },
            HelpEntry {
                keys: "S",
                desc: "stash menu",
            },
            HelpEntry {
                keys: "u",
                desc: "unstage scope",
            },
            HelpEntry {
                keys: "x",
                desc: "revert (y/Y)",
            },
            HelpEntry {
                keys: "e E",
                desc: "open in editor · open in diff tool",
            },
            HelpEntry {
                keys: "space",
                desc: "mark file reviewed (eye)",
            },
            HelpEntry {
                keys: "f",
                desc: "fetch remotes",
            },
            HelpEntry {
                keys: "p",
                desc: "pull behind",
            },
            HelpEntry {
                keys: "P",
                desc: "push ahead/diverged/new",
            },
            HelpEntry {
                keys: "d",
                desc: "default branch",
            },
            HelpEntry {
                keys: "b",
                desc: "depth 0 picker · graph local/origin/*",
            },
            HelpEntry {
                keys: "m",
                desc: "graph merge into HEAD",
            },
            HelpEntry {
                keys: "c",
                desc: "create branch at graph commit (no checkout)",
            },
            HelpEntry {
                keys: "W",
                desc: "remove linked worktree",
            },
            HelpEntry {
                keys: "r",
                desc: "refresh now",
            },
            HelpEntry {
                keys: "gx",
                desc: "open branch PR in browser",
            },
            HelpEntry {
                keys: "a p D",
                desc: "focused stash apply/pop/drop",
            },
            HelpEntry {
                keys: "A",
                desc: "blame menu",
            },
        ],
    },
    HelpGroup {
        title: "VIEW",
        entries: &[
            HelpEntry {
                keys: "i \\ M B",
                desc: "inline / split · wrap · msg · blame",
            },
            HelpEntry {
                keys: "< > - +",
                desc: "tree width · msg rows (= is +)",
            },
            HelpEntry {
                keys: "t",
                desc: "flat / tree · Staged split",
            },
            HelpEntry {
                keys: ".",
                desc: "show / hide ignored repos",
            },
            HelpEntry {
                keys: "T",
                desc: "cycle theme",
            },
            HelpEntry {
                keys: "Ctrl-o",
                desc: "full-file · keep hunk in view",
            },
            HelpEntry {
                keys: "o O",
                desc: "focus branches / clear (graph · repo)",
            },
            HelpEntry {
                keys: "PgUp PgDn",
                desc: "page focused pane",
            },
            HelpEntry {
                keys: "Ctrl-u Ctrl-d",
                desc: "page focused ±5",
            },
            HelpEntry {
                keys: "m",
                desc: "mouse on/off (graph commit: merge)",
            },
            HelpEntry {
                keys: ";",
                desc: "comment row/line · Ctrl-r resolves in box",
            },
            HelpEntry {
                keys: "V",
                desc: "highlight diff lines for ; / s / u / x / :",
            },
            HelpEntry {
                keys: "y",
                desc: "copy comments as markdown",
            },
            HelpEntry {
                keys: "'",
                desc: "copy entity reference",
            },
            HelpEntry {
                keys: "Esc",
                desc: "back / unfocus · right-click · never quit",
            },
            HelpEntry {
                keys: "Enter dblclick",
                desc: "focus right / drill",
            },
            HelpEntry {
                keys: ": Ctrl-p F",
                desc: "commands · go to file (> first switches)",
            },
            HelpEntry {
                keys: "?",
                desc: "help",
            },
            HelpEntry {
                keys: "gh",
                desc: "icon popover",
            },
            HelpEntry {
                keys: "Tab",
                desc: "other pane",
            },
            HelpEntry {
                keys: "q",
                desc: "quit",
            },
            HelpEntry {
                keys: "Ctrl-c Ctrl-c",
                desc: "quit (press twice)",
            },
        ],
    },
];

/// Compare-tab column that takes the place of GIT while a compare tab is active.
///
/// Lists the keys that act on the compare diff, when `x` may write, and the
/// git actions that stay on the Workspace tab. The tab-close chip is the
/// tab bar glyph.
pub const HELP_COMPARE_GROUP: HelpGroup = HelpGroup {
    title: "COMPARE",
    entries: &[
        HelpEntry {
            keys: "V",
            desc: "highlight for ; x ' :",
        },
        HelpEntry {
            keys: ";",
            desc: "comment (Ctrl-r resolves inside the box)",
        },
        HelpEntry {
            keys: "y",
            desc: "copy comments",
        },
        HelpEntry {
            keys: "'",
            desc: "copy reference",
        },
        HelpEntry {
            keys: "x",
            desc: "revert to merge base (only if head checked out, file clean)",
        },
        HelpEntry {
            keys: "Ctrl-o",
            desc: "full-file",
        },
        HelpEntry {
            keys: "space",
            desc: "mark reviewed",
        },
        HelpEntry {
            keys: "e E",
            desc: "editor · diff tool",
        },
        HelpEntry {
            keys: "A",
            desc: "blame menu",
        },
        HelpEntry {
            keys: super::render::TAB_CLOSE_GLYPH,
            desc: "close tab (or :)",
        },
        HelpEntry {
            keys: "s u",
            desc: "Workspace tab only",
        },
        HelpEntry {
            keys: "f p P d",
            desc: "Workspace tab only",
        },
        HelpEntry {
            keys: "b c W",
            desc: "Workspace tab only",
        },
        HelpEntry {
            keys: "S a p D",
            desc: "Workspace tab only",
        },
        HelpEntry {
            keys: "o O",
            desc: "Workspace tab only",
        },
        HelpEntry {
            keys: "r",
            desc: "refresh now",
        },
    ],
};

/// Help columns on a compare tab: MOVE, [`HELP_COMPARE_GROUP`], VIEW.
pub const HELP_COMPARE_GROUPS: &[HelpGroup] = &[HELP_GROUPS[0], HELP_COMPARE_GROUP, HELP_GROUPS[2]];

/// File-tab column that takes the place of GIT while a file tab is active.
///
/// Lists what acts on the read-only file and the git keys that stay on the
/// Workspace tab. The tab-close chip is the tab bar glyph.
pub const HELP_FILE_GROUP: HelpGroup = HelpGroup {
    title: "FILE",
    entries: &[
        HelpEntry {
            keys: "/ n N",
            desc: "search",
        },
        HelpEntry {
            keys: "\\ B",
            desc: "wrap · line blame",
        },
        HelpEntry {
            keys: "A",
            desc: "blame menu",
        },
        HelpEntry {
            keys: "'",
            desc: "copy reference",
        },
        HelpEntry {
            keys: "e",
            desc: "editor at line",
        },
        HelpEntry {
            keys: "r",
            desc: "reload",
        },
        HelpEntry {
            keys: super::render::TAB_CLOSE_GLYPH,
            desc: "close tab",
        },
        HelpEntry {
            keys: "s u x S",
            desc: "Workspace tab only",
        },
        HelpEntry {
            keys: "f p P d",
            desc: "Workspace tab only",
        },
    ],
};

/// Help columns on a file tab: MOVE, [`HELP_FILE_GROUP`], VIEW.
pub const HELP_FILE_GROUPS: &[HelpGroup] = &[HELP_GROUPS[0], HELP_FILE_GROUP, HELP_GROUPS[2]];

/// Which kind of tab the help overlay describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpTab {
    /// The Workspace tab: MOVE / GIT / VIEW.
    Workspace,
    /// A compare tab: MOVE / COMPARE / VIEW.
    Compare,
    /// A file tab: MOVE / FILE / VIEW.
    File,
}

/// Help columns for the active tab: [`HELP_COMPARE_GROUPS`] on a compare
/// tab, [`HELP_FILE_GROUPS`] on a file tab, else [`HELP_GROUPS`].
pub fn help_groups(tab: HelpTab) -> &'static [HelpGroup] {
    match tab {
        HelpTab::Workspace => HELP_GROUPS,
        HelpTab::Compare => HELP_COMPARE_GROUPS,
        HelpTab::File => HELP_FILE_GROUPS,
    }
}

/// Idle help footer must mention overlay-local `/` search.
pub const HELP_IDLE_FOOTER_SNIPPET: &str = "/ search help";
/// Active help-search footer Esc hint.
pub const HELP_SEARCH_ESC_HINT: &str = "Esc clears search";

/// Lower-right help overlay label: [`crate::version_label`].
///
/// Digits are [`crate::APP_VERSION`]; a dev build adds `-dev (sha)`.
pub fn help_version_label() -> String {
    crate::version_label()
}

/// Flattened help rows in column order (MOVE, then GIT, then VIEW).
#[cfg(test)]
pub fn help_entries() -> impl Iterator<Item = &'static HelpEntry> {
    HELP_GROUPS.iter().flat_map(|group| group.entries.iter())
}

/// Concatenated text matched for a help key row.
pub fn help_entry_label(keys: &str, desc: &str) -> String {
    format!("{keys} {desc}")
}

/// Case-insensitive substring match on keys + description.
/// Empty or whitespace-only query → no match.
pub fn help_entry_matches(keys: &str, desc: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return false;
    }
    help_entry_label(keys, desc).to_lowercase().contains(&q)
}

/// Indices of flattened help entries that match `query`, in order, then
/// matching legend rows numbered on after the entries (`entries + i` for
/// the `i`-th row of [`help_legend_specs`]).
#[cfg(test)]
pub fn help_match_indices(query: &str) -> Vec<usize> {
    let entries = help_entries().count();
    help_entries()
        .enumerate()
        .filter(|(_, e)| help_entry_matches(e.keys, e.desc, query))
        .map(|(i, _)| i)
        .chain(
            help_legend_specs()
                .enumerate()
                .filter(|(_, spec)| help_legend_matches(spec, query))
                .map(|(i, _)| entries + i),
        )
        .collect()
}

/// Round border (2) plus horizontal padding (1 each side).
pub const HELP_CHROME_COLS: usize = 4;

/// Blank columns kept at the right edge of each help column, so one
/// column's text never touches the next column's chips.
pub const HELP_COLUMN_GUTTER: usize = 2;

/// One painted line of a help entry after wrap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpVisualLine {
    /// True on the first line, which also shows the key chips.
    pub chips: bool,
    /// Leading spaces before [`Self::text`] (0 on the chip line).
    pub indent: usize,
    /// Description fragment for this line (empty when chips stand alone).
    pub text: String,
}

/// Chip pad versus description wrap width for one help column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpDescLayout {
    /// Columns before wrapped continuation text.
    pub indent: usize,
    /// Word-wrap width for the description.
    pub width: usize,
    /// When false, chips occupy the first visual line alone.
    pub desc_on_first_line: bool,
}

/// Idle footer painted under the help columns.
pub fn help_idle_footer() -> String {
    format!(
        "Needs a Nerd Font · {} · {HELP_IDLE_FOOTER_SNIPPET} · Esc closes",
        super::icons::REQUIRED_FONT
    )
}

/// Content width inside the help border and horizontal padding.
pub fn help_inner_width(term_width: usize) -> usize {
    term_width.saturating_sub(HELP_CHROME_COLS)
}

/// Painted columns of a key cluster: each chip is ` key ` plus one space.
fn help_chip_used_width(keys: &str) -> usize {
    keys.split(' ')
        .filter(|chip| !chip.is_empty())
        .map(|chip| chip.chars().count() + 3)
        .sum()
}

/// Key-chip column of `group`: its widest chip set plus one gap column,
/// so every description in the column starts at the same x.
pub fn help_key_width(group: &HelpGroup) -> usize {
    group
        .entries
        .iter()
        .map(|entry| help_chip_used_width(entry.keys))
        .max()
        .unwrap_or(0)
        + 1
}

/// Trailing gap spaces after the chips of `keys` so the cluster fills
/// `key_width` (at least one).
pub fn help_chip_gap_spaces(keys: &str, key_width: usize) -> usize {
    1.max(key_width.saturating_sub(help_chip_used_width(keys)))
}

/// Narrowest description wrap beside the key chips. A narrower column puts
/// the chips on their own line and wraps the description at the full text
/// width, so a description never wraps one or two columns wide.
pub const HELP_MIN_DESC_WIDTH: usize = 12;

/// Where `description` goes in a column whose text area is `content_width`
/// wide and whose key chips take `key_width`.
///
/// Beside the chips needs [`HELP_MIN_DESC_WIDTH`] columns and wins only
/// when it paints no more rows than chips on their own line. Rows per entry
/// therefore never grow as the column widens, which
/// [`help_column_widths`] relies on.
pub fn help_desc_layout(
    description: &str,
    content_width: usize,
    key_width: usize,
) -> HelpDescLayout {
    let col = content_width.max(1);
    let below = HelpDescLayout {
        indent: 0,
        width: col,
        desc_on_first_line: false,
    };
    let beside_width = col.saturating_sub(key_width);
    if beside_width < HELP_MIN_DESC_WIDTH {
        return below;
    }
    let beside_rows = wrap_help_description(description, beside_width).len();
    // Chips row plus the non-empty wrapped lines, as
    // [`help_entry_visual_lines`] paints them.
    let below_rows = 1 + wrap_help_description(description, col)
        .iter()
        .filter(|line| !line.is_empty())
        .count();
    if beside_rows <= below_rows {
        HelpDescLayout {
            indent: key_width,
            width: beside_width,
            desc_on_first_line: true,
        }
    } else {
        below
    }
}

/// Word-wrap `text` to `width` columns. Breaks overlong words. Never ellipsizes.
pub fn wrap_help_description(text: &str, width: usize) -> Vec<String> {
    let col = width.max(1);
    let words: Vec<&str> = if text.trim().is_empty() {
        Vec::new()
    } else {
        text.split_whitespace().collect()
    };
    if words.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in words {
        let wlen = word.chars().count();
        if wlen > col {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let chars: Vec<char> = word.chars().collect();
            for chunk in chars.chunks(col) {
                if chunk.len() == col {
                    lines.push(chunk.iter().collect());
                } else {
                    current = chunk.iter().collect();
                }
            }
            continue;
        }
        let next = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if next.chars().count() <= col {
            current = next;
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

/// Wrap the help footer to the overlay inner width.
pub fn wrap_help_footer(text: &str, inner_width: usize) -> Vec<String> {
    wrap_help_description(text, inner_width.max(1))
}

/// Right-align [`help_version_label`] on the last of `lines` within `width`.
///
/// Adds a row only when the last line cannot share the version without
/// crowding the footer copy.
pub fn attach_help_version(lines: &mut Vec<String>, width: usize) {
    use crate::helpers::visible_width;

    let version = help_version_label();
    let vw = visible_width(&version);
    let width = width.max(1);
    if lines.is_empty() {
        lines.push(String::new());
    }
    let last = lines.last().expect("footer lines");
    let lw = visible_width(last);
    let gap_needed = usize::from(lw > 0);
    if lw + gap_needed + vw <= width {
        let gap = width - lw - vw;
        let last = lines.last_mut().expect("footer lines");
        last.push_str(&" ".repeat(gap));
        last.push_str(&version);
    } else {
        let gap = width.saturating_sub(vw);
        lines.push(format!("{}{version}", " ".repeat(gap)));
    }
}

/// Idle footer rows, with the package version in the lower-right.
pub fn help_idle_footer_lines(inner_width: usize) -> Vec<String> {
    let inner = inner_width.max(1);
    let mut lines = wrap_help_footer(&help_idle_footer(), inner);
    attach_help_version(&mut lines, inner);
    lines
}

/// Visual lines for one help entry in a column whose text area is
/// `content_width` wide and whose key chips take `key_width`.
pub fn help_entry_visual_lines(
    description: &str,
    content_width: usize,
    key_width: usize,
) -> Vec<HelpVisualLine> {
    let layout = help_desc_layout(description, content_width, key_width);
    let wrapped = wrap_help_description(description, layout.width);
    if !layout.desc_on_first_line {
        let mut out = vec![HelpVisualLine {
            chips: true,
            indent: 0,
            text: String::new(),
        }];
        out.extend(
            wrapped
                .into_iter()
                .filter(|text| !text.is_empty())
                .map(|text| HelpVisualLine {
                    chips: false,
                    indent: layout.indent,
                    text,
                }),
        );
        out
    } else {
        wrapped
            .into_iter()
            .enumerate()
            .map(|(i, text)| HelpVisualLine {
                chips: i == 0,
                indent: if i == 0 { 0 } else { layout.indent },
                text,
            })
            .collect()
    }
}

/// Text area of a help column `column_width` wide: the column minus
/// [`HELP_COLUMN_GUTTER`].
pub fn help_column_content_width(column_width: usize) -> usize {
    column_width.saturating_sub(HELP_COLUMN_GUTTER).max(1)
}

/// Painted rows of one help column at `column_width`: its entries stack
/// on their own, so a wrapped entry never pads another column.
pub fn help_column_line_count(group: &HelpGroup, column_width: usize) -> usize {
    let content = help_column_content_width(column_width);
    let key_width = help_key_width(group);
    group
        .entries
        .iter()
        .map(|entry| help_entry_visual_lines(entry.desc, content, key_width).len())
        .sum()
}

/// Narrowest width in `floor..=cap` at which `group` paints in `rows` or
/// fewer, or `None` when even `cap` needs more. Binary search is sound
/// because a column's rows never grow as it widens (see
/// [`help_desc_layout`]).
fn help_min_column_width(
    group: &HelpGroup,
    floor: usize,
    cap: usize,
    rows: usize,
) -> Option<usize> {
    if floor > cap || help_column_line_count(group, cap) > rows {
        return None;
    }
    let (mut lo, mut hi) = (floor, cap);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if help_column_line_count(group, mid) <= rows {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}

/// Narrowest width of a help column: its chips (at least
/// [`HELP_MIN_DESC_WIDTH`] text columns) plus the gutter.
fn help_column_floor(group: &HelpGroup) -> usize {
    help_key_width(group).max(HELP_MIN_DESC_WIDTH) + HELP_COLUMN_GUTTER
}

/// Column widths for `groups` inside `inner_width`.
///
/// Columns flow on their own, so the overlay is as tall as its tallest
/// column. The widths are the most even split that keeps that column as
/// short as possible: a column with long rows (VIEW) takes width from a
/// short one (MOVE). Each column keeps room for its chips and the gutter
/// ([`help_column_floor`]); when the terminal is too narrow for that the
/// split is even.
pub fn help_column_widths(groups: &[HelpGroup], inner_width: usize) -> Vec<usize> {
    let count = groups.len().max(1);
    let even = vec![(inner_width / count).max(1); groups.len()];
    let floors: Vec<usize> = groups.iter().map(help_column_floor).collect();
    let floor_sum: usize = floors.iter().sum();
    if groups.is_empty() || floor_sum > inner_width {
        return even;
    }
    let fit = |rows: usize| -> Option<Vec<usize>> {
        let widths = groups
            .iter()
            .zip(&floors)
            .map(|(group, &floor)| {
                help_min_column_width(group, floor, inner_width - (floor_sum - floor), rows)
            })
            .collect::<Option<Vec<usize>>>()?;
        (widths.iter().sum::<usize>() <= inner_width).then_some(widths)
    };
    // Every column at its floor always fits, so `hi` is a valid bound.
    let mut lo = groups.iter().map(|g| g.entries.len()).max().unwrap_or(0);
    let mut hi = help_body_line_count(groups, &floors);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if fit(mid).is_some() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let Some(mut widths) = fit(lo) else {
        return even;
    };
    // Hand the spare columns to the narrowest column first, so the split
    // stays as even as the tallest column allows.
    for _ in widths.iter().sum::<usize>()..inner_width {
        let narrowest = (0..widths.len())
            .min_by_key(|&i| widths[i])
            .expect("at least one column");
        widths[narrowest] += 1;
    }
    widths
}

/// Body rows: the tallest column at `widths` (see [`help_column_widths`]).
pub fn help_body_line_count(groups: &[HelpGroup], widths: &[usize]) -> usize {
    groups
        .iter()
        .zip(widths)
        .map(|(group, &width)| help_column_line_count(group, width))
        .max()
        .unwrap_or(0)
}

/// Legend title row under the key columns.
pub const HELP_LEGEND_TITLE: &str = "ICONS";

/// The legend flows into at most this many columns.
pub const HELP_LEGEND_MAX_COLUMNS: usize = 3;

/// Narrowest legend column. The legend uses as many columns of at least
/// this width as fit, up to [`HELP_LEGEND_MAX_COLUMNS`].
pub const HELP_LEGEND_MIN_COLUMN_WIDTH: usize = 40;

/// Rows above the legend columns: one blank row, then the title.
pub const HELP_LEGEND_HEAD_ROWS: usize = 2;

/// Catalog rows the legend lists, in catalog order (Tree, Graph, Chrome).
pub fn help_legend_specs() -> impl Iterator<Item = &'static IconSpec> {
    ICON_CATALOG.iter().filter(|spec| spec.group.is_some())
}

/// Case-insensitive match on a legend row's name and meaning.
pub fn help_legend_matches(spec: &IconSpec, query: &str) -> bool {
    help_entry_matches(spec.name, spec.meaning, query)
}

/// Legend column widths, measured once from the static catalog.
struct HelpLegendWidths {
    /// Widest glyph of each legend group in either glyph mode, in
    /// [`IconGroup`] order (Tree, Graph, Chrome).
    glyph: [usize; 3],
    /// Longest legend name.
    name: usize,
}

static HELP_LEGEND_WIDTHS: LazyLock<HelpLegendWidths> = LazyLock::new(|| {
    use crate::helpers::visible_width;
    let mut glyph = [0; 3];
    for spec in help_legend_specs() {
        if let Some(group) = spec.group {
            let width = visible_width(spec.nerd).max(visible_width(spec.ascii));
            let slot = &mut glyph[group_slot(group)];
            *slot = (*slot).max(width);
        }
    }
    HelpLegendWidths {
        glyph,
        name: help_legend_specs()
            .map(|spec| spec.name.chars().count())
            .max()
            .unwrap_or(0),
    }
});

fn group_slot(group: IconGroup) -> usize {
    match group {
        IconGroup::Tree => 0,
        IconGroup::Graph => 1,
        IconGroup::Chrome => 2,
    }
}

/// Glyph column of legend group `group`: its widest glyph in either glyph
/// mode, so one-column glyphs do not pad to the graph's chip samples.
pub fn help_legend_glyph_width(group: IconGroup) -> usize {
    HELP_LEGEND_WIDTHS.glyph[group_slot(group)]
}

/// Name column: the longest legend name.
pub fn help_legend_name_width() -> usize {
    HELP_LEGEND_WIDTHS.name
}

/// Columns before a legend meaning in group `group`: glyph, gap, name,
/// gap.
pub fn help_legend_key_width(group: IconGroup) -> usize {
    help_legend_glyph_width(group) + 1 + help_legend_name_width() + 1
}

/// [`help_legend_key_width`] of the group `spec` is listed under.
fn spec_key_width(spec: &IconSpec) -> usize {
    spec.group.map_or(0, help_legend_key_width)
}

/// One legend block: an optional group heading row, then one catalog row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpLegendBlock {
    /// Heading painted above the row when it starts its group.
    pub heading: Option<IconGroup>,
    /// The catalog row.
    pub spec: &'static IconSpec,
}

/// Legend columns at one inner width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpLegendLayout {
    /// Width of every legend column (the rest of the inner width is blank).
    pub column_width: usize,
    /// Blocks per column, top to bottom.
    pub columns: Vec<Vec<HelpLegendBlock>>,
}

/// Visual lines of one legend row in a column `column_width` wide.
///
/// The meaning sits beside the glyph and name and wraps under the meaning
/// column, never under the glyph. Only a column too narrow for
/// [`HELP_MIN_DESC_WIDTH`] beside them puts the meaning on its own lines.
pub fn help_legend_visual_lines(spec: &IconSpec, column_width: usize) -> Vec<HelpVisualLine> {
    let content = help_column_content_width(column_width);
    let key_width = spec_key_width(spec);
    let beside = content.saturating_sub(key_width);
    if beside >= HELP_MIN_DESC_WIDTH {
        return wrap_help_description(spec.meaning, beside)
            .into_iter()
            .enumerate()
            .map(|(i, text)| HelpVisualLine {
                chips: i == 0,
                indent: if i == 0 { 0 } else { key_width },
                text,
            })
            .collect();
    }
    let mut lines = vec![HelpVisualLine {
        chips: true,
        indent: 0,
        text: String::new(),
    }];
    lines.extend(
        wrap_help_description(spec.meaning, content)
            .into_iter()
            .filter(|text| !text.is_empty())
            .map(|text| HelpVisualLine {
                chips: false,
                indent: 0,
                text,
            }),
    );
    lines
}

/// Painted rows of `block` in a column `column_width` wide.
pub fn help_legend_block_rows(block: &HelpLegendBlock, column_width: usize) -> usize {
    usize::from(block.heading.is_some()) + help_legend_visual_lines(block.spec, column_width).len()
}

/// Lay the legend out at `inner_width`: as many even columns as fit, each
/// filled top to bottom, as short as the blocks allow. A heading never
/// sits alone at the foot of a column.
pub fn help_legend_layout(inner_width: usize) -> HelpLegendLayout {
    let count = (inner_width / HELP_LEGEND_MIN_COLUMN_WIDTH).clamp(1, HELP_LEGEND_MAX_COLUMNS);
    let column_width = (inner_width / count).max(1);
    let mut prev = None;
    let blocks: Vec<HelpLegendBlock> = help_legend_specs()
        .map(|spec| {
            let heading = (spec.group != prev).then_some(spec.group).flatten();
            prev = spec.group;
            HelpLegendBlock { heading, spec }
        })
        .collect();
    let rows: Vec<usize> = blocks
        .iter()
        .map(|block| help_legend_block_rows(block, column_width))
        .collect();
    let total: usize = rows.iter().sum();
    let tallest = rows.iter().copied().max().unwrap_or(0);
    // Greedy fill at height `cap`; the first cap that needs no more than
    // `count` columns is the shortest.
    let fill = |cap: usize| -> Vec<Vec<HelpLegendBlock>> {
        let mut columns: Vec<Vec<HelpLegendBlock>> = vec![Vec::new()];
        let mut used = 0usize;
        for (block, &h) in blocks.iter().zip(&rows) {
            if used + h > cap && !columns.last().is_some_and(Vec::is_empty) {
                columns.push(Vec::new());
                used = 0;
            }
            used += h;
            columns.last_mut().expect("column").push(*block);
        }
        columns
    };
    let mut cap = tallest.max(total.div_ceil(count));
    let columns = loop {
        let columns = fill(cap);
        if columns.len() <= count {
            break columns;
        }
        cap += 1;
    };
    HelpLegendLayout {
        column_width,
        columns,
    }
}

/// Legend rows at `inner_width`: [`HELP_LEGEND_HEAD_ROWS`] plus the
/// tallest legend column.
pub fn help_legend_line_count(inner_width: usize) -> usize {
    let layout = help_legend_layout(inner_width);
    let tallest = layout
        .columns
        .iter()
        .map(|column| {
            column
                .iter()
                .map(|block| help_legend_block_rows(block, layout.column_width))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    if tallest == 0 {
        0
    } else {
        HELP_LEGEND_HEAD_ROWS + tallest
    }
}

/// Overlay rows: border (2) + title + wrapped body + footer.
pub fn help_overlay_row_count(body_rows: usize, footer_rows: usize) -> usize {
    2 + 1 + body_rows + footer_rows
}

/// Full overlay height for `groups` at `term_width` with a wrappable footer.
///
/// The body is the key columns plus the icon legend. The lower-right
/// package version is part of the footer row budget.
pub fn help_overlay_height(groups: &[HelpGroup], term_width: usize, footer: &str) -> usize {
    let inner = help_inner_width(term_width).max(1);
    let body = help_body_line_count(groups, &help_column_widths(groups, inner))
        + help_legend_line_count(inner);
    let mut footer_lines = wrap_help_footer(footer, inner);
    attach_help_version(&mut footer_lines, inner);
    help_overlay_row_count(body, footer_lines.len().max(1))
}

/// Help dialog height at box width `term_cols`, for the columns
/// [`help_groups`] paints on `tab`.
pub fn help_status_lines(term_cols: u16, tab: HelpTab) -> u16 {
    help_overlay_height(
        help_groups(tab),
        usize::from(term_cols.max(1)),
        &help_idle_footer(),
    ) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive_keys_or_desc() {
        assert!(help_entry_matches(
            "Ctrl-o",
            "full-file · keep hunk in view",
            "ctrl"
        ));
        assert!(help_entry_matches(
            "Ctrl-o",
            "full-file · keep hunk in view",
            "FILE"
        ));
        assert!(!help_entry_matches(
            "Ctrl-o",
            "full-file · keep hunk in view",
            "zzz"
        ));
    }

    #[test]
    fn empty_query_is_not_a_match() {
        assert!(!help_entry_matches("j k", "down / up", ""));
        assert!(!help_entry_matches("j k", "down / up", "   "));
    }

    #[test]
    fn groups_are_move_git_view() {
        assert_eq!(HELP_GROUPS.len(), HELP_COLUMN_COUNT);
        assert_eq!(HELP_GROUPS[0].title, "MOVE");
        assert_eq!(HELP_GROUPS[1].title, "GIT");
        assert_eq!(HELP_GROUPS[2].title, "VIEW");
        let keys: Vec<&str> = help_entries().map(|e| e.keys).collect();
        assert!(keys.contains(&"q"));
        assert!(keys.contains(&"Tab"));
        assert!(keys.contains(&"c"));
        assert!(keys.contains(&"a p D"));
        assert!(keys.contains(&"Home End"));
        let git_keys: Vec<&str> = HELP_GROUPS[1].entries.iter().map(|e| e.keys).collect();
        assert!(git_keys.contains(&"m"));
        assert!(git_keys.contains(&"A"));
        for group in [&HELP_COMPARE_GROUP, &HELP_FILE_GROUP] {
            assert!(
                group.entries.iter().any(|e| e.keys == "A"),
                "{}",
                group.title
            );
        }
        assert!(git_keys.contains(&"e E"));
        assert_eq!(
            HELP_GROUPS[1]
                .entries
                .iter()
                .find(|e| e.keys == "gx")
                .map(|e| e.desc),
            Some("open branch PR in browser")
        );
        let move_keys: Vec<&str> = HELP_GROUPS[0].entries.iter().map(|e| e.keys).collect();
        let view_keys: Vec<&str> = HELP_GROUPS[2].entries.iter().map(|e| e.keys).collect();
        assert!(move_keys.contains(&"j k"));
        assert!(move_keys.contains(&"gt gT"));
        assert!(move_keys.contains(&"g1-g9"));
        assert!(move_keys.contains(&"/"));
        assert!(move_keys.contains(&"n N"));
        assert!(!move_keys.contains(&"PgUp PgDn"));
        assert!(!move_keys.contains(&"Ctrl-u Ctrl-d"));
        assert!(view_keys.contains(&"PgUp PgDn"));
        assert!(view_keys.contains(&"Ctrl-u Ctrl-d"));
        assert!(view_keys.contains(&"."));
        assert!(view_keys.contains(&"T"));
        assert!(view_keys.contains(&"gh"), "the icon popover chord");
        assert!(view_keys.contains(&"i \\ M B"));
        assert!(!view_keys.contains(&"i \\"));
        assert!(!view_keys.contains(&"i"));
        assert!(view_keys.contains(&"Ctrl-o"));
        assert!(view_keys.contains(&": Ctrl-p F"));
        assert!(view_keys.contains(&"?"));
        assert!(!view_keys.iter().any(|keys| keys.contains("Ctrl-k")));
        assert!(view_keys.contains(&"o O"));
        assert!(view_keys.contains(&"m"));
        assert!(view_keys.contains(&";"));
        assert!(
            !view_keys.contains(&"; Ctrl-R"),
            "Ctrl-r only resolves inside the box"
        );
        assert!(view_keys.contains(&"V"));
        assert!(view_keys.contains(&"y"));
        assert!(view_keys.contains(&"'"));
        assert!(!view_keys.contains(&"y '"));
        assert!(view_keys.contains(&"Esc"));
        assert!(view_keys.contains(&"< > - +"));
        assert_eq!(
            HELP_GROUPS[2]
                .entries
                .iter()
                .find(|e| e.keys == "t")
                .map(|e| e.desc),
            Some("flat / tree · Staged split")
        );
        assert_eq!(
            HELP_GROUPS[2]
                .entries
                .iter()
                .find(|e| e.keys == "o O")
                .map(|e| e.desc),
            Some("focus branches / clear (graph · repo)")
        );
        assert!(help_match_indices("quit")
            .iter()
            .any(|&i| { help_entries().nth(i).is_some_and(|e| e.keys == "q") }));
        assert!(view_keys.contains(&"Ctrl-c Ctrl-c"));
        let git_c = HELP_GROUPS[1].entries.iter().find(|e| e.keys == "c");
        assert_eq!(
            git_c.map(|e| e.desc),
            Some("create branch at graph commit (no checkout)")
        );
        let view_m = HELP_GROUPS[2].entries.iter().find(|e| e.keys == "m");
        assert_eq!(
            view_m.map(|e| e.desc),
            Some("mouse on/off (graph commit: merge)")
        );
    }

    /// One key spelling: `Ctrl-` plus the key as typed (`Ctrl-o`, never
    /// `ctrl+o`, `Ctrl+O` or `Ctrl-O`), and `Enter`, not `⏎`.
    #[test]
    fn help_keys_use_one_spelling() {
        let groups = HELP_GROUPS
            .iter()
            .chain([&HELP_COMPARE_GROUP, &HELP_FILE_GROUP]);
        for entry in groups.flat_map(|g| g.entries.iter()) {
            let text = help_entry_label(entry.keys, entry.desc);
            assert!(!text.contains("Ctrl+"), "{text}");
            assert!(!text.to_lowercase().contains("ctrl+"), "{text}");
            assert!(!text.contains('⏎'), "{text}");
            assert!(!text.contains("Shift+"), "{text}");
            for (at, _) in text.match_indices("Ctrl-") {
                let key = text[at + 5..].chars().next().unwrap_or(' ');
                assert!(
                    !key.is_ascii_uppercase(),
                    "lowercase key after Ctrl-: {text}"
                );
            }
        }
    }

    #[test]
    fn overlay_height_grows_when_columns_narrow() {
        let row_count = HELP_GROUPS
            .iter()
            .map(|group| group.entries.len())
            .max()
            .unwrap_or(0);
        let legend = |cols: usize| help_legend_line_count(help_inner_width(cols)) as u16;
        let wide = help_status_lines(300, HelpTab::Workspace);
        let mid = help_status_lines(128, HelpTab::Workspace);
        let narrow = help_status_lines(80, HelpTab::Workspace);
        assert_eq!(wide, (2 + 1 + row_count + 1) as u16 + legend(300));
        assert!(
            mid - legend(128) > wide - legend(300),
            "128 cols still wraps some descriptions"
        );
        assert!(
            narrow - legend(80) > mid - legend(128),
            "narrow terminals wrap more and take more rows"
        );
        let at_140 = help_status_lines(140, HelpTab::Workspace) - legend(140);
        assert!(
            at_140 <= 26,
            "at 140×40 the key columns fit without scrolling; only the legend \
             scrolls (render `help_columns_keep_a_gutter_and_the_panes_rows`): {at_140}"
        );
    }

    /// Columns flow on their own: the body is the tallest column, not the
    /// sum of row-aligned maxima, and a long column takes width from a
    /// short one.
    #[test]
    fn columns_flow_on_their_own() {
        let inner = help_inner_width(140);
        let widths = help_column_widths(HELP_GROUPS, inner);
        assert_eq!(widths.len(), HELP_COLUMN_COUNT);
        assert_eq!(widths.iter().sum::<usize>(), inner, "{widths:?}");
        let heights: Vec<usize> = HELP_GROUPS
            .iter()
            .zip(&widths)
            .map(|(group, &width)| help_column_line_count(group, width))
            .collect();
        assert_eq!(
            help_body_line_count(HELP_GROUPS, &widths),
            *heights.iter().max().unwrap()
        );
        let even = vec![inner / HELP_COLUMN_COUNT; HELP_COLUMN_COUNT];
        assert!(
            help_body_line_count(HELP_GROUPS, &widths) <= help_body_line_count(HELP_GROUPS, &even),
            "the split never paints taller than an even split"
        );
        assert!(widths[2] > widths[0], "VIEW is wider than MOVE: {widths:?}");
    }

    /// Every chip set in a column ends by the same x, so descriptions line
    /// up; the key column is the widest set plus one gap.
    #[test]
    fn key_width_follows_the_widest_chip_set() {
        for group in HELP_GROUPS
            .iter()
            .chain([&HELP_COMPARE_GROUP, &HELP_FILE_GROUP])
        {
            let key_width = help_key_width(group);
            let widest = group
                .entries
                .iter()
                .map(|e| help_chip_used_width(e.keys))
                .max()
                .unwrap();
            assert_eq!(key_width, widest + 1, "{}", group.title);
            for entry in group.entries {
                assert_eq!(
                    help_chip_used_width(entry.keys) + help_chip_gap_spaces(entry.keys, key_width),
                    key_width,
                    "{} {}",
                    group.title,
                    entry.keys
                );
            }
        }
        // VIEW holds `Enter dblclick` (19 painted columns).
        assert_eq!(help_key_width(&HELP_GROUPS[2]), 20);
    }

    /// A column too narrow for [`HELP_MIN_DESC_WIDTH`] beside the chips
    /// puts the chips on their own line and wraps at the full text width.
    #[test]
    fn narrow_column_puts_chips_on_their_own_line() {
        let desc = "toggle fold (instant; no-op on graph/diff)";
        let key_width = 14;
        let narrow = help_desc_layout(desc, key_width + HELP_MIN_DESC_WIDTH - 1, key_width);
        assert!(!narrow.desc_on_first_line, "{narrow:?}");
        assert_eq!(narrow.width, key_width + HELP_MIN_DESC_WIDTH - 1);
        assert_eq!(narrow.indent, 0);
        let lines = help_entry_visual_lines(desc, 25, key_width);
        assert!(lines[0].chips && lines[0].text.is_empty(), "{lines:?}");
        assert!(lines[1..].iter().all(|l| !l.chips && l.indent == 0));
        let wide = help_desc_layout(desc, 80, key_width);
        assert!(wide.desc_on_first_line, "{wide:?}");
        assert_eq!((wide.indent, wide.width), (key_width, 80 - key_width));
        // Short text sits beside the chips once the minimum fits.
        let short = help_desc_layout("quit", key_width + HELP_MIN_DESC_WIDTH, key_width);
        assert!(short.desc_on_first_line, "{short:?}");
    }

    /// A column never paints more rows when it gets wider, so the width
    /// search in [`help_column_widths`] can bisect.
    #[test]
    fn column_rows_never_grow_with_width() {
        for group in HELP_GROUPS
            .iter()
            .chain([&HELP_COMPARE_GROUP, &HELP_FILE_GROUP])
        {
            let mut prev = usize::MAX;
            for width in help_column_floor(group)..=240 {
                let rows = help_column_line_count(group, width);
                assert!(rows <= prev, "{} at {width}: {rows} > {prev}", group.title);
                prev = rows;
            }
        }
    }

    /// Narrow terminals paint no taller than the row-aligned layout did
    /// (equal thirds, rows aligned across columns) and stay within two rows
    /// of the measured reflow height, and no description wraps narrower
    /// than [`HELP_MIN_DESC_WIDTH`].
    #[test]
    fn narrow_terminals_stay_under_the_row_aligned_height() {
        // Slack over the measured reflow height before the test fails.
        const SLACK: usize = 2;
        // (terminal cols, tab, row-aligned body rows, reflow body rows as
        // measured).
        for (term, tab, row_aligned, measured) in [
            (60usize, HelpTab::Workspace, 60usize, 56usize),
            (64, HelpTab::Compare, 251, 56),
            (80, HelpTab::Workspace, 86, 45),
            (100, HelpTab::Workspace, 47, 30),
            (140, HelpTab::Workspace, 28, 22),
        ] {
            let groups = help_groups(tab);
            let widths = help_column_widths(groups, help_inner_width(term));
            let body = help_body_line_count(groups, &widths);
            assert!(
                body <= row_aligned,
                "{term} cols: {body} rows > {row_aligned} ({widths:?})"
            );
            assert!(
                body <= measured + SLACK,
                "{term} cols: {body} rows > measured {measured} + {SLACK} ({widths:?})"
            );
            for (group, &width) in groups.iter().zip(&widths) {
                let content = help_column_content_width(width);
                let key_width = help_key_width(group);
                for entry in group.entries {
                    let layout = help_desc_layout(entry.desc, content, key_width);
                    assert!(
                        layout.width >= HELP_MIN_DESC_WIDTH,
                        "{term} cols {} {}: {layout:?}",
                        group.title,
                        entry.keys
                    );
                }
            }
        }
    }

    /// Wrapped text stays inside the column text area, so the gutter keeps
    /// two blank columns before the next column.
    #[test]
    fn descriptions_leave_the_gutter_blank() {
        for term in [60usize, 64, 80, 100, 120, 140, 200] {
            let inner = help_inner_width(term);
            for tab in [HelpTab::Workspace, HelpTab::Compare, HelpTab::File] {
                let groups = help_groups(tab);
                for (group, width) in groups.iter().zip(help_column_widths(groups, inner)) {
                    let content = help_column_content_width(width);
                    let key_width = help_key_width(group);
                    for entry in group.entries {
                        for line in help_entry_visual_lines(entry.desc, content, key_width) {
                            let start = if line.chips { key_width } else { line.indent };
                            if line.text.is_empty() {
                                continue;
                            }
                            assert!(
                                start + line.text.chars().count() + HELP_COLUMN_GUTTER <= width,
                                "{term} cols {}: {line:?}",
                                group.title
                            );
                        }
                    }
                }
            }
        }
    }

    /// A file tab paints MOVE / FILE / VIEW; FILE lists the viewer keys and
    /// the git keys that need the Workspace tab.
    #[test]
    fn file_column_lists_viewer_keys() {
        let rows: Vec<String> = HELP_FILE_GROUP
            .entries
            .iter()
            .map(|e| help_entry_label(e.keys, e.desc))
            .collect();
        assert_eq!(
            rows,
            [
                "/ n N search",
                "\\ B wrap · line blame",
                "A blame menu",
                "' copy reference",
                "e editor at line",
                "r reload",
                format!("{} close tab", super::super::render::TAB_CLOSE_GLYPH).as_str(),
                "s u x S Workspace tab only",
                "f p P d Workspace tab only",
            ]
        );
    }

    /// A compare tab paints MOVE / COMPARE / VIEW; COMPARE lists what acts
    /// on the compare diff and what needs the Workspace tab. The row budget
    /// is checked against the paint in `render.rs`
    /// (`compare_help_paints_centered_and_scrolls_every_body_row`,
    /// `file_help_paints_its_reserved_rows`).
    #[test]
    fn compare_column_lists_compare_keys() {
        assert_eq!(HELP_COMPARE_GROUPS.len(), HELP_COLUMN_COUNT);
        assert_eq!(help_groups(HelpTab::Workspace), HELP_GROUPS);
        assert_eq!(help_groups(HelpTab::Compare)[1].title, "COMPARE");
        assert_eq!(help_groups(HelpTab::Compare)[0], HELP_GROUPS[0]);
        assert_eq!(help_groups(HelpTab::Compare)[2], HELP_GROUPS[2]);
        assert_eq!(help_groups(HelpTab::File)[1].title, "FILE");
        assert_eq!(help_groups(HelpTab::File)[0], HELP_GROUPS[0]);
        assert_eq!(help_groups(HelpTab::File)[2], HELP_GROUPS[2]);
        let text: String = HELP_COMPARE_GROUP
            .entries
            .iter()
            .map(|e| help_entry_label(e.keys, e.desc))
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "V ",
            "; comment (Ctrl-r resolves",
            "y ",
            "' ",
            "x ",
            "merge base",
            "Ctrl-o",
            "space",
            "e E",
            "close tab",
            "s u",
            "f p P d",
            "b c W",
            "S a p D",
            "o O",
            "r refresh now",
        ] {
            assert!(text.contains(needle), "{needle} missing: {text}");
        }
        // `m` toggles mouse on a compare tab (no graph commit to merge).
        assert!(!text.contains("b m c W"), "{text}");
        let x_rows = HELP_COMPARE_GROUP
            .entries
            .iter()
            .filter(|e| e.keys.split(' ').any(|k| k == "x"))
            .count();
        assert_eq!(x_rows, 1, "one `x` chip: {text}");
    }

    /// The legend lists every catalog row with a legend group, under one
    /// heading per group in Tree / Graph / Chrome order, and never adds a
    /// key column.
    #[test]
    fn legend_lists_grouped_catalog_rows_under_headings() {
        use crate::tui::icons::IconKind;
        assert_eq!(HELP_GROUPS.len(), HELP_COLUMN_COUNT);
        let grouped = ICON_CATALOG.iter().filter(|s| s.group.is_some()).count();
        assert_eq!(help_legend_specs().count(), grouped);
        for kind in [
            IconKind::LinkedWorktree,
            IconKind::StatusConflict,
            IconKind::GraphStash,
            IconKind::ChipOverflow,
            IconKind::GraphRails,
        ] {
            assert!(help_legend_specs().any(|s| s.kind == kind), "{kind:?}");
        }
        for kind in [IconKind::Synced, IconKind::HelpMove, IconKind::FolderOpen] {
            assert!(!help_legend_specs().any(|s| s.kind == kind), "{kind:?}");
        }
        for inner in [42usize, 76, 96, 136, 296] {
            let layout = help_legend_layout(inner);
            let blocks: Vec<&HelpLegendBlock> = layout.columns.iter().flatten().collect();
            assert_eq!(blocks.len(), grouped, "{inner}");
            let headings: Vec<IconGroup> = blocks.iter().filter_map(|b| b.heading).collect();
            assert_eq!(
                headings,
                [IconGroup::Tree, IconGroup::Graph, IconGroup::Chrome],
                "{inner}"
            );
        }
    }

    /// The legend uses as many even columns as fit, up to three, and the
    /// shortest fill: no column is taller than it must be.
    #[test]
    fn legend_flows_into_even_columns() {
        for (inner, count) in [
            (38usize, 1usize),
            (42, 1),
            (79, 1),
            (80, 2),
            (96, 2),
            (136, 3),
            (296, 3),
        ] {
            let layout = help_legend_layout(inner);
            assert_eq!(layout.columns.len(), count, "{inner}");
            assert_eq!(layout.column_width, inner / count, "{inner}");
            let heights: Vec<usize> = layout
                .columns
                .iter()
                .map(|c| {
                    c.iter()
                        .map(|b| help_legend_block_rows(b, layout.column_width))
                        .sum()
                })
                .collect();
            let tallest = *heights.iter().max().unwrap();
            assert_eq!(
                help_legend_line_count(inner),
                HELP_LEGEND_HEAD_ROWS + tallest,
                "{inner}"
            );
            let total: usize = heights.iter().sum();
            assert!(tallest < total.div_ceil(count) + 8, "{inner}: {heights:?}");
        }
    }

    /// Legend text stays inside its column and leaves the gutter blank;
    /// the meaning wraps under the meaning column, not under the glyph.
    #[test]
    fn legend_text_stays_in_its_column() {
        let widest = [IconGroup::Tree, IconGroup::Graph, IconGroup::Chrome]
            .into_iter()
            .map(help_legend_key_width)
            .max()
            .unwrap_or(0);
        for term in [46usize, 60, 64, 80, 100, 120, 140, 200] {
            let inner = help_inner_width(term);
            let layout = help_legend_layout(inner);
            let content = help_column_content_width(layout.column_width);
            // Every painted width (46 columns and up) fits the meaning beside.
            assert!(content >= widest + HELP_MIN_DESC_WIDTH, "{term}");
            for block in layout.columns.iter().flatten() {
                let key_width = help_legend_key_width(block.spec.group.expect("legend row"));
                let lines = help_legend_visual_lines(block.spec, layout.column_width);
                assert!(!lines[0].text.is_empty(), "{term} {block:?}");
                for line in &lines {
                    let start = if line.chips { key_width } else { line.indent };
                    assert!(
                        start + line.text.chars().count() + HELP_COLUMN_GUTTER
                            <= layout.column_width,
                        "{term} {block:?}: {line:?}"
                    );
                    if !line.chips {
                        assert_eq!(line.indent, key_width, "{term} {block:?}");
                    }
                }
            }
        }
        // Below that the meaning drops under the glyph and name.
        let tree = help_legend_key_width(IconGroup::Tree);
        let narrow = help_legend_visual_lines(&ICON_CATALOG[0], tree + 4);
        assert!(narrow[0].chips && narrow[0].text.is_empty(), "{narrow:?}");
        assert!(narrow[1..].iter().all(|l| !l.chips && l.indent == 0));
    }

    /// `/` help search matches legend names and meanings; the legend rows
    /// count after the key entries.
    #[test]
    fn help_search_matches_legend_rows() {
        use crate::tui::icons::IconKind;
        let entries = help_entries().count();
        let at = |kind: IconKind| {
            entries
                + help_legend_specs()
                    .position(|s| s.kind == kind)
                    .expect("legend row")
        };
        let hits = help_match_indices("worktree");
        assert!(hits.contains(&at(IconKind::LinkedWorktree)), "{hits:?}");
        let hits = help_match_indices("STASH");
        assert!(hits.contains(&at(IconKind::GraphStash)), "{hits:?}");
        assert!(hits.iter().any(|&i| i < entries), "key rows still match");
        assert!(help_match_indices("conflict").contains(&at(IconKind::StatusConflict)));
        assert!(help_match_indices("").is_empty());
    }

    /// The dialog height counts the legend under the key columns.
    #[test]
    fn status_lines_count_the_legend() {
        for term in [60usize, 100, 140, 200] {
            for tab in [HelpTab::Workspace, HelpTab::Compare, HelpTab::File] {
                let groups = help_groups(tab);
                let inner = help_inner_width(term);
                let keys = help_body_line_count(groups, &help_column_widths(groups, inner));
                let footer = help_idle_footer_lines(inner).len();
                assert_eq!(
                    usize::from(help_status_lines(term as u16, tab)),
                    help_overlay_row_count(keys + help_legend_line_count(inner), footer),
                    "{term} {tab:?}"
                );
            }
        }
    }

    #[test]
    fn version_label_is_cargo_pkg_version() {
        assert_eq!(help_version_label(), crate::version_label());
        assert!(help_version_label().starts_with(&format!("v{}", crate::APP_VERSION)));
        assert_eq!(crate::APP_VERSION, env!("CARGO_PKG_VERSION"));
        for entry in help_entries() {
            assert!(
                !entry.keys.contains(crate::APP_VERSION)
                    && !entry.desc.contains(crate::APP_VERSION),
                "package version belongs in the footer, not the keymap: {entry:?}"
            );
        }
    }

    #[test]
    fn idle_footer_right_aligns_version() {
        let inner = 80;
        let lines = help_idle_footer_lines(inner);
        let last = lines.last().expect("footer");
        assert!(
            last.ends_with(&help_version_label()),
            "version should sit on the right: {last:?}"
        );
        assert!(
            last.contains(HELP_IDLE_FOOTER_SNIPPET),
            "footer copy must stay: {last:?}"
        );
        assert_eq!(crate::helpers::visible_width(last), inner, "{last:?}");
    }

    #[test]
    fn version_wraps_to_own_row_when_last_line_is_full() {
        let version = help_version_label();
        let mut lines = vec!["x".repeat(20)];
        attach_help_version(&mut lines, 20);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[1].ends_with(&version), "{lines:?}");
        assert_eq!(crate::helpers::visible_width(&lines[1]), 20);
        assert!(!lines[0].contains(&version), "must not crowd the left copy");
    }
}
