//! Help overlay entries and `/` search (highlight only).
//!
//! Three columns match `HELP_GROUPS` (MOVE / GIT / VIEW). Extra keys
//! (`q`, Tab, picker `C`, stash `a p D`, Home/End) stay in those groups.

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
                desc: "fold · pan when right+diff",
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
                keys: "Ctrl-Space ;",
                desc: "EasyMotion on focused list",
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
                keys: "e",
                desc: "open in editor",
            },
            HelpEntry {
                keys: "space",
                desc: "mark dirty file reviewed (eye)",
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
                keys: "C",
                desc: "create (in picker)",
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
                keys: "a p D",
                desc: "focused stash apply/pop/drop",
            },
        ],
    },
    HelpGroup {
        title: "VIEW",
        entries: &[
            HelpEntry {
                keys: "i",
                desc: "inline / split",
            },
            HelpEntry {
                keys: "t",
                desc: "flat / tree",
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
                keys: "PgUp PgDn",
                desc: "page focused pane",
            },
            HelpEntry {
                keys: "Ctrl-u Ctrl-d",
                desc: "page focused ±5",
            },
            HelpEntry {
                keys: "m",
                desc: "mouse · drag pane or split divider",
            },
            HelpEntry {
                keys: "Esc",
                desc: "back / unfocus · never quit",
            },
            HelpEntry {
                keys: "Enter dblclick",
                desc: "focus right / drill",
            },
            HelpEntry {
                keys: "?",
                desc: "this help",
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
                keys: "Ctrl-C Ctrl-C",
                desc: "quit (press twice)",
            },
        ],
    },
];

/// Idle help footer must mention overlay-local `/` search.
pub const HELP_IDLE_FOOTER_SNIPPET: &str = "/ search help";
/// Active help-search footer Esc hint.
pub const HELP_SEARCH_ESC_HINT: &str = "Esc clears search";

/// Flattened help rows in column order (MOVE, then GIT, then VIEW).
#[allow(dead_code)]
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

/// Indices of flattened help entries that match `query`, in order.
#[allow(dead_code)]
pub fn help_match_indices(query: &str) -> Vec<usize> {
    help_entries()
        .enumerate()
        .filter(|(_, e)| help_entry_matches(e.keys, e.desc, query))
        .map(|(i, _)| i)
        .collect()
}

/// Round border (2) plus horizontal padding (1 each side).
pub const HELP_CHROME_COLS: usize = 4;

/// Chip cluster width: fits `Ctrl-u Ctrl-d` plus ≥2 columns before the description.
pub const HELP_KEY_WIDTH: usize = 18;

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

/// Width of one of the three help columns at `term_width`.
pub fn help_column_width(term_width: usize) -> usize {
    (help_inner_width(term_width) / HELP_COLUMN_COUNT).max(1)
}

fn help_chip_used_width(keys: &str) -> usize {
    keys.split(' ')
        .filter(|chip| !chip.is_empty())
        .map(|chip| chip.chars().count() + 3)
        .sum()
}

/// Painted columns for a help key cluster (` chip ` plus trailing gap).
pub fn help_chip_pad_width(keys: &str) -> usize {
    help_chip_used_width(keys) + help_chip_gap_spaces(keys)
}

/// Trailing gap spaces after chips so the cluster occupies [`help_chip_pad_width`].
pub fn help_chip_gap_spaces(keys: &str) -> usize {
    1.max(HELP_KEY_WIDTH.saturating_sub(help_chip_used_width(keys)))
}

/// Chip pad vs description wrap width for a column.
pub fn help_desc_layout(column_width: usize, chip_pad: usize) -> HelpDescLayout {
    let col = column_width.max(1);
    let remaining = col.saturating_sub(chip_pad);
    if remaining >= 1 {
        HelpDescLayout {
            indent: chip_pad,
            width: remaining,
            desc_on_first_line: true,
        }
    } else {
        HelpDescLayout {
            indent: 0,
            width: col,
            desc_on_first_line: false,
        }
    }
}

/// Word-wrap `text` to `width` columns. Breaks overlong words. Never ellipsizes.
pub fn wrap_help_description(text: &str, width: usize) -> Vec<String> {
    let col = width.max(1);
    let words: Vec<&str> = if text.trim().is_empty() {
        Vec::new()
    } else {
        text.trim().split_whitespace().collect()
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

/// Visual lines for one help entry at `column_width`.
pub fn help_entry_visual_lines(
    description: &str,
    column_width: usize,
    keys: &str,
) -> Vec<HelpVisualLine> {
    let chip_pad = if keys.is_empty() {
        HELP_KEY_WIDTH
    } else {
        help_chip_pad_width(keys)
    };
    let layout = help_desc_layout(column_width, chip_pad);
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

/// Body rows after wrap: each aligned index uses the tallest of the three cells.
pub fn help_body_line_count(groups: &[HelpGroup], column_width: usize) -> usize {
    let row_count = groups
        .iter()
        .map(|group| group.entries.len())
        .max()
        .unwrap_or(0);
    let mut total = 0;
    for row in 0..row_count {
        let mut height = 1usize;
        for group in groups {
            if let Some(entry) = group.entries.get(row) {
                height =
                    height.max(help_entry_visual_lines(entry.desc, column_width, entry.keys).len());
            }
        }
        total += height;
    }
    total
}

/// Overlay rows: border (2) + title + wrapped body + footer.
pub fn help_overlay_row_count(body_rows: usize, footer_rows: usize) -> usize {
    2 + 1 + body_rows + footer_rows
}

/// Full overlay height for `groups` at `term_width` with a wrappable footer.
pub fn help_overlay_height(groups: &[HelpGroup], term_width: usize, footer: &str) -> usize {
    let body = help_body_line_count(groups, help_column_width(term_width));
    let footer_lines = wrap_help_footer(footer, help_inner_width(term_width))
        .len()
        .max(1);
    help_overlay_row_count(body, footer_lines)
}

/// Overlay rows reserved for `?` help at `term_cols`.
pub fn help_status_lines(term_cols: u16) -> u16 {
    help_overlay_height(
        HELP_GROUPS,
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
        assert!(keys.contains(&"C"));
        assert!(keys.contains(&"a p D"));
        assert!(keys.contains(&"Home End"));
        let git_keys: Vec<&str> = HELP_GROUPS[1].entries.iter().map(|e| e.keys).collect();
        assert!(git_keys.contains(&"m"));
        let move_keys: Vec<&str> = HELP_GROUPS[0].entries.iter().map(|e| e.keys).collect();
        let view_keys: Vec<&str> = HELP_GROUPS[2].entries.iter().map(|e| e.keys).collect();
        assert!(!move_keys.contains(&"PgUp PgDn"));
        assert!(!move_keys.contains(&"Ctrl-u Ctrl-d"));
        assert!(view_keys.contains(&"PgUp PgDn"));
        assert!(view_keys.contains(&"Ctrl-u Ctrl-d"));
        assert!(view_keys.contains(&"."));
        assert!(view_keys.contains(&"T"));
        assert!(view_keys.contains(&"Ctrl-o"));
        assert!(view_keys.contains(&"m"));
        assert!(view_keys.contains(&"Esc"));
        assert!(help_match_indices("quit")
            .iter()
            .any(|&i| { help_entries().nth(i).is_some_and(|e| e.keys == "q") }));
    }

    #[test]
    fn overlay_height_grows_when_columns_narrow() {
        let row_count = HELP_GROUPS
            .iter()
            .map(|group| group.entries.len())
            .max()
            .unwrap_or(0);
        let wide = help_status_lines(300);
        let mid = help_status_lines(128);
        let narrow = help_status_lines(80);
        assert_eq!(wide, (2 + 1 + row_count + 1) as u16);
        assert!(mid > wide, "128 cols still wraps some descriptions");
        assert!(
            narrow > mid,
            "narrow terminals wrap more and take more rows"
        );
        assert!(HELP_KEY_WIDTH >= 18);
    }
}
