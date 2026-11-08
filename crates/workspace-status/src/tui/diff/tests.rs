use super::*;
use crate::git::exec_git;

const FIXTURE: &str = "\
diff --git a/hello.ts b/hello.ts
index 1111111..2222222 100644
--- a/hello.ts
+++ b/hello.ts
@@ -10,3 +10,4 @@
 line1
-line2
+line2 changed
 line3
+line4
";

#[test]
fn parse_seeds_counters_from_hunk_header() {
    let hunks = parse_unified_diff(FIXTURE);
    assert_eq!(hunks.len(), 1);
    let kinds: Vec<_> = hunks[0]
        .lines
        .iter()
        .map(|l| (l.kind, l.old_no, l.new_no))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (DiffCellKind::Ctx, Some(10), Some(10)),
            (DiffCellKind::Del, Some(11), None),
            (DiffCellKind::Add, None, Some(11)),
            (DiffCellKind::Ctx, Some(12), Some(12)),
            (DiffCellKind::Add, None, Some(13)),
        ]
    );
}

#[test]
fn parse_tolerates_garbage_hunk_header() {
    let hunks = parse_unified_diff("@@ garbage @@\n+a\n");
    assert_eq!(hunks[0].lines[0].kind, DiffCellKind::Add);
    assert_eq!(hunks[0].lines[0].new_no, Some(0));
}

fn line_ref(
    section: DiffSection,
    kind: DiffCellKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
) -> Option<RowLineRef> {
    Some(RowLineRef {
        section,
        kind,
        old_no,
        new_no,
    })
}

#[test]
fn row_line_ref_inline_maps_each_line_and_skips_headers() {
    let content = DiffContent::from_unified(FIXTURE);
    let at = |row| row_line_ref(&content, DiffMode::Inline, row);
    let s = DiffSection::Unstaged;
    assert_eq!(at(0), None, "section label");
    assert_eq!(at(1), None, "hunk header");
    assert_eq!(at(2), line_ref(s, DiffCellKind::Ctx, Some(10), Some(10)));
    assert_eq!(at(3), line_ref(s, DiffCellKind::Del, Some(11), None));
    assert_eq!(at(4), line_ref(s, DiffCellKind::Add, None, Some(11)));
    assert_eq!(at(6), line_ref(s, DiffCellKind::Add, None, Some(13)));
    assert_eq!(at(7), None, "past the end");
}

#[test]
fn row_line_ref_split_prefers_the_new_side_then_the_deleted_line() {
    let content = DiffContent::from_unified("@@ -1,2 +1,1 @@\n-a\n-b\n+c\n");
    let at = |row| row_line_ref(&content, DiffMode::SideBySide, row);
    let s = DiffSection::Unstaged;
    // Row 2 pairs `-a` with `+c`; row 3 is `-b` beside an empty cell.
    assert_eq!(at(2), line_ref(s, DiffCellKind::Add, None, Some(1)));
    assert_eq!(at(3), line_ref(s, DiffCellKind::Del, Some(2), None));
    let fixture = DiffContent::from_unified(FIXTURE);
    assert_eq!(
        row_line_ref(&fixture, DiffMode::SideBySide, 2),
        line_ref(s, DiffCellKind::Ctx, Some(10), Some(10))
    );
}

/// FIXTURE rows: 2 ctx 10, 3 del 11, 4 add 11, 5 ctx 12, 6 add 13.
#[test]
fn row_for_line_matches_the_new_side_else_the_nearest_row() {
    let content = DiffContent::from_unified(FIXTURE);
    let at = |line| row_for_line(&content, DiffMode::Inline, line);
    assert_eq!(at(10), Some(2));
    assert_eq!(at(11), Some(4), "the added line, not the deleted old 11");
    assert_eq!(at(13), Some(6));
    assert_eq!(at(1), Some(2), "before the hunk: its first row");
    assert_eq!(at(9999), Some(6), "past the end: the last numbered row");
    let two_hunks =
        DiffContent::from_unified("@@ -1,2 +1,2 @@\n a\n-b\n+B\n@@ -20,2 +20,2 @@\n y\n-z\n+Z\n");
    let at = |line| row_for_line(&two_hunks, DiffMode::Inline, line);
    // Rows: 0 label, 1 hunk, 2 a(1), 3 -b, 4 +B(2), 5 hunk, 6 y(20), 7 -z, 8 +Z(21).
    assert_eq!(at(5), Some(4), "between hunks: nearer the first");
    assert_eq!(at(18), Some(6), "between hunks: nearer the second");
    assert_eq!(at(11), Some(4), "a tie takes the earlier row");
    let split = |line| row_for_line(&content, DiffMode::SideBySide, line);
    assert_eq!(
        row_line_ref(&content, DiffMode::SideBySide, split(11).unwrap())
            .and_then(|line_ref| line_ref.new_no),
        Some(11)
    );
}

#[test]
fn row_for_line_counts_deleted_rows_only_without_a_new_side() {
    let deleted = DiffContent::from_unified("@@ -1,3 +0,0 @@\n-a\n-b\n-c\n");
    let at = |line| row_for_line(&deleted, DiffMode::Inline, line);
    assert_eq!(at(2), Some(3), "old line 2");
    assert_eq!(at(50), Some(4));
    let binary = DiffContent::from_unified("Binary files a/x and b/x differ\n");
    assert_eq!(row_for_line(&binary, DiffMode::Inline, 1), None);
    assert_eq!(
        row_for_line(&DiffContent::default(), DiffMode::Inline, 1),
        None
    );
}

#[test]
fn row_line_ref_tells_staged_from_unstaged() {
    let content = DiffContent {
        staged: FIXTURE.into(),
        unstaged: "@@ -5 +5 @@\n-old\n+new\n".into(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let at = |row| row_line_ref(&content, DiffMode::Inline, row);
    assert_eq!(
        at(3),
        line_ref(DiffSection::Staged, DiffCellKind::Del, Some(11), None)
    );
    // Staged rows 0..=6, then the UNSTAGED label (7) and hunk (8).
    assert_eq!(at(7), None);
    assert_eq!(
        at(10),
        line_ref(DiffSection::Unstaged, DiffCellKind::Add, None, Some(5))
    );
    let committed = DiffContent::from_compare_lines(vec!["@@ -1 +1 @@".into(), "+x".into()]);
    assert_eq!(
        row_line_ref(&committed, DiffMode::Inline, 2),
        line_ref(DiffSection::Committed, DiffCellKind::Add, None, Some(1))
    );
}

#[test]
fn row_for_new_line_prefers_the_unstaged_section() {
    let content = DiffContent {
        staged: "@@ -1,2 +1,2 @@\n a\n-b\n+B\n".into(),
        unstaged: "@@ -1,2 +1,3 @@\n a\n+x\n B\n".into(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    // STAGED 0, hunk 1, a 2, -b 3, +B 4; UNSTAGED 5, hunk 6, a 7, +x 8,
    // B 9.
    let row = |line| row_for_new_line(&content, DiffMode::Inline, line);
    assert_eq!(row(1), Some(7), "unstaged first");
    assert_eq!(row(2), Some(8), "an added line");
    assert_eq!(row(3), Some(9));
    assert_eq!(row(9), None);
    let staged_only = DiffContent {
        unstaged: String::new(),
        ..content
    };
    assert_eq!(
        row_for_new_line(&staged_only, DiffMode::Inline, 2),
        Some(4),
        "staged when the unstaged section has no such line"
    );
    assert_eq!(
        row_for_new_line(&staged_only, DiffMode::SideBySide, 2),
        Some(3),
        "a split row pairs -b with +B"
    );
}

#[test]
fn worktree_compare_content_is_its_own_section_and_never_a_patch() {
    let content =
        DiffContent::from_worktree_compare_lines(FIXTURE.lines().map(String::from).collect());
    let rows = build_diff_rows(&content, DiffMode::Inline);
    assert_eq!(rows[0], DiffRow::Section(DiffSection::Worktree));
    assert_eq!(section_header(DiffSection::Worktree), "WORKING TREE");
    assert_eq!(
        row_line_ref(&content, DiffMode::Inline, 4),
        line_ref(DiffSection::Worktree, DiffCellKind::Add, None, Some(11))
    );
    for kind in [
        PartialPatchKind::Stage,
        PartialPatchKind::Unstage,
        PartialPatchKind::Revert,
        PartialPatchKind::RevertCommitted,
    ] {
        assert!(
            build_partial_patch(&content, DiffMode::Inline, 2, 4, kind, "f.rs").is_err(),
            "{kind:?}"
        );
    }
}

#[test]
fn row_line_ref_is_none_for_binary_meta_and_error_rows() {
    let binary = DiffContent::from_unified("Binary files a/x and b/x differ\n");
    for mode in [DiffMode::Inline, DiffMode::SideBySide] {
        assert_eq!(build_diff_rows(&binary, mode).len(), 2);
        assert_eq!(row_line_ref(&binary, mode, 1), None);
    }
    let no_eol = DiffContent::from_unified("@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n");
    assert_eq!(row_line_ref(&no_eol, DiffMode::Inline, 4), None);
    let failed = DiffContent {
        error: Some("fatal: bad object".into()),
        ..DiffContent::from_unified(FIXTURE)
    };
    let last = build_diff_rows(&failed, DiffMode::Inline).len() - 1;
    assert_eq!(row_line_ref(&failed, DiffMode::Inline, last), None);
}

/// A staged diff that loaded beside a failed unstaged `git diff` ends
/// with the failure line; with no rows the pane's empty text says it.
#[test]
fn partial_diff_failure_appends_an_error_row() {
    let content = DiffContent {
        staged: FIXTURE.into(),
        unstaged: String::new(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: Some("fatal: bad object".into()),
    };
    let rows = build_diff_rows(&content, DiffMode::Inline);
    assert_eq!(rows[0], DiffRow::Section(DiffSection::Staged));
    let last = rows.last().expect("rows");
    assert_eq!(
        *last,
        DiffRow::Error {
            text: "git diff failed: fatal: bad object".into()
        }
    );
    assert_eq!(row_search_text(last), "git diff failed: fatal: bad object");
    let error_row = rows.len() - 1;
    assert_eq!(
        build_partial_patch(
            &content,
            DiffMode::Inline,
            error_row,
            error_row,
            PartialPatchKind::Unstage,
            "README.md",
        ),
        Err("nothing to unstage in highlight".into())
    );
    let failed_only = DiffContent {
        error: Some("fatal: bad object".into()),
        ..DiffContent::default()
    };
    assert!(build_diff_rows(&failed_only, DiffMode::Inline).is_empty());
}

#[test]
fn build_diff_rows_emits_section_per_nonempty_slot() {
    let staged = build_diff_rows(
        &DiffContent {
            staged: FIXTURE.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        },
        DiffMode::Inline,
    );
    assert_eq!(staged[0], DiffRow::Section(DiffSection::Staged));
    assert!(!staged
        .iter()
        .any(|r| matches!(r, DiffRow::Section(DiffSection::Unstaged))));

    let both = build_diff_rows(
        &DiffContent {
            staged: FIXTURE.into(),
            unstaged: FIXTURE.into(),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        },
        DiffMode::Inline,
    );
    let sections: Vec<_> = both
        .iter()
        .filter_map(|r| match r {
            DiffRow::Section(s) => Some(*s),
            _ => None,
        })
        .collect();
    assert_eq!(sections, vec![DiffSection::Staged, DiffSection::Unstaged]);
    assert!(build_diff_rows(&DiffContent::default(), DiffMode::Inline).is_empty());
}

#[test]
fn huge_untracked_text_says_too_large_and_binary_keeps_its_stub() {
    let dir = crate::testutil::unique_dir("ws-huge-untracked");
    std::fs::create_dir_all(&dir).unwrap();
    let text = "x".repeat(1_200_000);
    std::fs::write(dir.join("big.txt"), &text).unwrap();
    let mut binary = vec![b'a'; 1_200_000];
    binary[10] = 0;
    std::fs::write(dir.join("big.bin"), &binary).unwrap();

    let big_text = untracked_content(&dir, "big.txt");
    assert_eq!(big_text.unstaged, "too large to preview (1.2 MB)\n");
    let rows = build_diff_rows(&big_text, DiffMode::Inline);
    assert!(
        rows.iter().any(|row| matches!(
            row,
            DiffRow::Line { left, .. }
                if left.kind == DiffCellKind::Meta && left.text.contains("too large")
        )),
        "{rows:?}"
    );
    let big_bin = untracked_content(&dir, "big.bin");
    assert!(big_bin.unstaged.starts_with("Binary files"), "{big_bin:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_git_diff_keeps_the_reason() {
    let root = crate::testutil::unique_dir("ws-diff-fails");
    let repo = root.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    crate::testutil::init_repo(&repo);
    std::fs::write(repo.join("README.md"), "# changed\n").unwrap();
    std::fs::write(repo.join(".git").join("index"), b"not an index").unwrap();
    let change = FileChange {
        path: "README.md".into(),
        staged_status: None,
        unstaged_status: Some("M".into()),
        untracked: false,
        old_path: None,
    };
    let content = load_file_diff(&root, "app", &change, None);
    assert!(content.unstaged.is_empty(), "{content:?}");
    let reason = content.error.expect("git diff failure reason");
    assert!(!reason.is_empty() && !reason.contains('\n'), "{reason}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn untracked_section_is_new() {
    let rows = build_diff_rows(
        &DiffContent {
            staged: String::new(),
            unstaged: FIXTURE.into(),
            is_new: true,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        },
        DiffMode::Inline,
    );
    assert_eq!(rows[0], DiffRow::Section(DiffSection::New));
}

#[test]
fn inline_mode_one_cell_per_line() {
    let rows = build_diff_rows(
        &DiffContent {
            staged: FIXTURE.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        },
        DiffMode::Inline,
    );
    let lines: Vec<_> = rows
        .iter()
        .filter(|r| matches!(r, DiffRow::Line { .. }))
        .collect();
    assert_eq!(lines.len(), 5);
    assert!(lines
        .iter()
        .all(|r| matches!(r, DiffRow::Line { right: None, .. })));
}

/// Diff rows hold paintable text: tabs expand to 4-column stops and every
/// other control character (ESC, BEL, C1) is U+FFFD, in both layouts and
/// in the hunk header. The parsed lines that patches read keep the bytes.
#[test]
fn diff_rows_expand_tabs_and_replace_control_characters() {
    let content = DiffContent::from_unified(
        "@@ -1,2 +1,2 @@ fn\x1b[2J\tx\n-a\tb\x07\n+ab\t\x1b[31mred\u{9b}\n \tctx\n",
    );
    for mode in [DiffMode::Inline, DiffMode::SideBySide] {
        let rows = build_diff_rows(&content, mode);
        let texts: Vec<String> = rows
            .iter()
            .flat_map(|row| match row {
                DiffRow::Hunk { text } => vec![text.clone()],
                DiffRow::Line { left, right } => std::iter::once(left.text.clone())
                    .chain(right.iter().map(|cell| cell.text.clone()))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        for text in &texts {
            assert!(!text.contains(char::is_control), "{mode:?}: {text:?}");
        }
        assert!(texts.contains(&"@@ -1,2 +1,2 @@ fn\u{FFFD}[2J  x".to_string()));
        assert!(texts.contains(&"a   b\u{FFFD}".to_string()), "{texts:?}");
        assert!(texts.contains(&"ab  \u{FFFD}[31mred\u{FFFD}".to_string()));
        assert!(texts.contains(&"    ctx".to_string()));
    }
    let hunks = parse_unified_diff(&content.unstaged);
    assert_eq!(hunks[0].lines[0].text, "a\tb\x07");
}

#[test]
fn side_by_side_pairs_del_with_add() {
    let rows = build_diff_rows(
        &DiffContent {
            staged: FIXTURE.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        },
        DiffMode::SideBySide,
    );
    let changed = rows.iter().find_map(|r| match r {
        DiffRow::Line { left, right } if left.text == "line2" => Some((left, right)),
        _ => None,
    });
    let (left, right) = changed.expect("del row");
    assert_eq!(left.kind, DiffCellKind::Del);
    assert_eq!(right.as_ref().map(|c| c.kind), Some(DiffCellKind::Add));
    assert_eq!(
        right.as_ref().map(|c| c.text.as_str()),
        Some("line2 changed")
    );

    let added = rows.iter().find_map(|r| match r {
        DiffRow::Line { left, right }
            if right.as_ref().map(|c| c.text.as_str()) == Some("line4") =>
        {
            Some(left.kind)
        }
        _ => None,
    });
    assert_eq!(added, Some(DiffCellKind::Empty));
}

#[test]
fn side_by_side_zips_del_add_runs() {
    let rows = build_diff_rows(
        &DiffContent::from_unified("@@ -1,2 +1,2 @@\n-a\n-b\n+a2\n+b2\n"),
        DiffMode::SideBySide,
    );
    let pairs: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            DiffRow::Line { left, right } => {
                Some((left.text.as_str(), right.as_ref().map(|c| c.text.as_str())))
            }
            _ => None,
        })
        .collect();
    assert_eq!(pairs, vec![("a", Some("a2")), ("b", Some("b2"))]);
}

#[test]
fn binary_marker_is_meta_cell() {
    let rows = build_diff_rows(
        &DiffContent::from_unified("Binary files a/x and b/x differ\n"),
        DiffMode::Inline,
    );
    let DiffRow::Line { left, .. } = &rows
        .iter()
        .find(|r| matches!(r, DiffRow::Line { .. }))
        .unwrap()
    else {
        panic!("expected line");
    };
    assert_eq!(left.kind, DiffCellKind::Meta);
    assert!(left.text.contains("Binary files"));
}

#[test]
fn gutter_width_floors_at_two_and_grows() {
    assert_eq!(gutter_width(&[]), 2);
    let small = build_diff_rows(
        &DiffContent {
            staged: FIXTURE.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        },
        DiffMode::Inline,
    );
    assert_eq!(gutter_width(&small), 2);
    let big = build_diff_rows(
        &DiffContent::from_unified("@@ -1200,1 +1200,1 @@\n-a\n+b\n"),
        DiffMode::Inline,
    );
    assert_eq!(gutter_width(&big), 4);
}

#[test]
fn header_rows_fit_on_one_row_for_a_short_path() {
    assert_eq!(diff_pane_header_rows("app/README.md", 40, 20), 1);
    assert_eq!(
        diff_pane_header_rows("", 40, 20),
        1,
        "empty path shows `Diff`"
    );
    assert_eq!(diff_pane_header_rows("abcd", 4, 20), 1, "exact fit");
}

#[test]
fn header_rows_wrap_a_long_path_by_display_columns() {
    assert_eq!(diff_pane_header_rows("abcde", 4, 20), 2);
    assert_eq!(diff_pane_header_rows(&"x".repeat(25), 10, 20), 3);
    // Four wide glyphs are eight columns: two rows at width 4, not one.
    assert_eq!(diff_pane_header_rows("日本語字", 4, 20), 2);
    assert_eq!(
        diff_pane_header_rows("日本語字", 5, 20),
        2,
        "a wide glyph never splits"
    );
}

#[test]
fn header_rows_cap_at_half_the_pane_height() {
    let path = "x".repeat(100);
    assert_eq!(diff_pane_header_rows(&path, 10, 12), 6);
    assert_eq!(diff_pane_header_rows(&path, 10, 7), 3, "floor of half");
    assert_eq!(diff_pane_header_rows(&path, 10, 3), 1);
}

#[test]
fn header_rows_are_one_on_a_tiny_or_zero_width_pane() {
    let path = "x".repeat(100);
    assert_eq!(diff_pane_header_rows(&path, 10, 1), 1);
    assert_eq!(diff_pane_header_rows(&path, 10, 0), 1);
    assert_eq!(diff_pane_header_rows(&path, 0, 20), 1);
}

#[test]
fn header_includes_path_mode_and_optional_full() {
    assert_eq!(
        diff_pane_header("app/README.md", "inline", false, false, 0, 0, 20, 5),
        "app/README.md  inline"
    );
    assert_eq!(
        diff_pane_header("src/lib.rs", "split", true, false, 3, 0, 10, 40),
        "src/lib.rs  split · full · pan 3  10/40"
    );
    assert_eq!(
        diff_pane_header("app/x.rs", "inline", false, true, 0, 0, 20, 5),
        "app/x.rs  inline · wrap"
    );
    assert_eq!(
        diff_pane_mode_label(DiffMode::SideBySide, DiffMode::Inline),
        "inline (too narrow)"
    );
}

#[test]
fn wrap_height_uses_display_columns_and_viewport_snaps() {
    let long = DiffRow::Line {
        left: DiffCell {
            kind: DiffCellKind::Add,
            text: "n".repeat(20),
            line_no: Some(1),
        },
        right: None,
    };
    let height = diff_row_visual_height(&long, 24, 3, false, 0.5);
    assert!(
        height >= 2,
        "20-col add must wrap in the code window: {height}"
    );
    let short = DiffRow::Line {
        left: DiffCell {
            kind: DiffCellKind::Add,
            text: "ok".into(),
            line_no: Some(1),
        },
        right: None,
    };
    assert_eq!(diff_row_visual_height(&short, 24, 3, false, 0.5), 1);
    assert_eq!(wrap_viewport_start(&[1, 1, 1, 1], 2, 2), 1);
    assert_eq!(wrap_viewport_start(&[3, 1, 1], 0, 2), 0);
    assert_eq!(wrap_viewport_start(&[], 0, 10), 0);
}

#[test]
fn blank_content_and_from_lines() {
    assert!(DiffContent::default().is_blank());
    assert!(!DiffContent::from_unified("@@ -1 +1 @@\n+a\n").is_blank());
    let from_vec = DiffContent::from_lines(vec!["@@ -1 +1 @@".into(), "+a".into()]);
    assert!(!from_vec.is_blank());
}

#[test]
fn syntax_fingerprint_changes_when_same_length_text_changes() {
    let alpha = DiffContent::from_lines(vec![
        "@@ -1,3 +1,4 @@".into(),
        " {".into(),
        r#"-  "ttlMs": 5000"#.into(),
        r#"+  "ttlMs": 2000"#.into(),
        r#"+  "name": "alpha-syntax""#.into(),
        " }".into(),
    ]);
    let omega = DiffContent::from_lines(vec![
        "@@ -1,3 +1,4 @@".into(),
        " {".into(),
        r#"-  "ttlMs": 5000"#.into(),
        r#"+  "ttlMs": 2000"#.into(),
        r#"+  "name": "omega-syntax""#.into(),
        " }".into(),
    ]);
    assert_eq!(alpha.unstaged.len(), omega.unstaged.len());
    assert_ne!(alpha.syntax_fingerprint(), omega.syntax_fingerprint());
    assert_eq!(
        alpha.syntax_fingerprint(),
        alpha.clone().syntax_fingerprint()
    );
}

#[test]
fn synthesize_empty_and_body() {
    assert_eq!(synthesize_all_add_diff(""), "@@ -0,0 +0,0 @@\n");
    assert_eq!(
        synthesize_all_add_diff("a\nb\n"),
        "@@ -0,0 +1,2 @@\n+a\n+b\n"
    );
    assert_eq!(
        synthesize_all_add_diff("a\r\nb\rc"),
        "@@ -0,0 +1,2 @@\n+a\r\n+b\rc\n\\ No newline at end of file\n"
    );
}

#[test]
fn anchor_prefers_visible_add_or_del() {
    let rows = build_diff_rows(
        &DiffContent::from_unified("@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n"),
        DiffMode::Inline,
    );
    // 0 section, 1 hunk, 2 ctx, 3 del, 4 add
    assert_eq!(anchor_row_index(&rows, 0, 10), 3);
    assert_eq!(scroll_to_keep_row(3, 9, rows.len()), 0);
}

#[test]
fn find_anchor_row_follows_change_text_not_old_index() {
    let short = build_diff_rows(
        &DiffContent::from_unified("@@ -46,3 +46,3 @@\n ctx\n-old-line\n+new-line\n"),
        DiffMode::Inline,
    );
    let needle = anchor_row_text(&short, 0, 10);
    let old_idx = anchor_row_index(&short, 0, 10);
    let mut body = String::from("@@ -1,20 +1,20 @@\n");
    for i in 0..15 {
        body.push_str(&format!(" pad-{i}\n"));
    }
    body.push_str("-old-line\n+new-line\n ctx\n");
    let long = build_diff_rows(&DiffContent::from_unified(body), DiffMode::Inline);
    let new_idx = find_anchor_row(&long, &needle);
    assert!(
        new_idx > old_idx,
        "full-file change must sit later than the hunk-only index ({old_idx} vs {new_idx})"
    );
    assert_eq!(row_search_text(&long[new_idx]), needle);
    assert!(
        scroll_to_keep_row(new_idx, 8, long.len()) > scroll_to_keep_row(old_idx, 8, long.len()),
        "stale hunk-only index would not keep the change in view"
    );
}

const TWO_HUNKS: &str = "\
diff --git a/regions.txt b/regions.txt
index 1111111..2222222 100644
--- a/regions.txt
+++ b/regions.txt
@@ -1,4 +1,4 @@
 keep-a
 keep-b
-ALPHA-OLD
+ALPHA-NEW
 keep-c
@@ -20,4 +20,4 @@
 keep-x
 keep-y
-OMEGA-OLD
+OMEGA-NEW
 keep-z
";

fn two_hunk_content() -> DiffContent {
    DiffContent {
        staged: String::new(),
        unstaged: TWO_HUNKS.into(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    }
}

fn first_hunk_row_span(mode: DiffMode) -> (usize, usize) {
    let rows = build_diff_rows(&two_hunk_content(), mode);
    let first = rows
        .iter()
        .position(|r| matches!(r, DiffRow::Hunk { .. }))
        .expect("first hunk");
    let second = rows
        .iter()
        .enumerate()
        .skip(first + 1)
        .find_map(|(i, r)| matches!(r, DiffRow::Hunk { .. }).then_some(i))
        .expect("second hunk");
    (first, second - 1)
}

#[test]
fn partial_patch_keeps_only_the_highlighted_hunk() {
    let content = two_hunk_content();
    let (start, end) = first_hunk_row_span(DiffMode::Inline);
    let patch = build_partial_patch(
        &content,
        DiffMode::Inline,
        start,
        end,
        PartialPatchKind::Stage,
        "regions.txt",
    )
    .expect("patch");
    assert!(
        patch.contains("ALPHA-NEW") && patch.contains("ALPHA-OLD"),
        "{patch}"
    );
    assert!(
        !patch.contains("OMEGA-NEW") && !patch.contains("OMEGA-OLD"),
        "{patch}"
    );
    assert!(
        patch.contains("diff --git a/regions.txt b/regions.txt"),
        "{patch}"
    );
}

#[test]
fn partial_patch_stages_one_add_line_inside_a_hunk() {
    let content = DiffContent::from_unified(FIXTURE);
    let rows = build_diff_rows(&content, DiffMode::Inline);
    let line4 = rows
        .iter()
        .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "line4"))
        .expect("line4");
    let patch = build_partial_patch(
        &content,
        DiffMode::Inline,
        line4,
        line4,
        PartialPatchKind::Stage,
        "hello.ts",
    )
    .expect("patch");
    assert!(patch.contains("+line4"), "{patch}");
    assert!(!patch.contains("line2 changed"), "{patch}");
    assert!(patch.contains(" line2\n"), "{patch}");
}

#[test]
fn partial_patch_fails_closed_on_context_only_and_committed() {
    let content = two_hunk_content();
    let rows = build_diff_rows(&content, DiffMode::Inline);
    let ctx = rows
        .iter()
        .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "keep-a"))
        .expect("context");
    let err = build_partial_patch(
        &content,
        DiffMode::Inline,
        ctx,
        ctx,
        PartialPatchKind::Stage,
        "regions.txt",
    )
    .unwrap_err();
    assert!(err.contains("nothing to stage"), "{err}");

    let committed = DiffContent {
        staged: String::new(),
        unstaged: TWO_HUNKS.into(),
        is_new: false,
        is_committed: true,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let err = build_partial_patch(
        &committed,
        DiffMode::Inline,
        0,
        3,
        PartialPatchKind::Stage,
        "regions.txt",
    )
    .unwrap_err();
    assert!(err.contains("committed"), "{err}");
}

#[test]
fn partial_patch_unstages_from_the_staged_section() {
    let content = DiffContent {
        staged: TWO_HUNKS.into(),
        unstaged: String::new(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let rows = build_diff_rows(&content, DiffMode::Inline);
    let first = rows
        .iter()
        .position(|r| matches!(r, DiffRow::Hunk { .. }))
        .expect("hunk");
    let second = rows
        .iter()
        .enumerate()
        .skip(first + 1)
        .find_map(|(i, r)| matches!(r, DiffRow::Hunk { .. }).then_some(i))
        .expect("second");
    let patch = build_partial_patch(
        &content,
        DiffMode::Inline,
        first,
        second - 1,
        PartialPatchKind::Unstage,
        "regions.txt",
    )
    .expect("patch");
    assert!(
        patch.contains("ALPHA-NEW") && !patch.contains("OMEGA-NEW"),
        "{patch}"
    );
    let err = build_partial_patch(
        &content,
        DiffMode::Inline,
        first,
        second - 1,
        PartialPatchKind::Stage,
        "regions.txt",
    )
    .unwrap_err();
    assert!(err.contains("nothing to stage"), "{err}");
}

const BINARY_STUB: &str = "Binary files a/x and b/x differ\n";

#[test]
fn partial_patch_fails_closed_on_binary() {
    let unstaged = DiffContent::from_unified(BINARY_STUB);
    let end = build_diff_rows(&unstaged, DiffMode::Inline)
        .len()
        .saturating_sub(1);
    let err = build_partial_patch(
        &unstaged,
        DiffMode::Inline,
        0,
        end,
        PartialPatchKind::Stage,
        "x.bin",
    )
    .unwrap_err();
    assert!(err.contains("cannot stage a binary highlight"), "{err}");

    let staged = DiffContent {
        staged: BINARY_STUB.into(),
        unstaged: String::new(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let end = build_diff_rows(&staged, DiffMode::Inline)
        .len()
        .saturating_sub(1);
    let err = build_partial_patch(
        &staged,
        DiffMode::Inline,
        0,
        end,
        PartialPatchKind::Unstage,
        "x.bin",
    )
    .unwrap_err();
    assert!(err.contains("cannot unstage a binary highlight"), "{err}");
}

#[test]
fn partial_patch_fails_closed_on_mixed_sections() {
    let content = DiffContent {
        staged: TWO_HUNKS.into(),
        unstaged: TWO_HUNKS.into(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let end = build_diff_rows(&content, DiffMode::Inline).len() - 1;
    let err = build_partial_patch(
        &content,
        DiffMode::Inline,
        0,
        end,
        PartialPatchKind::Stage,
        "regions.txt",
    )
    .unwrap_err();
    assert!(err.contains("highlight spans staged and unstaged"), "{err}");
    let err = build_partial_patch(
        &content,
        DiffMode::Inline,
        0,
        end,
        PartialPatchKind::Unstage,
        "regions.txt",
    )
    .unwrap_err();
    assert!(err.contains("highlight spans staged and unstaged"), "{err}");
}

fn revert_patch(content: &DiffContent, start: usize, end: usize) -> Result<String, String> {
    build_partial_patch(
        content,
        DiffMode::Inline,
        start,
        end,
        PartialPatchKind::Revert,
        "regions.txt",
    )
}

fn line_row(content: &DiffContent, text: &str) -> usize {
    build_diff_rows(content, DiffMode::Inline)
        .iter()
        .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == text))
        .unwrap_or_else(|| panic!("row {text}"))
}

const PAIRED_HUNK: &str = "\
diff --git a/regions.txt b/regions.txt
index 1111111..2222222 100644
--- a/regions.txt
+++ b/regions.txt
@@ -1,4 +1,4 @@
 ctx-a
-old-1
-old-2
+new-1
+new-2
 ctx-b
";

#[test]
fn revert_patch_mirrors_the_stage_transform() {
    let content = DiffContent::from_unified(PAIRED_HUNK);
    let row = line_row(&content, "new-1");
    let patch = revert_patch(&content, row, row).expect("patch");
    let hunk = &patch[patch.find("@@").expect("hunk")..];
    assert_eq!(
        hunk, "@@ -1,3 +1,4 @@\n ctx-a\n+new-1\n new-2\n ctx-b\n",
        "unselected + is context, unselected - drops"
    );
    let stage = build_partial_patch(
        &content,
        DiffMode::Inline,
        row,
        row,
        PartialPatchKind::Stage,
        "regions.txt",
    )
    .expect("stage patch");
    let hunk = &stage[stage.find("@@").expect("hunk")..];
    assert_eq!(
        hunk, "@@ -1,4 +1,5 @@\n ctx-a\n old-1\n+new-1\n old-2\n ctx-b\n",
        "stage keeps the forward transform; new-1 lands in pair order"
    );
}

#[test]
fn revert_patch_fails_closed() {
    let content = two_hunk_content();
    let ctx = line_row(&content, "keep-a");
    let err = revert_patch(&content, ctx, ctx).unwrap_err();
    assert_eq!(err, "nothing to revert in highlight");

    let committed = DiffContent {
        is_committed: true,
        vs_worktree: false,
        committed_label: false,
        error: None,
        ..two_hunk_content()
    };
    let err = revert_patch(&committed, 0, 3).unwrap_err();
    assert_eq!(err, "cannot revert a committed diff");

    let binary = DiffContent::from_unified(BINARY_STUB);
    let end = build_diff_rows(&binary, DiffMode::Inline).len() - 1;
    let err = revert_patch(&binary, 0, end).unwrap_err();
    assert_eq!(err, "cannot revert a binary highlight");

    let new_file = DiffContent {
        staged: String::new(),
        unstaged: synthesize_all_add_diff("one\ntwo\n"),
        is_new: true,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let end = build_diff_rows(&new_file, DiffMode::Inline).len() - 1;
    let err = revert_patch(&new_file, 0, end).unwrap_err();
    assert_eq!(err, "cannot revert lines of a new file");

    // `git add -N`: tracked, so not `is_new`, but the unstaged diff is
    // a new-file patch.
    let intent_to_add = DiffContent::from_unified(
            "diff --git a/regions.txt b/regions.txt\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/regions.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n",
        );
    assert!(!intent_to_add.is_new);
    let end = build_diff_rows(&intent_to_add, DiffMode::Inline).len() - 1;
    let err = revert_patch(&intent_to_add, 0, end).unwrap_err();
    assert_eq!(err, "cannot revert lines of a new file");

    let staged_only = DiffContent {
        staged: TWO_HUNKS.into(),
        unstaged: String::new(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let end = build_diff_rows(&staged_only, DiffMode::Inline).len() - 1;
    let err = revert_patch(&staged_only, 0, end).unwrap_err();
    assert_eq!(err, "nothing to revert in highlight");

    let mixed = DiffContent {
        staged: TWO_HUNKS.into(),
        unstaged: TWO_HUNKS.into(),
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
    };
    let end = build_diff_rows(&mixed, DiffMode::Inline).len() - 1;
    let err = revert_patch(&mixed, 0, end).unwrap_err();
    assert_eq!(err, "highlight spans staged and unstaged");

    let renamed = DiffContent::from_unified(
            "diff --git a/old.txt b/regions.txt\nsimilarity index 90%\nrename from old.txt\nrename to regions.txt\n--- a/old.txt\n+++ b/regions.txt\n@@ -1,2 +1,2 @@\n keep\n-a\n+b\n",
        );
    let end = build_diff_rows(&renamed, DiffMode::Inline).len() - 1;
    let err = revert_patch(&renamed, 0, end).unwrap_err();
    assert_eq!(err, "cannot revert highlight");

    // A reverse apply would undo the mode change or recreate the file.
    for header in [
        "old mode 100644\nnew mode 100755\n",
        "deleted file mode 100644\n",
    ] {
        let content = DiffContent::from_unified(format!(
                "diff --git a/regions.txt b/regions.txt\n{header}--- a/regions.txt\n+++ b/regions.txt\n@@ -1,2 +1,2 @@\n keep\n-a\n+b\n"
            ));
        let end = build_diff_rows(&content, DiffMode::Inline).len() - 1;
        let err = revert_patch(&content, 0, end).unwrap_err();
        assert_eq!(err, "cannot revert highlight", "{header}");
    }
}

mod real_git {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::git::{apply_cached_patch, apply_worktree_patch_reverse, blob_bytes};
    use crate::testutil::{git, init_repo, unique_dir};

    const REGIONS: &str = "\
keep-a
keep-b
keep-c
ALPHA-OLD
keep-d
keep-e
keep-f
pad-1
pad-2
pad-3
pad-4
pad-5
pad-6
OMEGA-OLD
keep-x
keep-y
keep-z
";

    fn repo_with_regions(prefix: &str) -> std::path::PathBuf {
        let dir = unique_dir(prefix);
        init_repo(&dir);
        fs::write(dir.join("regions.txt"), REGIONS).unwrap();
        git(&dir, &["add", "regions.txt"]);
        git(&dir, &["commit", "-q", "-m", "regions"]);
        dir
    }

    fn content(dir: &Path) -> DiffContent {
        DiffContent {
            staged: exec_git(&["diff", "--cached", "--", "regions.txt"], dir),
            unstaged: exec_git(&["diff", "--", "regions.txt"], dir),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        }
    }

    fn worktree(dir: &Path) -> String {
        fs::read_to_string(dir.join("regions.txt")).unwrap()
    }

    /// `main` commits [`REGIONS`]; `feature` commits ALPHA-NEW and
    /// OMEGA-NEW and a new `summary.txt`, and is checked out clean.
    /// Returns the dir and the compare content for `path`.
    fn committed_compare(prefix: &str, path: &str) -> (std::path::PathBuf, DiffContent) {
        let dir = repo_with_regions(prefix);
        git(&dir, &["checkout", "-q", "-b", "feature"]);
        let edited = REGIONS
            .replace("ALPHA-OLD", "ALPHA-NEW")
            .replace("OMEGA-OLD", "OMEGA-NEW");
        fs::write(dir.join("regions.txt"), edited).unwrap();
        fs::write(dir.join("summary.txt"), "summary\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "feature"]);
        let lines = crate::git::diff_compare_file_ctx(&dir, "main", "HEAD", path, None, None)
            .expect("compare diff");
        (dir, DiffContent::from_compare_lines(lines))
    }

    #[test]
    fn revert_committed_restores_one_compare_hunk_to_the_merge_base() {
        let (dir, content) = committed_compare("ws-diff-revert-committed", "regions.txt");
        let rows = build_diff_rows(&content, DiffMode::Inline);
        assert!(matches!(rows[0], DiffRow::Section(DiffSection::Committed)));
        let alpha = line_row(&content, "ALPHA-NEW");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            1,
            alpha,
            PartialPatchKind::RevertCommitted,
            "regions.txt",
        )
        .expect("patch");
        apply_worktree_patch_reverse(&dir, &patch).expect("revert");

        assert_eq!(worktree(&dir), REGIONS.replace("OMEGA-OLD", "OMEGA-NEW"));
        assert!(exec_git(&["diff", "--cached"], &dir).is_empty());

        // Plain Revert still refuses a committed diff.
        let err = build_partial_patch(
            &content,
            DiffMode::Inline,
            1,
            alpha,
            PartialPatchKind::Revert,
            "regions.txt",
        )
        .unwrap_err();
        assert_eq!(err, "cannot revert a committed diff");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_committed_refuses_new_files_and_worktree_diffs() {
        let (dir, added) = committed_compare("ws-diff-revert-committed-new", "summary.txt");
        let end = build_diff_rows(&added, DiffMode::Inline).len() - 1;
        let err = build_partial_patch(
            &added,
            DiffMode::Inline,
            0,
            end,
            PartialPatchKind::RevertCommitted,
            "summary.txt",
        )
        .unwrap_err();
        assert_eq!(err, "cannot revert lines of a new file");

        fs::write(dir.join("regions.txt"), "dirty\n").unwrap();
        let worktree_diff = content(&dir);
        let err = build_partial_patch(
            &worktree_diff,
            DiffMode::Inline,
            0,
            3,
            PartialPatchKind::RevertCommitted,
            "regions.txt",
        )
        .unwrap_err();
        assert_eq!(err, "cannot revert highlight");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_range_restores_only_the_highlighted_hunk() {
        let dir = repo_with_regions("ws-diff-revert-hunk");
        let staged_body = REGIONS.replace("keep-x", "KEEP-X-STAGED");
        fs::write(dir.join("regions.txt"), &staged_body).unwrap();
        git(&dir, &["add", "regions.txt"]);
        let edited = staged_body
            .replace("ALPHA-OLD", "ALPHA-NEW")
            .replace("OMEGA-OLD", "OMEGA-NEW");
        fs::write(dir.join("regions.txt"), &edited).unwrap();
        let cached_before = exec_git(&["diff", "--cached"], &dir);

        let content = content(&dir);
        let rows = build_diff_rows(&content, DiffMode::Inline);
        let unstaged = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Section(DiffSection::Unstaged)))
            .expect("unstaged section");
        let alpha = line_row(&content, "ALPHA-NEW");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            unstaged + 1,
            alpha,
            PartialPatchKind::Revert,
            "regions.txt",
        )
        .expect("patch");
        apply_worktree_patch_reverse(&dir, &patch).expect("revert");

        assert_eq!(worktree(&dir), edited.replace("ALPHA-NEW", "ALPHA-OLD"));
        assert_eq!(exec_git(&["diff", "--cached"], &dir), cached_before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_range_drops_one_added_line_and_keeps_its_neighbour() {
        let dir = repo_with_regions("ws-diff-revert-line");
        let edited = REGIONS
            .replace("keep-b\n", "keep-b\nadd-1\nadd-2\n")
            .replace("keep-c", "KEEP-C-NEW");
        fs::write(dir.join("regions.txt"), &edited).unwrap();

        let content = content(&dir);
        let row = line_row(&content, "add-1");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            row,
            row,
            PartialPatchKind::Revert,
            "regions.txt",
        )
        .expect("patch");
        apply_worktree_patch_reverse(&dir, &patch).expect("revert");

        assert_eq!(worktree(&dir), edited.replace("add-1\n", ""));
        assert!(exec_git(&["diff", "--cached"], &dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_mode_revert_of_one_paired_row_keeps_the_other_edits() {
        let dir = repo_with_regions("ws-diff-revert-split");
        let edited = REGIONS
            .replace("keep-b", "KEEP-B")
            .replace("keep-d", "KEEP-D");
        fs::write(dir.join("regions.txt"), &edited).unwrap();

        let content = content(&dir);
        let rows = build_diff_rows(&content, DiffMode::SideBySide);
        let paired = rows
            .iter()
            .position(|r| {
                matches!(r, DiffRow::Line { left, right: Some(right) }
                        if left.text == "keep-b" && right.text == "KEEP-B")
            })
            .expect("paired keep-b / KEEP-B row");
        let hunks = rows
            .iter()
            .filter(|r| matches!(r, DiffRow::Hunk { .. }))
            .count();
        assert_eq!(hunks, 1, "both edits share one hunk");
        let patch = build_partial_patch(
            &content,
            DiffMode::SideBySide,
            paired,
            paired,
            PartialPatchKind::Revert,
            "regions.txt",
        )
        .expect("patch");
        apply_worktree_patch_reverse(&dir, &patch).expect("revert");

        assert_eq!(worktree(&dir), edited.replace("KEEP-B", "keep-b"));
        assert!(exec_git(&["diff", "--cached"], &dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Change `keep-b` / `keep-c` (one run of two pairs), highlight one
    /// row, apply `kind`, and return (index, worktree) for regions.txt.
    fn pair_run_apply(
        mode: DiffMode,
        kind: PartialPatchKind,
        left: &str,
        right: Option<&str>,
    ) -> (String, String) {
        let dir = repo_with_regions("ws-diff-pair-run");
        let edited = REGIONS
            .replace("keep-b", "KEEP-B")
            .replace("keep-c", "KEEP-C");
        fs::write(dir.join("regions.txt"), &edited).unwrap();
        let content = content(&dir);
        let row = build_diff_rows(&content, mode)
            .iter()
            .position(|r| match r {
                DiffRow::Line { left: l, right: r } => {
                    l.text == left
                        && right.is_none_or(|want| r.as_ref().is_some_and(|r| r.text == want))
                }
                _ => false,
            })
            .unwrap_or_else(|| panic!("row {left} / {right:?}"));
        let patch =
            build_partial_patch(&content, mode, row, row, kind, "regions.txt").expect("patch");
        match kind {
            PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
            PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
            PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                apply_worktree_patch_reverse(&dir, &patch)
            }
        }
        .unwrap_or_else(|err| panic!("{kind:?} {left}: {err}\n{patch}"));
        let index = blob_bytes(&dir, "", "regions.txt").expect("index blob");
        let out = (String::from_utf8(index).unwrap(), worktree(&dir));
        let _ = fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn one_pair_of_a_change_run_keeps_line_order() {
        use DiffMode::{Inline, SideBySide};
        use PartialPatchKind::{Revert, Stage};
        let edited = REGIONS
            .replace("keep-b", "KEEP-B")
            .replace("keep-c", "KEEP-C");
        let only_c = REGIONS.replace("keep-c", "KEEP-C");
        let only_b = REGIONS.replace("keep-b", "KEEP-B");
        let both_b = REGIONS.replace("keep-b\n", "keep-b\nKEEP-B\n");
        let c_back = edited.replace("KEEP-B\n", "KEEP-B\nkeep-c\n");
        // Split binds a pair to one row. Inline has no row for a whole
        // pair, so it picks the one line whose place depends on the pair.
        let cases = [
            (
                SideBySide,
                Stage,
                "keep-c",
                Some("KEEP-C"),
                &only_c,
                &edited,
            ),
            (
                SideBySide,
                Stage,
                "keep-b",
                Some("KEEP-B"),
                &only_b,
                &edited,
            ),
            (
                SideBySide,
                Revert,
                "keep-c",
                Some("KEEP-C"),
                &REGIONS.to_string(),
                &only_b,
            ),
            (
                SideBySide,
                Revert,
                "keep-b",
                Some("KEEP-B"),
                &REGIONS.to_string(),
                &only_c,
            ),
            (Inline, Stage, "KEEP-B", None, &both_b, &edited),
            (
                Inline,
                Revert,
                "keep-c",
                None,
                &REGIONS.to_string(),
                &c_back,
            ),
        ];
        for (mode, kind, left, right, index, worktree) in cases {
            assert_eq!(
                pair_run_apply(mode, kind, left, right),
                (index.clone(), worktree.clone()),
                "{mode:?} {kind:?} {left}"
            );
        }
    }

    /// `l1`..`l30` with the chosen edits: `X1` / `X2` after `l2` (the
    /// first hunk changes the line count), `l12` gone, `ADD-Y` after
    /// `l22`.
    fn shifted_body(x1: bool, x2: bool, drop_l12: bool, add_y: bool) -> String {
        let mut out = String::new();
        for n in 1..=30 {
            if !(drop_l12 && n == 12) {
                out.push_str(&format!("l{n}\n"));
            }
            if n == 2 {
                if x1 {
                    out.push_str("X1\n");
                }
                if x2 {
                    out.push_str("X2\n");
                }
            }
            if n == 22 && add_y {
                out.push_str("ADD-Y\n");
            }
        }
        out
    }

    /// Highlight rows `from..=to` of a three-hunk diff, apply `kind`,
    /// and return (index, worktree). `context` sets `diff.context`.
    fn shifted_apply(
        context: Option<&str>,
        kind: PartialPatchKind,
        from: &str,
        to: &str,
    ) -> (String, String) {
        let dir = unique_dir("ws-diff-hunk-start");
        init_repo(&dir);
        if let Some(n) = context {
            git(&dir, &["config", "diff.context", n]);
        }
        let file = dir.join("lines.txt");
        fs::write(&file, shifted_body(false, false, false, false)).unwrap();
        git(&dir, &["add", "lines.txt"]);
        git(&dir, &["commit", "-q", "-m", "lines"]);
        fs::write(&file, shifted_body(true, true, true, true)).unwrap();
        if kind == PartialPatchKind::Unstage {
            git(&dir, &["add", "lines.txt"]);
        }
        let content = DiffContent {
            staged: exec_git(&["diff", "--cached", "--", "lines.txt"], &dir),
            unstaged: exec_git(&["diff", "--", "lines.txt"], &dir),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        };
        let hunks = parse_unified_diff(if kind == PartialPatchKind::Unstage {
            &content.staged
        } else {
            &content.unstaged
        })
        .len();
        assert_eq!(hunks, 3, "three separate hunks at context {context:?}");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            line_row(&content, from),
            line_row(&content, to),
            kind,
            "lines.txt",
        )
        .expect("patch");
        match kind {
            PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
            PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
            PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                apply_worktree_patch_reverse(&dir, &patch)
            }
        }
        .unwrap_or_else(|err| panic!("{kind:?} {from}..{to}: {err}\n{patch}"));
        let index = blob_bytes(&dir, "", "lines.txt").expect("index blob");
        let out = (
            String::from_utf8(index).unwrap(),
            fs::read_to_string(&file).unwrap(),
        );
        let _ = fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn later_hunks_land_after_skipped_or_partial_earlier_hunks() {
        use PartialPatchKind::{Revert, Stage, Unstage};
        let base = shifted_body(false, false, false, false);
        let edited = shifted_body(true, true, true, true);
        // (from, to, stage result, unstage / revert result)
        let picks = [
            (
                "ADD-Y",
                "ADD-Y",
                shifted_body(false, false, false, true),
                shifted_body(true, true, true, false),
            ),
            (
                "l12",
                "l12",
                shifted_body(false, false, true, false),
                shifted_body(true, true, false, true),
            ),
            (
                "X2",
                "l12",
                shifted_body(false, true, true, false),
                shifted_body(true, false, false, true),
            ),
        ];
        for context in [Some("0"), None] {
            for (from, to, staged, undone) in &picks {
                let label = format!("context {context:?} {from}..{to}");
                assert_eq!(
                    shifted_apply(context, Stage, from, to),
                    (staged.clone(), edited.clone()),
                    "Stage {label}"
                );
                assert_eq!(
                    shifted_apply(context, Unstage, from, to),
                    (undone.clone(), edited.clone()),
                    "Unstage {label}"
                );
                assert_eq!(
                    shifted_apply(context, Revert, from, to),
                    (base.clone(), undone.clone()),
                    "Revert {label}"
                );
            }
        }
    }

    const NO_EOL_BASE: &str = "a\nb\nlast";
    const NO_EOL_EDIT: &str = "a\nb\nLAST";

    /// Highlight one row of a last-line change in a file with no
    /// trailing newline, apply `kind`, and return (index, worktree).
    fn no_eol_apply(
        prefix: &str,
        kind: PartialPatchKind,
        row: &str,
    ) -> Result<(String, String), String> {
        let dir = unique_dir(prefix);
        init_repo(&dir);
        let file = dir.join("no-eol.txt");
        fs::write(&file, NO_EOL_BASE).unwrap();
        git(&dir, &["add", "no-eol.txt"]);
        git(&dir, &["commit", "-q", "-m", "no eol"]);
        fs::write(&file, NO_EOL_EDIT).unwrap();
        if kind == PartialPatchKind::Unstage {
            git(&dir, &["add", "no-eol.txt"]);
        }
        let content = DiffContent {
            staged: exec_git(&["diff", "--cached", "--", "no-eol.txt"], &dir),
            unstaged: exec_git(&["diff", "--", "no-eol.txt"], &dir),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        };
        let row = line_row(&content, row);
        let result = build_partial_patch(&content, DiffMode::Inline, row, row, kind, "no-eol.txt")
            .and_then(|patch| match kind {
                PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
                PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
                PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                    apply_worktree_patch_reverse(&dir, &patch)
                }
            })
            .map(|()| {
                let index = blob_bytes(&dir, "", "no-eol.txt").expect("index blob");
                (
                    String::from_utf8(index).unwrap(),
                    fs::read_to_string(&file).unwrap(),
                )
            });
        let _ = fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn no_eol_last_line_moves_one_side_without_joining_lines() {
        use PartialPatchKind::{Revert, Stage, Unstage};
        let restored = "a\nb\nlast\nLAST";
        let cases: [(PartialPatchKind, &str, &str, &str); 6] = [
            (Stage, "last", "a\nb\n", NO_EOL_EDIT),
            (Stage, "LAST", restored, NO_EOL_EDIT),
            (Unstage, "last", restored, NO_EOL_EDIT),
            (Unstage, "LAST", "a\nb\n", NO_EOL_EDIT),
            (Revert, "last", NO_EOL_BASE, restored),
            (Revert, "LAST", NO_EOL_BASE, "a\nb\n"),
        ];
        for (kind, row, index, worktree) in cases {
            let got = no_eol_apply("ws-diff-no-eol", kind, row)
                .unwrap_or_else(|err| panic!("{kind:?} {row}: {err}"));
            assert_eq!(
                got,
                (index.to_string(), worktree.to_string()),
                "{kind:?} {row}"
            );
        }
    }

    /// Commit `base` as `f.txt` and write `edit` over it, or with no `base`
    /// leave `edit` untracked. The diff loads the way the TUI loads it
    /// ([`load_file_diff`]), so the test sees git's real output bytes.
    fn exact_bytes_repo(prefix: &str, base: Option<&[u8]>, edit: &[u8]) -> (PathBuf, DiffContent) {
        let dir = unique_dir(prefix);
        init_repo(&dir);
        let file = dir.join("f.txt");
        if let Some(base) = base {
            fs::write(&file, base).unwrap();
            git(&dir, &["add", "f.txt"]);
            git(&dir, &["commit", "-q", "-m", "base"]);
        }
        fs::write(&file, edit).unwrap();
        let change = crate::snapshot::FileChange {
            path: "f.txt".into(),
            staged_status: None,
            unstaged_status: base.map(|_| "M".to_string()),
            untracked: base.is_none(),
            old_path: None,
        };
        let content = load_file_diff(&dir, "", &change, None);
        (dir, content)
    }

    /// Stage painted rows `from..=to` and read the index blob of `f.txt`.
    fn stage_rows(dir: &Path, content: &DiffContent, from: usize, to: usize) -> Vec<u8> {
        let patch = build_partial_patch(
            content,
            DiffMode::Inline,
            from,
            to,
            PartialPatchKind::Stage,
            "f.txt",
        )
        .expect("patch");
        apply_cached_patch(dir, &patch, false).unwrap_or_else(|err| panic!("{err}\n{patch}"));
        blob_bytes(dir, "", "f.txt").expect("index blob")
    }

    /// A line stage of a CRLF file keeps every `\r`, and a lone `\r` inside
    /// a line does not split it, so the line numbers match the file.
    #[test]
    fn line_stage_of_a_crlf_file_writes_its_exact_bytes() {
        let (dir, content) = exact_bytes_repo(
            "ws-diff-crlf",
            Some(b"p\rq\r\na\r\nc\r\n"),
            b"p\rq\r\na\r\nB\r\nc\r\nd\r\n",
        );
        let row = line_row(&content, "B");
        let line = row_line_ref(&content, DiffMode::Inline, row).expect("line ref");
        assert_eq!(line.new_no, Some(3));
        assert_eq!(
            String::from_utf8(stage_rows(&dir, &content, row, row)).unwrap(),
            "p\rq\r\na\r\nB\r\nc\r\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Commit `base` as `f.txt`, write `edit` over it (and stage it for
    /// [`PartialPatchKind::Unstage`]), apply `kind` to the painted row
    /// `text` the way the TUI does, and return (index blob, worktree bytes).
    fn exact_bytes_apply(
        prefix: &str,
        kind: PartialPatchKind,
        base: &[u8],
        edit: &[u8],
        text: &str,
    ) -> (Vec<u8>, Vec<u8>) {
        let dir = unique_dir(prefix);
        init_repo(&dir);
        let file = dir.join("f.txt");
        fs::write(&file, base).unwrap();
        git(&dir, &["add", "f.txt"]);
        git(&dir, &["commit", "-q", "-m", "base"]);
        fs::write(&file, edit).unwrap();
        let staged = kind == PartialPatchKind::Unstage;
        if staged {
            git(&dir, &["add", "f.txt"]);
        }
        let change = crate::snapshot::FileChange {
            path: "f.txt".into(),
            staged_status: staged.then(|| "M".to_string()),
            unstaged_status: (!staged).then(|| "M".to_string()),
            untracked: false,
            old_path: None,
        };
        let content = load_file_diff(&dir, "", &change, None);
        let row = line_row(&content, text);
        let patch = build_partial_patch(&content, DiffMode::Inline, row, row, kind, "f.txt")
            .expect("patch");
        match kind {
            PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
            PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
            PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                apply_worktree_patch_reverse(&dir, &patch)
            }
        }
        .unwrap_or_else(|err| panic!("{kind:?}: {err}\n{patch}"));
        let out = (
            blob_bytes(&dir, "", "f.txt").expect("index blob"),
            fs::read(&file).unwrap(),
        );
        let _ = fs::remove_dir_all(&dir);
        out
    }

    /// A line unstage of a CRLF file keeps every `\r` in the index and
    /// leaves the worktree as it was.
    #[test]
    fn line_unstage_of_a_crlf_file_writes_its_exact_bytes() {
        let edit: &[u8] = b"p\rq\r\na\r\nB\r\nc\r\nd\r\n";
        let (index, worktree) = exact_bytes_apply(
            "ws-diff-crlf-unstage",
            PartialPatchKind::Unstage,
            b"p\rq\r\na\r\nc\r\n",
            edit,
            "B",
        );
        assert_eq!(String::from_utf8(index).unwrap(), "p\rq\r\na\r\nc\r\nd\r\n");
        assert_eq!(worktree, edit);
    }

    /// A line revert of a CRLF file keeps every `\r` in the worktree and
    /// leaves the index as it was.
    #[test]
    fn line_revert_of_a_crlf_file_writes_its_exact_bytes() {
        let base: &[u8] = b"p\rq\r\na\r\nc\r\n";
        let (index, worktree) = exact_bytes_apply(
            "ws-diff-crlf-revert",
            PartialPatchKind::Revert,
            base,
            b"p\rq\r\na\r\nB\r\nc\r\nd\r\n",
            "B",
        );
        assert_eq!(index, base);
        assert_eq!(
            String::from_utf8(worktree).unwrap(),
            "p\rq\r\na\r\nc\r\nd\r\n"
        );
    }

    /// The last line of git's diff output keeps its trailing whitespace
    /// when one line of it is staged.
    #[test]
    fn line_stage_keeps_trailing_whitespace() {
        let (dir, content) = exact_bytes_repo("ws-diff-trailing-ws", Some(b"x\n"), b"x\ny \t\n");
        let row = line_row(&content, "y   ");
        assert_eq!(
            String::from_utf8(stage_rows(&dir, &content, row, row)).unwrap(),
            "x\ny \t\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Staging every line of an untracked CRLF file with no final newline
    /// writes the file's bytes: the `\r` stay and no newline is added.
    #[test]
    fn line_stage_of_a_new_crlf_file_without_final_newline_writes_its_exact_bytes() {
        let (dir, content) = exact_bytes_repo("ws-diff-new-crlf", None, b"u1\r\nu2");
        let first = line_row(&content, "u1");
        let last = line_row(&content, "u2");
        assert_eq!(
            String::from_utf8(stage_rows(&dir, &content, first, last)).unwrap(),
            "u1\r\nu2"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A diff with bytes that are not UTF-8 paints U+FFFD, and a line stage
    /// refuses instead of writing U+FFFD into the index.
    #[test]
    fn line_stage_refuses_a_diff_that_is_not_utf8() {
        let (dir, content) = exact_bytes_repo("ws-diff-not-utf8", Some(b"a\n"), b"a\n\xffz\n");
        let row = line_row(&content, "\u{FFFD}z");
        let err = build_partial_patch(
            &content,
            DiffMode::Inline,
            row,
            row,
            PartialPatchKind::Stage,
            "f.txt",
        )
        .unwrap_err();
        assert_eq!(err, "cannot stage highlight: diff is not UTF-8 text");
        assert_eq!(blob_bytes(&dir, "", "f.txt").expect("index blob"), b"a\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_range_keeps_an_adjacent_staged_addition() {
        let dir = repo_with_regions("ws-diff-unstage-line");
        let edited = REGIONS.replace("keep-b\n", "keep-b\nadd-1\nadd-2\n");
        fs::write(dir.join("regions.txt"), &edited).unwrap();
        git(&dir, &["add", "regions.txt"]);

        let content = content(&dir);
        let row = line_row(&content, "add-1");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            row,
            row,
            PartialPatchKind::Unstage,
            "regions.txt",
        )
        .expect("patch");
        apply_cached_patch(&dir, &patch, true).expect("unstage one line");

        let cached = exec_git(&["diff", "--cached"], &dir);
        let unstaged = exec_git(&["diff"], &dir);
        assert!(
            cached.contains("+add-2") && !cached.contains("add-1"),
            "{cached}"
        );
        assert!(
            unstaged.contains("+add-1") && !unstaged.contains("+add-2"),
            "{unstaged}"
        );
        assert_eq!(worktree(&dir), edited);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The unstaged `regions.txt` diff through the production loader.
    fn loaded_unstaged(dir: &Path) -> DiffContent {
        let change = FileChange {
            path: "regions.txt".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        };
        let parent = dir.parent().expect("fixture parent");
        let name = dir.file_name().expect("fixture name").to_string_lossy();
        load_file_diff(parent, &name, &change, None)
    }

    #[test]
    fn user_color_diff_always_leaves_no_escape_in_loaded_diffs() {
        let dir = repo_with_regions("ws-diff-user-color");
        git(&dir, &["config", "color.diff", "always"]);
        fs::write(
            dir.join("regions.txt"),
            REGIONS.replace("ALPHA-OLD", "ALPHA-NEW"),
        )
        .unwrap();

        let content = loaded_unstaged(&dir);
        assert!(content.error.is_none(), "{content:?}");
        assert!(content.unstaged.contains("\n+ALPHA-NEW"), "{content:?}");
        assert!(!content.unstaged.contains('\x1b'), "{:?}", content.unstaged);

        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        let commit = crate::git::diff_commit_file(&dir, &head, "regions.txt").join("\n");
        assert!(commit.contains("+ALPHA-OLD"), "{commit}");
        assert!(!commit.contains('\x1b'), "{commit:?}");
        // The seed commit has no parent: this read takes the `show` fallback.
        let root = exec_git(&["rev-list", "--max-parents=0", "HEAD"], &dir);
        let shown = crate::git::diff_commit_file(&dir, &root, "README.md").join("\n");
        assert!(shown.contains("+# seed"), "{shown}");
        assert!(!shown.contains('\x1b'), "{shown:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_external_diff_and_textconv_do_not_replace_loaded_hunks() {
        let dir = repo_with_regions("ws-diff-user-ext");
        git(&dir, &["config", "diff.external", "echo EXTERNAL"]);
        fs::write(
            dir.join("regions.txt"),
            REGIONS.replace("ALPHA-OLD", "ALPHA-NEW"),
        )
        .unwrap();

        let content = loaded_unstaged(&dir);
        assert!(content.error.is_none(), "{content:?}");
        assert!(
            content.unstaged.contains("@@") && content.unstaged.contains("\n+ALPHA-NEW"),
            "{:?}",
            content.unstaged
        );
        assert!(
            !content.unstaged.contains("EXTERNAL"),
            "{:?}",
            content.unstaged
        );

        // A textconv filter rewrites the lines, so a patch built from them
        // would not apply to the real blob.
        git(&dir, &["config", "--unset", "diff.external"]);
        git(&dir, &["config", "diff.shout.textconv", "echo TEXTCONV"]);
        fs::write(dir.join(".git/info/attributes"), "regions.txt diff=shout\n").unwrap();
        let content = loaded_unstaged(&dir);
        assert!(
            content.unstaged.contains("\n+ALPHA-NEW"),
            "{:?}",
            content.unstaged
        );
        assert!(
            !content.unstaged.contains("TEXTCONV"),
            "{:?}",
            content.unstaged
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_diff_noprefix_still_stages_one_line() {
        let dir = repo_with_regions("ws-diff-user-noprefix");
        git(&dir, &["config", "diff.noprefix", "true"]);
        assert_one_line_stage_applies(&dir);
    }

    /// Git 2.45 and later read `diff.srcPrefix` / `diff.dstPrefix`, and
    /// `diff.noprefix=false` does not reset them. A patch applies with
    /// `-p1`, so an empty prefix or a prefix of two path components breaks
    /// it. An older git ignores the keys, so the stage applies there too.
    #[test]
    fn user_diff_src_dst_prefix_still_stages_one_line() {
        for (name, src, dst) in [
            ("ws-diff-user-prefix-deep", "x/y/", "z/w/"),
            ("ws-diff-user-prefix-empty", "", ""),
        ] {
            let dir = repo_with_regions(name);
            git(&dir, &["config", "diff.srcPrefix", src]);
            git(&dir, &["config", "diff.dstPrefix", dst]);
            assert_one_line_stage_applies(&dir);
        }
    }

    /// Edits two lines of `regions.txt`, stages only the `ALPHA` line with
    /// a partial patch, and checks the index and the worktree.
    fn assert_one_line_stage_applies(dir: &Path) {
        let edited = REGIONS
            .replace("ALPHA-OLD", "ALPHA-NEW")
            .replace("OMEGA-OLD", "OMEGA-NEW");
        fs::write(dir.join("regions.txt"), &edited).unwrap();

        let content = loaded_unstaged(dir);
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            line_row(&content, "ALPHA-OLD"),
            line_row(&content, "ALPHA-NEW"),
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .expect("patch");
        apply_cached_patch(dir, &patch, false)
            .unwrap_or_else(|err| panic!("stage one line: {err}\n{patch}"));

        let index = String::from_utf8(blob_bytes(dir, "", "regions.txt").expect("index blob"))
            .expect("utf8 index blob");
        assert_eq!(index, REGIONS.replace("ALPHA-OLD", "ALPHA-NEW"));
        assert_eq!(worktree(dir), edited);
        let _ = fs::remove_dir_all(dir);
    }
}
