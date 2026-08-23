//! Help overlay entries and `/` search (highlight only).
//!
//! Three columns match Ink `HELP_GROUPS` (MOVE / GIT / VIEW). Rust extras
//! (`q`, Tab, picker `C`, stash `a/p/D`, mouse) stay in those groups.

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
                keys: "j/k",
                desc: "move",
            },
            HelpEntry {
                keys: "arrows",
                desc: "same",
            },
            HelpEntry {
                keys: "z",
                desc: "fold this row",
            },
            HelpEntry {
                keys: "zz",
                desc: "fold subtree",
            },
            HelpEntry {
                keys: "h/l",
                desc: "fold tree / pan diff",
            },
            HelpEntry {
                keys: "/",
                desc: "pane or help search",
            },
            HelpEntry {
                keys: "n/N",
                desc: "next / prev",
            },
            HelpEntry {
                keys: ";",
                desc: "EasyMotion",
            },
        ],
    },
    HelpGroup {
        title: "GIT",
        entries: &[
            HelpEntry {
                keys: "s",
                desc: "stage",
            },
            HelpEntry {
                keys: "S",
                desc: "stash menu",
            },
            HelpEntry {
                keys: "u",
                desc: "unstage",
            },
            HelpEntry {
                keys: "x",
                desc: "revert (y/n)",
            },
            HelpEntry {
                keys: "e",
                desc: "edit",
            },
            HelpEntry {
                keys: "space",
                desc: "mark reviewed",
            },
            HelpEntry {
                keys: "f",
                desc: "fetch",
            },
            HelpEntry {
                keys: "p",
                desc: "pull behind",
            },
            HelpEntry {
                keys: "P",
                desc: "push",
            },
            HelpEntry {
                keys: "d",
                desc: "default branch",
            },
            HelpEntry {
                keys: "b",
                desc: "branch picker",
            },
            HelpEntry {
                keys: "C",
                desc: "create (in picker)",
            },
            HelpEntry {
                keys: "W",
                desc: "remove worktree",
            },
            HelpEntry {
                keys: "r",
                desc: "refresh",
            },
            HelpEntry {
                keys: "a/p/D",
                desc: "focused stash",
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
                desc: "tree / flat",
            },
            HelpEntry {
                keys: ".",
                desc: "show ignored",
            },
            HelpEntry {
                keys: "T",
                desc: "cycle theme",
            },
            HelpEntry {
                keys: "Ctrl+O",
                desc: "full-file context",
            },
            HelpEntry {
                keys: "PgUp/PgDn",
                desc: "page",
            },
            HelpEntry {
                keys: "Ctrl+u/d",
                desc: "list ±5",
            },
            HelpEntry {
                keys: "m",
                desc: "toggle mouse",
            },
            HelpEntry {
                keys: "click",
                desc: "select row",
            },
            HelpEntry {
                keys: "dbl-click",
                desc: "enter / drill",
            },
            HelpEntry {
                keys: "drag",
                desc: "resize split",
            },
            HelpEntry {
                keys: "Esc",
                desc: "back",
            },
            HelpEntry {
                keys: "Enter",
                desc: "drill",
            },
            HelpEntry {
                keys: "?",
                desc: "close this help",
            },
            HelpEntry {
                keys: "Tab",
                desc: "other pane",
            },
            HelpEntry {
                keys: "q",
                desc: "quit",
            },
        ],
    },
];

/// Flattened help rows in column order (MOVE, then GIT, then VIEW).
///
/// `n` / `N` and `/` search use this order.
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
pub fn help_match_indices(query: &str) -> Vec<usize> {
    help_entries()
        .enumerate()
        .filter(|(_, e)| help_entry_matches(e.keys, e.desc, query))
        .map(|(i, _)| i)
        .collect()
}

/// Next/prev match index with wrap. Empty `hits` → `None`.
pub fn step_help_match(hits: &[usize], current: Option<usize>, dir: i32) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    let pos = current.and_then(|cur| hits.iter().position(|&h| h == cur));
    let Some(pos) = pos else {
        return if dir < 0 {
            hits.last().copied()
        } else {
            hits.first().copied()
        };
    };
    let len = hits.len() as i32;
    let next = (pos as i32 + dir).rem_euclid(len) as usize;
    hits.get(next).copied()
}

/// Flat index for `groups[group_i].entries[row]`.
pub fn help_flat_index(group_i: usize, row: usize) -> Option<usize> {
    let group = HELP_GROUPS.get(group_i)?;
    if row >= group.entries.len() {
        return None;
    }
    let before: usize = HELP_GROUPS
        .iter()
        .take(group_i)
        .map(|g| g.entries.len())
        .sum();
    Some(before + row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive_keys_or_desc() {
        assert!(help_entry_matches("Ctrl+O", "full-file context", "ctrl"));
        assert!(help_entry_matches("Ctrl+O", "full-file context", "FILE"));
        assert!(!help_entry_matches("Ctrl+O", "full-file context", "zzz"));
    }

    #[test]
    fn empty_query_is_not_a_match() {
        assert!(!help_entry_matches("j/k", "move", ""));
        assert!(!help_entry_matches("j/k", "move", "   "));
    }

    #[test]
    fn step_wraps_help_hits() {
        let hits = vec![2, 5, 9];
        assert_eq!(step_help_match(&hits, None, 1), Some(2));
        assert_eq!(step_help_match(&hits, Some(2), 1), Some(5));
        assert_eq!(step_help_match(&hits, Some(9), 1), Some(2));
        assert_eq!(step_help_match(&hits, Some(2), -1), Some(9));
        assert_eq!(step_help_match(&[], None, 1), None);
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
        assert!(keys.contains(&"a/p/D"));
        let move_keys: Vec<&str> = HELP_GROUPS[0].entries.iter().map(|e| e.keys).collect();
        let view_keys: Vec<&str> = HELP_GROUPS[2].entries.iter().map(|e| e.keys).collect();
        assert!(!move_keys.contains(&"PgUp/PgDn"));
        assert!(!move_keys.contains(&"Ctrl+u/d"));
        assert!(view_keys.contains(&"PgUp/PgDn"));
        assert!(view_keys.contains(&"Ctrl+u/d"));
        assert!(view_keys.contains(&"."));
        assert!(view_keys.contains(&"T"));
        assert!(view_keys.contains(&"Ctrl+O"));
        assert!(view_keys.contains(&"m"));
        assert!(view_keys.contains(&"Esc"));
        assert!(help_match_indices("quit")
            .iter()
            .any(|&i| { help_entries().nth(i).is_some_and(|e| e.keys == "q") }));
    }
}
