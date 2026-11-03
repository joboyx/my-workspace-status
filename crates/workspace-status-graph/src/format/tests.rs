use super::*;
use crate::glyphs::{ASCII, UNICODE};
use crate::model::{Commit, GraphRow, Stash, Worktree};

#[test]
fn short_id_keeps_ids_shorter_than_seven() {
    assert_eq!(short_id("abc"), "abc");
    assert_eq!(short_id("abcdefg"), "abcdefg");
    assert_eq!(short_id("abcdefghij"), "abcdefg");
}

#[test]
fn format_relative_date_uses_three_hour_cutoff() {
    let now = 1_700_000_000;
    assert_eq!(format_relative_date(now, now), "just now");
    assert_eq!(format_relative_date(now - 59, now), "just now");
    assert_eq!(format_relative_date(now - 60, now), "1m");
    assert_eq!(format_relative_date(now - 3599, now), "59m");
    assert_eq!(format_relative_date(now - 3600, now), "1h");
    assert_eq!(format_relative_date(now - 3 * 3600, now), "3h");
    assert_eq!(
        format_relative_date(now - 3 * 3600 - 1, now),
        format_local_timestamp(now - 3 * 3600 - 1),
    );
    assert_eq!(
        format_relative_date(now - 86400, now),
        format_local_timestamp(now - 86400),
    );
    assert_eq!(format_relative_date(now + 10, now), "just now");
}

#[test]
fn format_utc_timestamp_stays_civil_utc() {
    let now = 1_700_000_000;
    assert_eq!(format_utc_timestamp(now), "2023-11-14 22:13");
    assert_eq!(format_utc_timestamp(now - 86400), "2023-11-13 22:13");
    assert_eq!(format_utc_timestamp(now - 3 * 3600 - 1), "2023-11-14 19:13");
}

#[test]
fn format_timestamp_at_offset_uses_asia_manila() {
    let unix = 1_700_000_000;
    assert_eq!(
        format_timestamp_at_offset(unix, 8 * 3600),
        "2023-11-15 06:13"
    );
    assert_eq!(
        format_timestamp_at_offset(unix - 86400, 8 * 3600),
        "2023-11-14 06:13"
    );
    assert_eq!(
        format_timestamp_at_offset(unix - 3 * 3600 - 1, 8 * 3600),
        "2023-11-15 03:13"
    );
}

#[test]
fn format_local_timestamp_matches_libc_civil() {
    let unix = 1_700_000_000;
    match local_civil(unix) {
        Some((year, month, day, hour, min, _)) => {
            assert_eq!(
                format_local_timestamp(unix),
                format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}")
            );
        }
        None => assert_eq!(format_local_timestamp(unix), format_utc_timestamp(unix)),
    }
}

#[test]
fn format_utc_offset_signs_and_pads() {
    assert_eq!(format_utc_offset(7200), "UTC+02:00");
    assert_eq!(format_utc_offset(-19_800), "UTC-05:30");
    assert_eq!(format_utc_offset(0), "UTC+00:00");
}

#[test]
fn format_local_exact_adds_seconds_to_the_local_minute() {
    let unix = 1_700_000_042;
    let exact = format_local_exact(unix);
    assert_eq!(exact.len(), 19, "{exact}");
    assert!(exact.starts_with(&format_local_timestamp(unix)), "{exact}");
    // A whole-minute local offset keeps the seconds of the unix value.
    assert!(exact.ends_with(&format!(":{:02}", unix % 60)), "{exact}");
}

#[test]
fn pick_meta_columns_drops_hash_then_date_then_author() {
    let hash = "abcdefg";
    let date = "12d ";
    let author = "Ada Lovelace     ";
    assert_eq!(
        pick_meta_columns(80, hash, date, author, MIN_LEFT_KEEP),
        MetaCols {
            hash: true,
            date: true,
            author: true
        }
    );
    let with_all = meta_columns_text(
        hash,
        date,
        author,
        MetaCols {
            hash: true,
            date: true,
            author: true,
        },
    )
    .chars()
    .count();
    let tight = MIN_LEFT_KEEP + with_all - 1;
    let cols = pick_meta_columns(tight, hash, date, author, MIN_LEFT_KEEP);
    assert!(!cols.hash, "{cols:?}");
    let hash_dropped = meta_columns_text(
        hash,
        date,
        author,
        MetaCols {
            hash: false,
            date: true,
            author: true,
        },
    )
    .chars()
    .count();
    let tighter = MIN_LEFT_KEEP + hash_dropped - 1;
    let cols = pick_meta_columns(tighter, hash, date, author, MIN_LEFT_KEEP);
    assert!(!cols.hash && !cols.date, "{cols:?}");
    let date_dropped = meta_columns_text(
        hash,
        date,
        author,
        MetaCols {
            hash: false,
            date: false,
            author: true,
        },
    )
    .chars()
    .count();
    let tightest = MIN_LEFT_KEEP + date_dropped - 1;
    let cols = pick_meta_columns(tightest, hash, date, author, MIN_LEFT_KEEP);
    assert!(!cols.hash && !cols.date && !cols.author, "{cols:?}");
}

#[test]
fn format_sync_ascii_uses_caret_and_v() {
    let sync = SyncState {
        branch: "main".into(),
        status: SyncStatus::Diverged,
        ahead: 2,
        behind: 3,
    };
    assert_eq!(format_sync(&sync, &ASCII), "main ^2 v3");
}

fn sample_commit_row() -> GraphRow {
    GraphRow::Commit {
        commit: Commit {
            id: "aaa1111bbbb".into(),
            subject: "add graph crate".into(),
            refs: vec!["main".into(), "origin/main".into()],
            author_name: "Ada Lovelace".into(),
            author_date_unix: 1_700_000_000 - 3600,
            ..Commit::default()
        },
        is_head: true,
        worktrees: vec![Worktree {
            path: ".worktrees/feature/graph".into(),
            head_id: Some("aaa1111bbbb".into()),
            branch: Some("feature/graph".into()),
            hidden: false,
            is_current: false,
        }],
    }
}

#[test]
fn format_row_commit_is_subject_only() {
    let row = sample_commit_row();
    assert_eq!(format_row(&row, &UNICODE), "⊙ add graph crate");
    assert_eq!(format_label(&row, &UNICODE), "add graph crate");
}

#[test]
fn format_commit_spacer_puts_refs_hash_date_author() {
    let GraphRow::Commit {
        commit,
        is_head,
        worktrees,
    } = sample_commit_row()
    else {
        panic!("commit row");
    };
    let line = format_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head,
        worktrees: &worktrees,
        head_branch: Some("main"),
        glyphs: &UNICODE,
        available: 120,
        date_width: 4,
        author_width: 12,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    assert!(
        line.contains("[main]"),
        "checkout + synced main chip: {line}"
    );
    assert!(
        !line.contains("origin/main"),
        "merged origin/main should fold into main: {line}"
    );
    assert!(line.contains("aaa1111"), "{line}");
    assert!(line.contains("1h"), "{line}");
    assert!(line.contains("Ada Lovelace"), "{line}");
    assert!(line.contains(".worktrees/feature/graph"), "{line}");
    let hash_at = line.find("aaa1111").expect("hash");
    let ref_at = line.find("[main]").expect("ref");
    assert!(ref_at < hash_at, "refs left of meta: {line}");
}

#[test]
fn spacer_meta_tags_date_and_author_parts() {
    let GraphRow::Commit {
        commit,
        is_head,
        worktrees,
    } = sample_commit_row()
    else {
        panic!("commit row");
    };
    let (text, parts) = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head,
        worktrees: &worktrees,
        head_branch: Some("main"),
        glyphs: &UNICODE,
        available: 120,
        date_width: 4,
        author_width: 12,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    assert_eq!(parts_text(&parts), text);
    let kind_of = |text: &str| {
        parts
            .iter()
            .find(|p| p.text == text)
            .map(|p| p.kind)
            .unwrap_or_else(|| panic!("{text}: {parts:?}"))
    };
    assert_eq!(kind_of("aaa1111"), LabelKind::MetaHash { parent: None });
    assert_eq!(kind_of("1h"), LabelKind::MetaDate);
    assert_eq!(kind_of("Ada Lovelace"), LabelKind::MetaAuthor);
    // The pads and the leading blanks stay plain meta.
    assert!(
        parts
            .iter()
            .filter(|p| p.text.trim().is_empty())
            .all(|p| p.kind == LabelKind::Meta),
        "{parts:?}"
    );
}

#[test]
fn meta_columns_text_unchanged_by_parts() {
    let cols = |hash, date, author| MetaCols { hash, date, author };
    for (c, author, want) in [
        (cols(true, true, true), "Ada", " abcdefg 12d  Ada"),
        (cols(false, true, true), "Ada", " 12d  Ada"),
        (cols(false, false, true), "Ada", " Ada"),
        (cols(true, true, true), "", " abcdefg 12d "),
        (cols(false, false, false), "Ada", ""),
    ] {
        assert_eq!(meta_columns_text("abcdefg", "12d ", author, c), want);
        let parts = meta_columns_parts("abcdefg", "12d ", author, c);
        assert_eq!(parts_text(&parts), want, "{parts:?}");
        assert!(parts.iter().all(|p| p.target.is_none()), "{parts:?}");
    }
}

#[test]
fn format_commit_spacer_keeps_unmatched_origin_ref() {
    let commit = Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs: vec!["origin/feature/x".into()],
        ..Commit::default()
    };
    let line = format_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: Some("main"),
        glyphs: &UNICODE,
        available: 80,
        date_width: 4,
        author_width: 1,
        now_unix: 0,
        default_branch_override: None,
    });
    assert!(line.contains("[origin/feature/x]"), "{line}");
}

#[test]
fn format_commit_spacer_drops_hash_before_refs() {
    let commit = Commit {
        id: "abcdefghhhh".into(),
        subject: "topic".into(),
        refs: vec!["feature/long-branch-name".into()],
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Commit::default()
    };
    let line = format_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available: 20,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    assert!(
        line.contains("[feature") || line.contains("feature/") || line.contains('+'),
        "keep refs or [+N] when dropping hash: {line}"
    );
    assert!(
        !line.contains("abcdefg"),
        "hash should drop before refs: {line}"
    );
}

fn sample_stash() -> Stash {
    Stash {
        id: "s1abcdefhhhh".into(),
        stash_ref: "stash@{0}".into(),
        subject: "WIP on main".into(),
        author_name: "Ada Lovelace".into(),
        author_date_unix: 1_700_000_000 - 86400,
        parent_id: Some("aaa1111".into()),
        ..Stash::default()
    }
}

#[test]
fn format_uncommitted_label_is_dirty_or_clean() {
    let dirty = GraphRow::Uncommitted { has_changes: true };
    let clean = GraphRow::Uncommitted { has_changes: false };
    assert_eq!(format_row(&dirty, &UNICODE), "○ uncommitted changes");
    assert_eq!(format_row(&clean, &UNICODE), "○ working tree clean");
    assert_eq!(format_label(&clean, &ASCII), "o working tree clean");
}

#[test]
fn format_label_stash_is_subject_only() {
    let row = GraphRow::Stash(sample_stash());
    assert_eq!(format_label(&row, &UNICODE), "WIP on main");
    assert_eq!(format_row(&row, &UNICODE), "◇ stash@{0}  WIP on main");
}

#[test]
fn format_stash_spacer_puts_ref_hash_date_author() {
    let stash = sample_stash();
    let line = format_stash_spacer(StashSpacerOpts {
        stash: &stash,
        available: 80,
        date_width: 16,
        author_width: 12,
        now_unix: 1_700_000_000,
    });
    assert!(line.contains("stash@{0}"), "{line}");
    assert!(line.contains("s1abcde"), "{line}");
    assert!(
        line.contains(&format_local_timestamp(1_700_000_000 - 86400)),
        "{line}"
    );
    assert!(line.contains("Ada Lovelace"), "{line}");
    let ref_at = line.find("stash@{0}").expect("ref");
    let hash_at = line.find("s1abcde").expect("hash");
    assert!(ref_at < hash_at, "stash@{{n}} left of meta: {line}");
}

#[test]
fn format_stash_spacer_drops_hash_before_ref() {
    let stash = Stash {
        id: "abcdefghhhh".into(),
        stash_ref: "stash@{0}".into(),
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Stash::default()
    };
    let line = format_stash_spacer(StashSpacerOpts {
        stash: &stash,
        available: 20,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
    });
    assert!(
        !line.contains("abcdefg"),
        "hash should drop before stash@{{n}}: {line}"
    );
}

#[test]
fn detached_head_keeps_head_chip() {
    let chips = format_commit_ref_chips(
        &[GraphRef::local("main")],
        true,
        Some("HEAD (detached)"),
        &ASCII,
    );
    assert!(chips.starts_with("[HEAD]"), "{chips}");
    assert!(chips.contains("[main]"), "{chips}");
}

#[test]
fn named_head_uses_checkout_mark_not_head_chip() {
    let chips = format_commit_ref_chips(&[GraphRef::local("main")], true, Some("main"), &ASCII);
    assert!(!chips.contains("[HEAD]"), "{chips}");
    assert_eq!(chips, "[+main]");
}

#[test]
fn checked_out_ref_chips_sort_before_default_branch() {
    let refs = [
        GraphRef::local("main"),
        GraphRef::local("zzz"),
        GraphRef::tag("v1"),
    ];
    assert_eq!(
        format_commit_ref_chips(&refs, true, Some("zzz"), &ASCII),
        "[+zzz] [main] [v1]"
    );
    assert_eq!(
        format_commit_ref_chips(&refs, true, Some("main"), &ASCII),
        "[+main] [zzz] [v1]"
    );
    assert_eq!(
        format_commit_ref_chips(&refs, false, Some("zzz"), &ASCII),
        "[main] [zzz] [v1]"
    );
}

#[test]
fn merged_named_head_is_one_chip() {
    let refs = [GraphRef::local("main"), GraphRef::remote("origin/main")];
    assert_eq!(
        format_commit_ref_chips(&refs, true, Some("main"), &ASCII),
        "[+=main]"
    );
    assert_eq!(
        format_commit_ref_chips(&refs, true, Some("main"), &UNICODE),
        "[main]"
    );
    let parts = commit_ref_chip_parts(&refs, true, Some("main"), &UNICODE, None);
    let mark_parts: Vec<&str> = parts
        .iter()
        .filter(|p| p.kind == LabelKind::ChipHead || p.kind == LabelKind::ChipRemote)
        .filter(|p| p.text != "[" && p.text != "]" && p.text != "main")
        .map(|p| p.text.as_str())
        .collect();
    assert_eq!(
        mark_parts,
        [""],
        "checkout+sync must be one run, not two icons: {parts:?}"
    );
}

#[test]
fn current_worktree_head_spacer_uses_footer_chip_not_path() {
    let commit = Commit {
        id: "aaa1111bbbb".into(),
        subject: "tip".into(),
        refs: vec![GraphRef::local("main"), GraphRef::remote("origin/main")],
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Commit::default()
    };
    let worktrees = [
        Worktree {
            path: ".worktrees/recon".into(),
            head_id: Some(commit.id.clone()),
            branch: Some("main".into()),
            hidden: false,
            is_current: true,
        },
        Worktree {
            path: ".worktrees/other".into(),
            head_id: Some(commit.id.clone()),
            branch: Some("main".into()),
            hidden: false,
            is_current: false,
        },
    ];
    let (line, _) = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: true,
        worktrees: &worktrees,
        head_branch: Some("main"),
        glyphs: &UNICODE,
        available: 120,
        date_width: 4,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    let chip = format_commit_ref_chips(&commit.refs, true, Some("main"), &UNICODE);
    assert_eq!(chip, "[main]");
    assert!(
        line.contains(&chip),
        "spacer must use the footer chip: {line}"
    );
    assert!(
        !line.contains(".worktrees"),
        "worktree path must not be a second chip: {line}"
    );
    assert!(
        !line.contains(UNICODE.worktree),
        "worktree glyph must not prefix the footer chip: {line}"
    );
}

#[test]
fn format_row_hidden_worktree() {
    let row = GraphRow::Worktree(Worktree {
        path: "notes".into(),
        head_id: None,
        branch: None,
        hidden: true,
        is_current: false,
    });
    assert_eq!(format_row(&row, &UNICODE), " notes  [hidden]");
    assert_eq!(format_row(&row, &ASCII), "L notes  [hidden]");
    assert!(!format_row(&row, &UNICODE).contains("🔗"));
}

#[test]
fn label_parts_split_the_row_glyph_and_keep_the_label_text() {
    let worktree = |hidden, is_current, branch: Option<&str>| {
        GraphRow::Worktree(Worktree {
            path: "notes".into(),
            head_id: None,
            branch: branch.map(Into::into),
            hidden,
            is_current,
        })
    };
    let rows = [
        GraphRow::Uncommitted { has_changes: true },
        GraphRow::Uncommitted { has_changes: false },
        worktree(true, false, None),
        worktree(false, true, Some("feat")),
        worktree(true, true, Some("feat")),
        GraphRow::Stash(Stash {
            subject: "WIP on main".into(),
            ..Stash::default()
        }),
        GraphRow::Commit {
            commit: Commit {
                subject: "add graph".into(),
                ..Commit::default()
            },
            is_head: false,
            worktrees: Vec::new(),
        },
    ];
    for glyphs in [&UNICODE, &ASCII] {
        for row in &rows {
            let parts = format_label_parts(row, glyphs);
            assert_eq!(parts_text(&parts), format_label(row, glyphs), "{row:?}");
            let icons: Vec<_> = parts.iter().filter_map(|part| part.kind.icon()).collect();
            match row {
                GraphRow::Uncommitted { has_changes } => {
                    assert_eq!(parts[0].text, glyphs.uncommitted);
                    assert_eq!(
                        parts[0].kind,
                        LabelKind::UncommittedMark {
                            dirty: *has_changes
                        }
                    );
                    assert_eq!(icons, vec![(crate::GraphIconKind::Uncommitted, None)]);
                }
                GraphRow::Worktree(wt) => {
                    assert_eq!(parts[0].text, glyphs.worktree);
                    assert_eq!(icons, vec![(crate::GraphIconKind::Worktree, Some(0))]);
                    let kind_of = |text: &str| {
                        parts
                            .iter()
                            .find(|part| part.text.contains(text))
                            .map(|part| part.kind)
                    };
                    assert_eq!(kind_of("notes"), Some(LabelKind::Subject));
                    assert_eq!(kind_of("[hidden]"), wt.hidden.then_some(LabelKind::Meta));
                    assert_eq!(
                        kind_of("[HEAD]"),
                        wt.is_current.then_some(LabelKind::ChipHead)
                    );
                }
                _ => {
                    assert!(icons.is_empty());
                    assert!(parts.iter().all(|part| part.kind == LabelKind::Subject));
                }
            }
        }
    }
}

#[test]
fn commit_spacer_worktree_marks_name_their_worktree() {
    let commit = Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs: vec!["feat".into()],
        ..Commit::default()
    };
    let wt = |path: &str, branch: &str, hidden: bool| Worktree {
        path: path.into(),
        head_id: Some("abcdefg".into()),
        branch: Some(branch.into()),
        hidden,
        is_current: false,
    };
    // `feat` is already a ref chip, so only `b` and `c` paint a mark.
    let worktrees = [
        wt("a", "feat", false),
        wt("b", "other", false),
        wt("c", "notes", true),
    ];
    let (text, parts) = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &worktrees,
        head_branch: None,
        glyphs: &ASCII,
        available: 80,
        date_width: 0,
        author_width: 0,
        now_unix: 0,
        default_branch_override: None,
    });
    assert!(
        text.starts_with("L b other L c notes  [hidden] [feat]"),
        "{text}"
    );
    let icons: Vec<_> = parts.iter().filter_map(|part| part.kind.icon()).collect();
    assert_eq!(
        icons,
        vec![
            (crate::GraphIconKind::Worktree, Some(1)),
            (crate::GraphIconKind::Worktree, Some(2)),
            // The hash column (no date or author column at width 0).
            (crate::GraphIconKind::CommitHash, None),
        ]
    );
    for text in [" b other", " c notes", "  [hidden]"] {
        assert!(
            parts
                .iter()
                .any(|part| part.text == text && part.kind == LabelKind::Meta),
            "{text}: {parts:?}"
        );
    }
}

#[test]
fn format_commit_spacer_hides_overflow_chips_with_count() {
    let commit = Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs: vec![
            "main".into(),
            "feature/long-name".into(),
            GraphRef::tag("v1"),
        ],
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Commit::default()
    };
    let line = format_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available: 28,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    assert!(line.contains("[main]"), "kept first chip: {line}");
    assert!(line.contains("[+2]"), "exact overflow chip: {line}");
    assert!(
        !line.contains("feature/long-name"),
        "overflow chip must not remain: {line}"
    );
    assert!(
        !line.contains("[v1]"),
        "overflow tag must not remain: {line}"
    );
    assert!(
        !line.contains('…'),
        "no leftover room to truncate the next name: {line}"
    );
    let overflow = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available: 28,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    })
    .1;
    assert!(
        overflow
            .iter()
            .any(|p| p.kind == LabelKind::Overflow && p.text == "[+2]"),
        "overflow run must be LabelKind::Overflow: {overflow:?}"
    );
}

#[test]
fn overflow_chip_counts_hidden_tags() {
    let commit = Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs: vec![
            GraphRef::tag("v1.0.0"),
            GraphRef::tag("v1.1.0"),
            GraphRef::tag("release-candidate"),
        ],
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Commit::default()
    };
    let line = format_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available: 24,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    assert!(line.contains("[+"), "tag overflow must show [+N]: {line}");
    let fully_visible = usize::from(line.contains("[v1.0.0]"))
        + usize::from(line.contains("[v1.1.0]"))
        + usize::from(line.contains("[release-candidate]"));
    assert!(
        fully_visible < 3,
        "at least one tag must not fit in full: {line}"
    );
    let overflow = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available: 24,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    })
    .1
    .into_iter()
    .find(|p| p.kind == LabelKind::Overflow)
    .expect("overflow chip");
    let truncated = usize::from(line.contains('…'));
    let hidden = 3 - fully_visible - truncated;
    assert!(
        hidden > 0 && overflow.text == overflow_chip_text(hidden),
        "overflow counts fully hidden tags only (truncated does not): {line} overflow={}",
        overflow.text
    );
}

#[test]
fn overflow_chip_counts_branches_and_tags_together() {
    let commit = Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs: vec![
            GraphRef::local("main"),
            GraphRef::local("feature/x"),
            GraphRef::tag("v2"),
            GraphRef::tag("nightly"),
        ],
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 60,
        ..Commit::default()
    };
    let (line, parts) = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available: 22,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    let overflow = parts
        .iter()
        .find(|p| p.kind == LabelKind::Overflow)
        .expect("overflow chip");
    assert!(
        overflow.text.starts_with("[+") && overflow.text.ends_with(']'),
        "overflow must be a chip: {line}"
    );
    assert!(
        !line.contains("[feature/x]") || !line.contains("[v2]") || !line.contains("[nightly]"),
        "some branch/tag chips must hide: {line}"
    );
    assert!(line.contains("[main]"), "kept default branch: {line}");
}

#[test]
fn worktree_marks_stay_visible_when_refs_overflow() {
    let commit = Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs: vec![
            GraphRef::local("main"),
            GraphRef::local("feature/x"),
            GraphRef::tag("v2"),
        ],
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 60,
        ..Commit::default()
    };
    let worktrees = [Worktree {
        path: "notes".into(),
        head_id: Some(commit.id.clone()),
        branch: None,
        hidden: false,
        is_current: false,
    }];
    let (line, parts) = assemble_commit_spacer(CommitSpacerOpts {
        commit: &commit,
        is_head: false,
        worktrees: &worktrees,
        head_branch: None,
        glyphs: &ASCII,
        available: 28,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    });
    assert!(
        line.contains("notes"),
        "worktree mark stays visible: {line}"
    );
    assert!(
        parts.iter().any(|p| p.kind == LabelKind::Overflow),
        "hidden branch/tag refs still show [+N]: {line}"
    );
    assert!(
        !line.contains("[feature/x]") || !line.contains("[v2]"),
        "some refs hide: {line}"
    );
    // The worktree mark keeps its own identity; `[+N]` names exactly
    // the chips it hides.
    let overflow = parts
        .iter()
        .find(|p| p.kind == LabelKind::Overflow)
        .expect("overflow part");
    let Some(PartTarget::Overflow(hidden)) = &overflow.target else {
        panic!("overflow target: {overflow:?}");
    };
    assert_eq!(overflow.text, overflow_chip_text(hidden.len()));
    let shown: Vec<&PartTarget> = parts.iter().filter_map(|p| p.target.as_ref()).collect();
    for chip in hidden {
        assert!(!shown.contains(&chip), "{chip:?} is hidden, not shown");
    }
    assert!(parts
        .iter()
        .any(|p| p.kind == LabelKind::WorktreeMark { worktree: 0 } && p.target.is_none()));
}

#[test]
fn chip_runs_name_their_refs_and_marks_stay_one_run() {
    let refs = [
        GraphRef::local("main"),
        GraphRef::remote("origin/main"),
        GraphRef::local("feat"),
        GraphRef::remote("origin/side"),
        GraphRef::tag("v1"),
    ];
    let parts = commit_ref_chip_parts(&refs, true, Some("main"), &ASCII, None);
    assert_eq!(
        parts_text(&parts),
        format_commit_ref_chips(&refs, true, Some("main"), &ASCII),
        "targets do not change the text"
    );
    let target_of = |text: &str| {
        parts
            .iter()
            .find(|p| p.text == text)
            .and_then(|p| p.target.clone())
    };
    assert_eq!(
        target_of("main"),
        Some(PartTarget::Chip(vec![
            GraphRef::local("main"),
            GraphRef::remote("origin/main")
        ]))
    );
    let marks: Vec<&LabelPart> = parts
        .iter()
        .filter(|p| matches!(p.target, Some(PartTarget::Marks { .. })))
        .collect();
    assert_eq!(marks.len(), 1, "checkout + sync is one run: {parts:?}");
    assert_eq!(marks[0].text, "+=");
    assert_eq!(
        marks[0].target,
        Some(PartTarget::Marks {
            branch: "main".into(),
            checkout: true,
            remote: Some("origin/main".into()),
        })
    );
    assert_eq!(
        marks[0].icon(),
        Some((crate::GraphIconKind::ChipMarks, None))
    );
    assert_eq!(
        target_of("feat"),
        Some(PartTarget::Chip(vec![GraphRef::local("feat")]))
    );
    assert_eq!(
        target_of("[origin/side]"),
        Some(PartTarget::Chip(vec![GraphRef::remote("origin/side")]))
    );
    assert_eq!(
        target_of("[v1]"),
        Some(PartTarget::Chip(vec![GraphRef::tag("v1")]))
    );
    // Every chip run carries a target; only the gaps are plain.
    for part in &parts {
        assert_eq!(part.target.is_none(), part.text == " ", "{part:?}");
    }
    let icon = |text: &str| {
        parts
            .iter()
            .find(|p| p.text == text)
            .and_then(LabelPart::icon)
            .map(|(kind, _)| kind)
    };
    assert_eq!(icon("main"), Some(crate::GraphIconKind::DefaultChip));
    assert_eq!(icon("feat"), Some(crate::GraphIconKind::LocalChip));
    assert_eq!(
        icon("[origin/side]"),
        Some(crate::GraphIconKind::RemoteChip)
    );
    assert_eq!(icon("[v1]"), Some(crate::GraphIconKind::TagChip));
    let detached = commit_ref_chip_parts(&refs, true, None, &ASCII, None);
    assert_eq!(detached[0].text, "[HEAD]");
    assert_eq!(detached[0].target, Some(PartTarget::DetachedHead));
}

#[test]
fn chip_targets_survive_truncation_overflow_and_slicing() {
    let commit = topic_commit(vec![
        GraphRef::local("main"),
        GraphRef::local("feature/long-name"),
        GraphRef::tag("v1"),
        GraphRef::tag("v2"),
    ]);
    let (line, parts) = ascii_spacer(&commit, 32);
    let truncated = parts
        .iter()
        .find(|p| p.text.ends_with('…'))
        .unwrap_or_else(|| panic!("truncated chip name: {line}"));
    let feature = Some(PartTarget::Chip(vec![GraphRef::local("feature/long-name")]));
    assert_eq!(truncated.target, feature);
    let overflow = parts
        .iter()
        .find(|p| p.kind == LabelKind::Overflow)
        .unwrap_or_else(|| panic!("[+N]: {line}"));
    assert_eq!(overflow.text, "[+2]");
    assert_eq!(
        overflow.target,
        Some(PartTarget::Overflow(vec![
            PartTarget::Chip(vec![GraphRef::tag("v1")]),
            PartTarget::Chip(vec![GraphRef::tag("v2")]),
        ]))
    );
    // Slicing (pan) and truncation (footer) keep every target.
    let sliced = slice_label_parts(&parts, 3, 20);
    for part in &sliced {
        let whole = parts
            .iter()
            .find(|p| p.text.contains(part.text.as_str()) && p.kind == part.kind)
            .expect("sliced from a part");
        assert_eq!(part.target, whole.target, "{part:?}");
    }
    let cut = trunc_label_parts(&parts, 10);
    assert_eq!(cut.last().map(|p| p.text.as_str()), Some("…"));
    assert_eq!(cut.last().and_then(|p| p.target.clone()), None);
    assert!(cut
        .iter()
        .any(|p| p.target == Some(PartTarget::Chip(vec![GraphRef::local("main")]))));
    // A lone chip too narrow for its name still truncates with a target.
    let lone = topic_commit(vec![GraphRef::tag("release-candidate")]);
    let (_, lone_parts) = ascii_spacer(&lone, 18);
    let tag = lone_parts
        .iter()
        .find(|p| p.kind == LabelKind::ChipTag)
        .expect("tag chip");
    assert!(tag.text.ends_with("…]"), "{tag:?}");
    assert_eq!(
        tag.target,
        Some(PartTarget::Chip(vec![GraphRef::tag("release-candidate")]))
    );
}

#[test]
fn sync_header_parts_colour_the_marks_and_keep_the_text() {
    let sync = |status, ahead, behind| SyncState {
        branch: "main".into(),
        status,
        ahead,
        behind,
    };
    for (state, glyphs) in [
        (sync(SyncStatus::Diverged, 2, 3), &ASCII),
        (sync(SyncStatus::Ahead, 1, 0), &UNICODE),
        (sync(SyncStatus::NoUpstream, 0, 0), &UNICODE),
        (sync(SyncStatus::UpToDate, 0, 0), &ASCII),
    ] {
        let parts = format_sync_parts(&state, glyphs, None);
        assert_eq!(parts_text(&parts), format_sync(&state, glyphs));
        assert_eq!(parts[0].kind, LabelKind::ChipDefault);
    }
    let parts = format_sync_parts(&sync(SyncStatus::Diverged, 2, 3), &UNICODE, None);
    let kinds: Vec<(&str, LabelKind)> = parts.iter().map(|p| (p.text.as_str(), p.kind)).collect();
    assert_eq!(
        kinds,
        vec![
            ("main", LabelKind::ChipDefault),
            (" ", LabelKind::Meta),
            ("\u{f062}2", LabelKind::SyncAhead),
            (" ", LabelKind::Meta),
            ("\u{f063}3", LabelKind::SyncBehind),
        ]
    );
    let feature = SyncState {
        branch: "feat".into(),
        ..sync(SyncStatus::NoUpstream, 0, 0)
    };
    let parts = format_sync_parts(&feature, &ASCII, None);
    assert_eq!(parts[0].kind, LabelKind::ChipLocal);
    assert_eq!(parts_text(&parts), "feat no-upstream");
}

#[test]
fn commit_spacer_never_exceeds_available_with_many_refs() {
    let refs: Vec<GraphRef> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                GraphRef::local(format!("feature/branch-{i:02}"))
            } else {
                GraphRef::tag(format!("v{i}.0.0-long"))
            }
        })
        .collect();
    let commit = Commit {
        id: "abcdefghhhh".into(),
        subject: "topic".into(),
        refs,
        author_name: "Ada Lovelace".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Commit::default()
    };
    for available in [8, 16, 24, 40, 64] {
        let (line, parts) = assemble_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: None,
            glyphs: &ASCII,
            available,
            date_width: 10,
            author_width: 12,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        assert!(
            line.chars().count() <= available,
            "spacer grew past pane: available={available} len={} line={line:?}",
            line.chars().count()
        );
        assert!(
            parts.iter().any(|p| p.kind == LabelKind::Overflow),
            "many refs must overflow: available={available} line={line}"
        );
        if let Some(at) = line.find('…') {
            let before = line[..at].rfind('[');
            let after = line[at..].find(']');
            assert!(
                before.is_some() && after.is_some(),
                "name ellipsis must stay inside chip brackets: {line}"
            );
        }
    }
}

fn topic_commit(refs: Vec<GraphRef>) -> Commit {
    Commit {
        id: "abcdefg".into(),
        subject: "topic".into(),
        refs,
        author_name: "Ada".into(),
        author_date_unix: 1_700_000_000 - 120,
        ..Commit::default()
    }
}

fn ascii_spacer(commit: &Commit, available: usize) -> (String, Vec<LabelPart>) {
    assemble_commit_spacer(CommitSpacerOpts {
        commit,
        is_head: false,
        worktrees: &[],
        head_branch: None,
        glyphs: &ASCII,
        available,
        date_width: 3,
        author_width: 3,
        now_unix: 1_700_000_000,
        default_branch_override: None,
    })
}

#[test]
fn format_commit_spacer_truncates_next_chip_name_keeps_brackets() {
    let commit = topic_commit(vec![
        GraphRef::local("main"),
        GraphRef::local("feature/long-name"),
    ]);
    let (line, parts) = ascii_spacer(&commit, 20);
    assert!(line.contains("[main]"), "kept first chip: {line}");
    assert!(
        line.contains('…'),
        "next chip name must truncate when part of it fits: {line}"
    );
    assert!(
        line.contains("…]"),
        "truncated chip keeps closing bracket: {line}"
    );
    assert!(
        !line.contains("[feature/long-name]"),
        "full second name must not remain: {line}"
    );
    assert!(
        parts.iter().all(|p| p.kind != LabelKind::Overflow),
        "truncated last chip is not hidden: {line} {parts:?}"
    );
    assert!(
        format_commit_ref_chips(&commit.refs, false, None, &ASCII).contains("[feature/long-name]"),
        "footer/full chips still list the complete name"
    );
}

#[test]
fn truncated_chip_does_not_count_toward_overflow_n() {
    let commit = topic_commit(vec![
        GraphRef::local("main"),
        GraphRef::local("feature/long-name"),
        GraphRef::tag("v1"),
    ]);
    let (line, parts) = ascii_spacer(&commit, 32);
    let overflow = parts
        .iter()
        .find(|p| p.kind == LabelKind::Overflow)
        .unwrap_or_else(|| panic!("expected [+N] after a truncated chip: {line}"));
    assert_eq!(
        overflow.text, "[+1]",
        "truncated name is not hidden: {line}"
    );
    assert!(
        line.contains('…'),
        "second chip should be truncated: {line}"
    );
    assert!(
        !line.contains("[feature/long-name]"),
        "full second name must not remain: {line}"
    );
    assert!(!line.contains("[v1]"), "third chip is fully hidden: {line}");
    assert!(line.contains("[main]"), "first chip stays: {line}");
}

#[test]
fn omit_overflow_chip_when_only_the_last_chip_is_truncated() {
    let commit = topic_commit(vec![GraphRef::local("feature/long-name")]);
    let (line, parts) = ascii_spacer(&commit, 18);
    assert!(
        line.contains('…') && line.contains("…]"),
        "lone chip truncates in place: {line}"
    );
    assert!(
        !line.contains("[feature/long-name]"),
        "full name must not remain when truncated: {line}"
    );
    assert!(
        parts.iter().all(|p| p.kind != LabelKind::Overflow),
        "nothing else is hidden: {line} {parts:?}"
    );
}

#[test]
fn format_commit_message_joins_subject_and_body() {
    assert_eq!(format_commit_message("fix login", ""), "fix login");
    assert_eq!(format_commit_message("", "body only"), "body only");
    assert_eq!(
        format_commit_message("fix login", "details\n"),
        "fix login\n\ndetails"
    );
}

#[test]
fn wrap_commit_message_hard_breaks_and_preserves_blank_lines() {
    let lines = wrap_commit_message("abcdefghij\n\nxyz", 4, 8);
    assert_eq!(lines, vec!["abcd", "efgh", "ij", "", "xyz"]);
    let capped = wrap_commit_message("abcdefghij", 4, 2);
    assert_eq!(capped.len(), 2);
    assert!(capped[1].ends_with('…'), "{capped:?}");
    assert!(!capped.join("").contains("ij"), "{capped:?}");
}

#[test]
fn wrap_commit_message_marks_lines_cut_at_a_newline() {
    let cut = wrap_commit_message("one\ntwo\nthree", 8, 2);
    assert_eq!(cut, vec!["one", "two…"]);
    let blank = wrap_commit_message("one\n\nthree", 8, 2);
    assert_eq!(blank, vec!["one", "…"]);
    let full_width = wrap_commit_message("abcd\nefgh\nij", 4, 2);
    assert_eq!(full_width, vec!["abcd", "efg…"]);
    let exact = wrap_commit_message("one\ntwo", 8, 2);
    assert_eq!(exact, vec!["one", "two"], "nothing left: no mark");
}
