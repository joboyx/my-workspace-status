//! Tree shape under a focus query: the focused families, the dimmed
//! outside-focus group, and the match highlight of tree rows.
//!
//! The split itself (which families and checkouts a query keeps, and the
//! counts) is [`crate::tui::tree_focus::split_by_focus`]; this module turns
//! it into tree nodes with the family builders of [`super`].

use std::collections::BTreeMap;

use crate::snapshot::{CheckoutKind, WorkspaceRepoSnapshot, WorkspaceSnapshot};
use crate::tui::tree_focus::{match_ranges, split_by_focus, TreeFocus};

use super::{
    family_container, family_needs_attention, family_nodes, sort_family_members,
    with_no_updates_group, FocusChip, NodeChrome, NodeKind, SegRole, TextSeg, TreeNode,
};

/// Row id of the group that holds the families outside the focus
/// (`N outside focus · M dirty`). It is the last child of the workspace
/// root, starts folded, and its rows paint muted.
pub const OUTSIDE_FOCUS_GROUP_ID: &str = "group:outside-focus";

/// Root children under `focus` and the workspace chip.
///
/// Focused families keep their kept checkouts only and sort as the plain
/// tree does: attention families, then `No updates`. A family that is a
/// container in the plain tree (a primary plus linked checkouts) stays a
/// container even when the focus keeps one checkout, and shows `k/n wt`
/// when it keeps fewer than all. Outside families keep every checkout and
/// go, attention first then clean, under [`OUTSIDE_FOCUS_GROUP_ID`].
pub(super) fn focused_children(
    focus: &TreeFocus,
    snapshot: &WorkspaceSnapshot,
    families: BTreeMap<String, Vec<&WorkspaceRepoSnapshot>>,
    tree_mode: bool,
) -> (Vec<TreeNode>, FocusChip) {
    let split = split_by_focus(focus, snapshot);
    let mut focused = Vec::new();
    let mut outside_attention = Vec::new();
    let mut outside_clean = Vec::new();
    for (primary, members) in families {
        let Some(family) = split.focused.iter().find(|f| f.primary_path == primary) else {
            let needs = family_needs_attention(&members);
            let nodes = family_nodes(&primary, members, tree_mode);
            if needs {
                outside_attention.extend(nodes);
            } else {
                outside_clean.extend(nodes);
            }
            continue;
        };
        let is_container = members
            .iter()
            .any(|m| m.checkout_kind == CheckoutKind::Linked)
            && members
                .iter()
                .any(|m| m.checkout_kind == CheckoutKind::Primary);
        let mut kept: Vec<&WorkspaceRepoSnapshot> = members
            .into_iter()
            .filter(|m| family.keeps(&m.repo))
            .collect();
        let needs = family_needs_attention(&kept);
        let nodes = if is_container && family.is_partial() {
            sort_family_members(&mut kept);
            let mut container = family_container(&primary, kept, tree_mode);
            container.chrome.checkout_total = family.checkout_count;
            vec![container]
        } else {
            family_nodes(&primary, kept, tree_mode)
        };
        focused.push((needs, nodes));
    }
    let mut children = with_no_updates_group(focused);
    let counts = split.counts;
    if counts.outside > 0 {
        let mut outside = outside_attention;
        outside.extend(outside_clean);
        children.push(TreeNode {
            id: OUTSIDE_FOCUS_GROUP_ID.into(),
            kind: NodeKind::Group,
            label: format!(
                "{} outside focus · {} dirty",
                counts.outside, counts.outside_dirty
            ),
            repo: None,
            primary_repo: None,
            hidden: false,
            file: None,
            children: outside,
            chrome: NodeChrome::default(),
        });
    }
    let chip = FocusChip {
        query: focus.query().trim().to_string(),
        focused: counts.focused,
        total: counts.total,
    };
    (children, chip)
}

/// Split the text `focus` matches in the repo and branch names of a tree
/// row out of its label `segments` as [`SegRole::FocusMatch`] segments,
/// for paint.
///
/// Only repo and checkout rows (`kind`) highlight, and only their name and
/// branch segments ([`is_name_or_branch`]): the separator and the muted
/// ` · <primary>` of a linked checkout stay as they are. The split keeps
/// every column where it was, so icon spans and widths do not change.
/// Paint calls it after the force-push text fit, which reads the name as
/// one segment.
pub fn highlight_focus_matches(kind: NodeKind, segments: &mut Vec<TextSeg>, focus: &TreeFocus) {
    if !matches!(kind, NodeKind::Repo | NodeKind::Checkout) || focus.is_empty() {
        return;
    }
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        let ranges = if is_name_or_branch(&seg) {
            match_ranges(focus, &seg.text)
        } else {
            Vec::new()
        };
        if ranges.is_empty() {
            out.push(seg);
            continue;
        }
        let piece = |text: &str, matched: bool| TextSeg {
            text: text.to_string(),
            role: if matched {
                SegRole::FocusMatch
            } else {
                seg.role
            },
            bold: seg.bold || matched,
            dim: seg.dim && !matched,
            hex: if matched { None } else { seg.hex },
            icon: None,
        };
        let mut at = 0;
        for range in ranges {
            if range.start > at {
                out.push(piece(&seg.text[at..range.start], false));
            }
            out.push(piece(&seg.text[range.clone()], true));
            at = range.end;
        }
        if at < seg.text.len() {
            out.push(piece(&seg.text[at..], false));
        }
    }
    *segments = out;
}

/// True for the repo name and branch segments of a repo or checkout row
/// label: text with no icon tag that is either bold (the repo path, the
/// linked short name, a checkout's branch or name) or in a branch role (a
/// repo row's branch).
fn is_name_or_branch(seg: &TextSeg) -> bool {
    seg.icon.is_none()
        && (seg.bold || matches!(seg.role, SegRole::BranchFeature | SegRole::BranchDefault))
}

#[cfg(test)]
mod tests;
