//! Help overlay entries and `/` search (highlight only).
//!
//! Three columns match Ink `HELP_GROUPS` (MOVE / GIT / VIEW). Rust extras
//! (`q`, Tab, picker `C`, stash `a p D`, Home/End) stay in those groups.

/// One help row: key chips plus a short description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    pub keys: &'static str,
    pub desc: &'static str,
}

/// One help column (Ink MOVE / GIT / VIEW).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpGroup {
    pub title: &'static str,
    pub entries: &'static [HelpEntry],
}

/// Help overlay stays on three groups.
pub const HELP_COLUMN_COUNT: usize = 3;

/// Short key list shown in the `?` overlay, grouped like Ink.
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
/// Active help-search footer Esc hint (Ink `HELP_SEARCH_ESC_HINT`).
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive_keys_or_desc() {
        assert!(help_entry_matches("Ctrl-o", "full-file · keep hunk in view", "ctrl"));
        assert!(help_entry_matches("Ctrl-o", "full-file · keep hunk in view", "FILE"));
        assert!(!help_entry_matches("Ctrl-o", "full-file · keep hunk in view", "zzz"));
    }

    #[test]
    fn empty_query_is_not_a_match() {
        assert!(!help_entry_matches("j k", "down / up", ""));
        assert!(!help_entry_matches("j k", "down / up", "   "));
    }

    #[test]
    fn groups_are_move_git_view_with_rust_extras() {
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
}
