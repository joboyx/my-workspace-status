//! Help overlay entries and `/` search (highlight only).

/// One help row: key chips plus a short description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    pub keys: &'static str,
    pub desc: &'static str,
}

/// Short key list shown in the `?` overlay.
pub const HELP_ENTRIES: &[HelpEntry] = &[
    HelpEntry { keys: "q", desc: "quit" },
    HelpEntry { keys: "?", desc: "close this help" },
    HelpEntry { keys: "j/k", desc: "move" },
    HelpEntry { keys: "arrows", desc: "same" },
    HelpEntry { keys: "z", desc: "fold this row" },
    HelpEntry { keys: "zz", desc: "fold subtree" },
    HelpEntry { keys: "h/l", desc: "fold tree / pan diff" },
    HelpEntry { keys: "t", desc: "tree / flat" },
    HelpEntry { keys: ".", desc: "show ignored" },
    HelpEntry { keys: "space", desc: "mark reviewed" },
    HelpEntry { keys: "/", desc: "pane or help search" },
    HelpEntry { keys: "n/N", desc: "next / prev" },
    HelpEntry { keys: "Ctrl+u/d", desc: "list ±5" },
    HelpEntry { keys: "PgUp/PgDn", desc: "page" },
    HelpEntry { keys: "Ctrl+O", desc: "full-file context" },
    HelpEntry { keys: "s", desc: "stage" },
    HelpEntry { keys: "u", desc: "unstage" },
    HelpEntry { keys: "x", desc: "revert (y/n)" },
    HelpEntry { keys: "e", desc: "edit" },
    HelpEntry { keys: "f", desc: "fetch" },
    HelpEntry { keys: "p", desc: "pull behind" },
    HelpEntry { keys: "d", desc: "default branch" },
    HelpEntry { keys: "r", desc: "refresh" },
    HelpEntry { keys: "P", desc: "push" },
    HelpEntry { keys: "S", desc: "stash menu" },
    HelpEntry { keys: "b", desc: "branch picker" },
    HelpEntry { keys: "C", desc: "create (in picker)" },
    HelpEntry { keys: "W", desc: "remove worktree" },
    HelpEntry { keys: "Tab", desc: "other pane" },
    HelpEntry { keys: "Enter", desc: "drill" },
    HelpEntry { keys: "Esc", desc: "back" },
    HelpEntry { keys: "a/p/D", desc: "focused stash" },
    HelpEntry { keys: "m", desc: "toggle mouse" },
    HelpEntry { keys: "click", desc: "select row" },
    HelpEntry { keys: "dbl-click", desc: "enter / drill" },
    HelpEntry { keys: "i", desc: "inline / split" },
    HelpEntry { keys: "drag", desc: "resize split" },
    HelpEntry { keys: ";", desc: "EasyMotion" },
    HelpEntry { keys: "T", desc: "cycle theme" },
];

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

/// Indices of `HELP_ENTRIES` that match `query`, in order.
pub fn help_match_indices(query: &str) -> Vec<usize> {
    HELP_ENTRIES
        .iter()
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
}
