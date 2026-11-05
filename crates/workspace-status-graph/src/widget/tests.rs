use super::*;
use crate::action::Action;
use crate::action::Effect;
use crate::chrome::{graph_chrome_budget, selection_detail_lines, GraphFooterSelection};
use crate::format::{format_local_timestamp, overflow_chip_text};
use crate::hex_color;
use crate::model::{Commit, GraphRef, GraphRow, Stash, SyncState, SyncStatus, Worktree};
use crate::paint::{paint_model, paint_model_with, PaintOpts};
use crate::topology::cells_text;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

const NOW: i64 = 1_700_000_000;

fn commit(id: &str, subject: &str, parents: &[&str]) -> Commit {
    Commit {
        id: id.into(),
        subject: subject.into(),
        parents: parents.iter().map(|p| (*p).to_string()).collect(),
        author_name: "Ada".into(),
        author_date_unix: NOW - 3600,
        ..Commit::default()
    }
}

fn worktree(path: &str, head: &str, hidden: bool) -> Worktree {
    Worktree {
        path: path.into(),
        head_id: Some(head.into()),
        branch: Some("feature/graph".into()),
        hidden,
        is_current: false,
    }
}

fn sample_model() -> GraphModel {
    let head = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let parent = "bbb2222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    GraphModel {
        commits: vec![
            Commit {
                id: head.into(),
                subject: "add graph crate".into(),
                parents: vec![parent.into()],
                refs: vec!["main".into()],
                author_name: "Ada Lovelace".into(),
                author_date_unix: NOW - 120,
                ..Commit::default()
            },
            commit(parent, "prior commit", &[]),
        ],
        stashes: vec![Stash {
            id: "ccc3333ccccccccccccccccccccccccccccccccc".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            body: String::new(),
            author_name: "Ada Lovelace".into(),
            author_date_unix: NOW - 86400,
            parent_id: Some(head.into()),
            ..Stash::default()
        }],
        worktrees: vec![
            worktree(".worktrees/feature/graph", head, false),
            worktree("notes", head, true),
        ],
        head_id: Some(head.into()),
        sync: Some(SyncState {
            branch: "main".into(),
            status: SyncStatus::Ahead,
            ahead: 1,
            behind: 0,
        }),
        uncommitted: Some(true),
        window: 2,
        ..GraphModel::default()
    }
}

fn merge_model() -> GraphModel {
    GraphModel {
        commits: vec![
            commit("m999", "merge", &["a111", "b222"]),
            commit("a111", "left", &["r000"]),
            commit("b222", "right", &["r000"]),
            commit("r000", "root", &[]),
        ],
        head_id: Some("m999".into()),
        ..GraphModel::default()
    }
}

fn two_parent_join_model() -> GraphModel {
    GraphModel {
        commits: vec![
            commit("mainTip", "main tip", &["base"]),
            commit("tipA", "tip A", &["base"]),
            commit("tipB", "tip B", &["base"]),
            commit("base", "shared parent", &[]),
        ],
        head_id: Some("mainTip".into()),
        ..GraphModel::default()
    }
}

fn render_lines(model: &GraphModel, width: u16, height: u16, ascii: bool) -> Vec<String> {
    render_lines_ex(model, width, height, ascii, None, false)
}

fn render_lines_ex(
    model: &GraphModel,
    width: u16,
    height: u16,
    ascii: bool,
    selected: Option<usize>,
    expand: bool,
) -> Vec<String> {
    render_lines_scrolled(model, width, height, ascii, selected, expand, 0)
}

fn render_lines_scrolled(
    model: &GraphModel,
    width: u16,
    height: u16,
    ascii: bool,
    selected: Option<usize>,
    expand: bool,
    msg_scroll: usize,
) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(model)
                .ascii(ascii)
                .now_unix(NOW)
                .selected(selected)
                .commit_msg_expand(expand)
                .commit_msg_scroll(msg_scroll)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..width {
                let symbol = buffer[(x, y)].symbol();
                if !symbol.is_empty() {
                    line.push_str(symbol);
                }
            }
            line.trim_end().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn expanded_footer_scrolls_a_message_taller_than_the_footer() {
    let body = (0..30)
        .map(|i| format!("L{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let commit = Commit {
        id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        subject: "subject".into(),
        body,
        author_name: "Ada".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    };
    let model = GraphModel {
        commits: vec![commit],
        uncommitted: None,
        window: 1,
        ..GraphModel::default()
    };
    // One-row meta: the 44-column meta line fits 48 columns.
    let top = render_lines_scrolled(&model, 48, 20, true, Some(0), true, 0);
    let footer = &top[top.len() - 5..];
    assert!(footer[0].starts_with("subject"), "{top:#?}");
    assert!(footer[3].starts_with("L01"), "{top:#?}");
    assert!(footer[4].contains("aaa1111"), "meta pinned: {top:#?}");
    assert!(!top.join("\n").contains("L02"), "{top:#?}");
    assert!(
        footer[0].ends_with('█'),
        "scrollbar thumb at the top: {top:#?}"
    );

    let bottom = render_lines_scrolled(&model, 48, 20, true, Some(0), true, 999);
    let footer = &bottom[bottom.len() - 5..];
    assert!(footer[0].starts_with("L26"), "clamped to max: {bottom:#?}");
    assert!(footer[3].starts_with("L29"), "{bottom:#?}");
    assert!(footer[4].contains("aaa1111"), "meta pinned: {bottom:#?}");
    assert!(
        footer[3].ends_with('█'),
        "scrollbar thumb at the bottom: {bottom:#?}"
    );
}

/// 40 linear commits: `commit 0` has a 1-line message, `commit 1` a
/// 30-line body (taller than any N). No uncommitted row, so visible row
/// `i` is commit `i`.
fn fixed_footer_model() -> GraphModel {
    let mut model = tall_linear_model(40);
    model.uncommitted = None;
    model.commits[1].body = (0..30)
        .map(|i| format!("L{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    model
}

struct FooterRender {
    selected: usize,
    expand: bool,
    msg_scroll: usize,
    msg_lines: Option<usize>,
    ascii: bool,
}

fn render_footer(model: &GraphModel, width: u16, height: u16, opts: FooterRender) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            let mut widget = GraphWidget::new(model)
                .ascii(opts.ascii)
                .now_unix(NOW)
                .selected(Some(opts.selected))
                .commit_msg_expand(opts.expand)
                .commit_msg_scroll(opts.msg_scroll);
            if let Some(n) = opts.msg_lines {
                widget = widget.commit_msg_lines(n);
            }
            widget.render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// Rows from the top that paint an ASCII gutter cell (`*` / `@` node or
/// `|` rail) in column 1: the painted list height of a linear model that
/// overflows the list.
fn painted_list_rows(buf: &Buffer) -> Vec<u16> {
    (0..buf.area.height)
        .take_while(|&y| matches!(buf[(1, y)].symbol(), "*" | "@" | "|"))
        .collect()
}

#[test]
fn expanded_footer_height_does_not_follow_the_selected_message() {
    let model = fixed_footer_model();
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    let opts = |selected| FooterRender {
        selected,
        expand: true,
        msg_scroll: 0,
        msg_lines: None,
        ascii: true,
    };
    let short = render_footer(&model, width, height, opts(0));
    let long = render_footer(&model, width, height, opts(1));
    let short_list = painted_list_rows(&short);
    let long_list = painted_list_rows(&long);
    // Default N=4: footer is the rule, 4 message rows, and meta.
    assert_eq!(short_list.len(), usize::from(height - 6), "{short_list:?}");
    assert_eq!(
        short_list, long_list,
        "list height follows N, not the message"
    );
    let footer_top = height - 5;
    assert!(buf_row(&short, footer_top).starts_with("commit 0"));
    assert!(buf_row(&long, footer_top).starts_with("commit 1"));
    assert!(
        buf_row(&short, height - 1).contains("c00aaaa"),
        "meta on bottom row"
    );
    assert!(
        buf_row(&long, height - 1).contains("c01aaaa"),
        "meta on bottom row"
    );

    // A different N moves the split the same way for both messages.
    let n3 = |selected| FooterRender {
        msg_lines: Some(3),
        ..opts(selected)
    };
    let short3 = painted_list_rows(&render_footer(&model, width, height, n3(0)));
    let long3 = painted_list_rows(&render_footer(&model, width, height, n3(1)));
    assert_eq!(short3.len(), usize::from(height - 5));
    assert_eq!(short3, long3);

    // N is clamped to 1..=20.
    let n0 = painted_list_rows(&render_footer(
        &model,
        width,
        60,
        FooterRender {
            msg_lines: Some(0),
            ..opts(0)
        },
    ));
    assert_eq!(
        n0.len(),
        60 - 3,
        "N=0 clamps to rule + 1 message row + meta"
    );
    let n99 = painted_list_rows(&render_footer(
        &model,
        width,
        60,
        FooterRender {
            msg_lines: Some(99),
            ..opts(0)
        },
    ));
    assert_eq!(
        n99.len(),
        60 - 22,
        "N=99 clamps to rule + 20 message rows + meta"
    );
}

#[test]
fn expanded_footer_pins_meta_under_blank_rows_for_a_short_message() {
    let model = fixed_footer_model();
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    let buf = render_footer(
        &model,
        width,
        height,
        FooterRender {
            selected: 0,
            expand: true,
            msg_scroll: 0,
            msg_lines: None,
            ascii: true,
        },
    );
    let top = height - 5;
    assert_eq!(buf_row(&buf, top).trim_end(), "commit 0");
    for y in top + 1..height - 1 {
        assert_eq!(buf_row(&buf, y).trim(), "", "blank padding row {y}");
    }
    assert!(buf_row(&buf, height - 1).contains("c00aaaa"));
}

#[test]
fn collapsed_footer_is_two_rows_for_any_message() {
    let model = fixed_footer_model();
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    for selected in [0, 1] {
        let buf = render_footer(
            &model,
            width,
            height,
            FooterRender {
                selected,
                expand: false,
                msg_scroll: 0,
                msg_lines: Some(12),
                ascii: true,
            },
        );
        assert_eq!(painted_list_rows(&buf).len(), usize::from(height - 3));
        assert_eq!(buf_row(&buf, height - 3), "_".repeat(usize::from(width)));
        assert!(buf_row(&buf, height - 2).starts_with(&format!("commit {selected}")));
    }
}

#[test]
fn footer_rule_tops_the_footer_in_the_caller_glyph_and_colour() {
    let model = fixed_footer_model();
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    let rule_color = Color::Rgb(1, 2, 3);
    let draw = |ascii: bool, rule: Option<&'static str>, expand: bool| {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                let mut widget = GraphWidget::new(&model)
                    .ascii(ascii)
                    .now_unix(NOW)
                    .selected(Some(0))
                    .commit_msg_expand(expand);
                if let Some(glyph) = rule {
                    widget = widget.footer_rule(glyph, rule_color);
                }
                widget.render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    };
    for (expand, footer_h) in [(true, 6u16), (false, 3)] {
        let rule_y = height - footer_h;
        let custom = draw(false, Some("\u{2581}"), expand);
        assert_eq!(
            buf_row(&custom, rule_y),
            "\u{2581}".repeat(usize::from(width)),
            "expand {expand}"
        );
        for x in 0..width {
            assert_eq!(custom[(x, rule_y)].fg, rule_color, "x={x}");
        }
        assert!(
            buf_row(&custom, rule_y + 1).starts_with("commit 0"),
            "body starts under the rule"
        );
        assert!(buf_row(&custom, height - 1).contains("c00aaaa"));
        // The last list row sits right above the rule.
        assert!(!buf_row(&custom, rule_y - 1).trim().is_empty());

        let ascii = draw(true, Some("_"), expand);
        assert_eq!(buf_row(&ascii, rule_y), "_".repeat(usize::from(width)));
        let ascii_default = draw(true, None, expand);
        assert_eq!(
            buf_row(&ascii_default, rule_y),
            "_".repeat(usize::from(width))
        );
        let unicode_default = draw(false, None, expand);
        assert_eq!(
            buf_row(&unicode_default, rule_y),
            "\u{2581}".repeat(usize::from(width))
        );
    }
}

#[test]
fn short_pane_footer_keeps_the_rule_and_the_list_row() {
    let model = fixed_footer_model();
    let buf = render_footer(
        &model,
        20,
        3,
        FooterRender {
            selected: 0,
            expand: true,
            msg_scroll: 0,
            msg_lines: None,
            ascii: true,
        },
    );
    assert_eq!(painted_list_rows(&buf), vec![0]);
    assert_eq!(buf_row(&buf, 1), "_".repeat(20));
    assert!(buf_row(&buf, 2).contains("c00aaaa"), "meta stays pinned");
}

#[test]
fn no_footer_paints_no_rule() {
    let model = fixed_footer_model();
    for height in [1u16, 2] {
        let buf = render_footer(
            &model,
            20,
            height,
            FooterRender {
                selected: 0,
                expand: false,
                msg_scroll: 0,
                msg_lines: None,
                ascii: true,
            },
        );
        for y in 0..height {
            assert!(!buf_row(&buf, y).contains("___"), "{height}: row {y}");
        }
    }
}

#[test]
fn overflowing_footer_hints_hidden_lines_below() {
    let model = fixed_footer_model();
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    let render = |msg_scroll, ascii| {
        render_footer(
            &model,
            width,
            height,
            FooterRender {
                selected: 1,
                expand: true,
                msg_scroll,
                msg_lines: None,
                ascii,
            },
        )
    };
    // Message: subject, blank, L00..L29 = 32 lines in 4 rows.
    let last_msg_row = height - 2;
    let bar_x = width - 1;
    let hint_at = |buf: &Buffer, text: &str| {
        let row = buf_row(buf, last_msg_row);
        let cols: Vec<&str> = (0..width)
            .map(|x| buf[(x, last_msg_row)].symbol())
            .collect();
        let start = usize::from(bar_x) - text.chars().count();
        assert_eq!(cols[start..usize::from(bar_x)].concat(), text, "{row:?}");
    };
    let top = render(0, false);
    hint_at(&top, "\u{f103}28");
    assert!(buf_row(&top, last_msg_row).starts_with("L01"));
    let mid = render(10, false);
    hint_at(&mid, "\u{f103}18");
    let ascii = render(10, true);
    hint_at(&ascii, "v18");
    assert!(!buf_row(&ascii, last_msg_row).contains('\u{f103}'));
    let end = render(999, false);
    let end_row = buf_row(&end, last_msg_row);
    assert!(end_row.starts_with("L29"), "{end_row:?}");
    assert!(
        !end_row.contains('\u{f103}'),
        "no hint at the end: {end_row:?}"
    );
    // The hint stays inside the footer: meta still on the bottom row.
    assert!(buf_row(&top, height - 1).contains("c01aaaa"));
    // A short message has no hint.
    let short = render_footer(
        &model,
        width,
        height,
        FooterRender {
            selected: 0,
            expand: true,
            msg_scroll: 0,
            msg_lines: None,
            ascii: false,
        },
    );
    assert!((0..height).all(|y| !buf_row(&short, y).contains('\u{f103}')));
}

/// A message of exactly N lines fits: no hint, no footer scrollbar.
/// One more line overflows: `<more_below>1` hint and the footer scrollbar.
#[test]
fn footer_hint_and_scrollbar_start_at_n_plus_one_lines() {
    let n = COMMIT_MSG_LINES_DEFAULT;
    let mut model = tall_linear_model(40);
    model.uncommitted = None;
    let body = |count: usize| {
        (0..count)
            .map(|i| format!("B{i:02}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Subject + blank + body: commit 2 is exactly N lines, commit 3 N+1.
    model.commits[2].body = body(n - 2);
    model.commits[3].body = body(n - 1);
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    let render = |selected| {
        render_footer(
            &model,
            width,
            height,
            FooterRender {
                selected,
                expand: true,
                msg_scroll: 0,
                msg_lines: None,
                ascii: false,
            },
        )
    };
    let top = height - 1 - n as u16;
    let bar_x = width - 1;
    let last_msg_row = height - 2;
    let bar_painted =
        |buf: &Buffer| (top..height - 1).any(|y| buf[(bar_x, y)].symbol().trim() != "");

    let exact = render(2);
    let exact_last = buf_row(&exact, last_msg_row);
    assert!(
        exact_last.starts_with(&format!("B{:02}", n - 3)),
        "{exact_last:?}"
    );
    assert!(!exact_last.contains('\u{f103}'), "{exact_last:?}");
    assert!(!bar_painted(&exact), "exactly N lines: no footer scrollbar");

    let over = render(3);
    let over_last = buf_row(&over, last_msg_row);
    assert!(
        over_last.starts_with(&format!("B{:02}", n - 3)),
        "{over_last:?}"
    );
    let hint_start = usize::from(bar_x) - "\u{f103}1".chars().count();
    let cols: Vec<&str> = (0..width)
        .map(|x| over[(x, last_msg_row)].symbol())
        .collect();
    assert_eq!(cols[hint_start..usize::from(bar_x)].concat(), "\u{f103}1");
    assert!(bar_painted(&over), "N+1 lines: footer scrollbar");
    assert!(
        buf_row(&over, height - 1).contains("c03aaaa"),
        "meta pinned"
    );
}

#[test]
fn expanded_footer_paints_body_collapsed_hides_it() {
    let body = "UNIQUE_GRAPH_BODY";
    let tail = "TAILTOKEN";
    let commit = Commit {
        id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        subject: format!("{}{tail}", "n".repeat(40)),
        body: body.into(),
        author_name: "Ada".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    };
    let model = GraphModel {
        commits: vec![commit],
        uncommitted: None,
        window: 1,
        ..GraphModel::default()
    };
    let collapsed = render_lines_ex(&model, 28, 12, true, Some(0), false).join("\n");
    assert!(
        !collapsed.contains(body),
        "collapsed footer hides body:\n{collapsed}"
    );
    assert!(
        !collapsed.contains(tail),
        "collapsed footer clips subject:\n{collapsed}"
    );
    let expanded = render_lines_ex(&model, 28, 12, true, Some(0), true).join("\n");
    assert!(
        expanded.contains(body),
        "expanded footer shows body:\n{expanded}"
    );
    assert!(
        expanded.contains(tail),
        "expanded footer shows subject tail:\n{expanded}"
    );
}

#[test]
fn paints_head_sync_stash_and_worktree() {
    let lines = render_lines(&sample_model(), 120, 16, false);
    let joined = lines.join("\n");
    assert!(joined.contains("main \u{f062}1"), "sync header: {joined}");
    assert!(
        joined.contains("○ uncommitted changes"),
        "uncommitted: {joined}"
    );
    assert!(joined.contains("◇"), "stash diamond: {joined}");
    assert!(joined.contains("stash@{0}"), "stash ref: {joined}");
    assert!(joined.contains("WIP on main"), "stash subject: {joined}");
    assert!(joined.contains("aaa1111"), "{joined}");
    assert!(
        joined.contains("[main]") || joined.contains("[main]"),
        "HEAD branch chip: {joined}"
    );
    assert!(joined.contains(".worktrees/feature/graph"), "{joined}");
    assert!(
        joined.contains(""),
        "linked worktree uses ICON_LINKED_WORKTREE: {joined}"
    );
    assert!(!joined.contains("🔗"), "emoji desyncs the gutter: {joined}");
    assert!(joined.contains("bbb2222"), "{joined}");
    assert!(joined.contains("prior commit"), "{joined}");
}

#[test]
fn stash_sits_above_parent_commit() {
    let lines = render_lines(&sample_model(), 120, 16, false);
    let stash = lines
        .iter()
        .position(|l| l.contains("stash@{0}"))
        .expect("stash row");
    let head = lines
        .iter()
        .position(|l| l.contains("add graph crate"))
        .expect("head row");
    assert!(stash < head, "stash {stash} should sit above HEAD {head}");
}

#[test]
fn commit_paints_subject_then_refs_hash_date_author() {
    let model = sample_model();
    let lines = render_lines(&model, 120, 16, false);
    let subject = lines
        .iter()
        .position(|l| l.contains("add graph crate"))
        .expect("subject row");
    assert!(
        !lines[subject].contains("aaa1111"),
        "hash must not sit on the subject: {}",
        lines[subject]
    );
    assert!(
        !lines[subject].contains("[main]") && !lines[subject].contains("[main]"),
        "refs must not sit on the subject: {}",
        lines[subject]
    );
    let meta = &lines[subject + 1];
    assert!(meta.contains("aaa1111"), "short hash on spacer: {meta}");
    assert!(
        meta.contains("[main]") || meta.contains("[main]"),
        "branch chip on spacer: {meta}"
    );
    let painted = paint_model_with(
        &model,
        &UNICODE,
        PaintOpts {
            now_unix: Some(NOW),
            line_width: Some(200),
            ..PaintOpts::default()
        },
    );
    let subject_line = painted
        .iter()
        .find(|l| l.label.contains("add graph crate"))
        .expect("subject painted");
    assert!(subject_line.selectable);
    let spacer = painted
        .iter()
        .skip_while(|l| !l.label.contains("add graph crate"))
        .nth(1)
        .expect("commit spacer");
    assert!(!spacer.selectable);
    assert_eq!(spacer.row_index, subject_line.row_index);
    assert!(spacer.label.contains("aaa1111"), "{}", spacer.label);
    assert!(spacer.label.contains("2m"), "{}", spacer.label);
    assert!(spacer.label.contains("Ada Lovelace"), "{}", spacer.label);
}

#[test]
fn stash_paints_subject_then_ref_hash_date_author() {
    let model = sample_model();
    let lines = render_lines(&model, 120, 16, false);
    let subject = lines
        .iter()
        .position(|l| l.contains("WIP on main"))
        .expect("stash subject row");
    assert!(
        !lines[subject].contains("ccc3333"),
        "hash must not sit on the subject: {}",
        lines[subject]
    );
    assert!(
        !lines[subject].contains("stash@{0}"),
        "stash@{{n}} must not sit on the subject: {}",
        lines[subject]
    );
    let meta = &lines[subject + 1];
    assert!(meta.contains("stash@{0}"), "stash ref on spacer: {meta}");
    assert!(meta.contains("ccc3333"), "short hash on spacer: {meta}");
    let painted = paint_model_with(
        &model,
        &UNICODE,
        PaintOpts {
            now_unix: Some(NOW),
            line_width: Some(200),
            ..PaintOpts::default()
        },
    );
    let subject_line = painted
        .iter()
        .find(|l| l.selectable && l.label.contains("WIP on main"))
        .expect("stash subject painted");
    assert!(subject_line.selectable);
    let spacer = painted
        .iter()
        .skip_while(|l| !(l.selectable && l.label.contains("WIP on main")))
        .nth(1)
        .expect("stash spacer");
    assert!(!spacer.selectable);
    assert_eq!(spacer.row_index, subject_line.row_index);
    assert!(spacer.label.contains("stash@{0}"), "{}", spacer.label);
    assert!(spacer.label.contains("ccc3333"), "{}", spacer.label);
    assert!(
        spacer.label.contains(&format_local_timestamp(NOW - 86400)),
        "{}",
        spacer.label
    );
    assert!(spacer.label.contains("Ada Lovelace"), "{}", spacer.label);
    assert!(
        !subject_line.label.contains("stash@{0}"),
        "node is subject-only: {}",
        subject_line.label
    );
}

#[test]
fn stash_leaf_sits_on_spur_not_fake_lane() {
    let model = sample_model();
    let lines = render_lines(&model, 120, 16, false);
    let joined = lines.join("\n");
    let painted = paint_model(&model, &UNICODE, None);
    let stash_line = painted
        .iter()
        .find(|l| l.selectable && l.label.contains("WIP on main"))
        .expect("stash painted");
    let diamond_idx = stash_line
        .gutter
        .iter()
        .position(|c| c.ch == "◇")
        .expect("◇ in gutter");
    assert!(
        diamond_idx >= crate::CELL_W,
        "◇ must sit on a spur off stash^1, not lane 0: {}",
        cells_text(&stash_line.gutter)
    );
    assert_eq!(
        stash_line.gutter[diamond_idx].color_lane,
        Some(0),
        "spur colour is stash^1 lane, not a fake lane palette"
    );
    let spacer = painted
        .iter()
        .skip_while(|l| !(l.selectable && l.label.contains("WIP on main")))
        .nth(1)
        .expect("stash spacer");
    assert_eq!(
        spacer.gutter[diamond_idx].ch,
        UNICODE.vertical,
        "spacer must carry the short spur, got {}",
        cells_text(&spacer.gutter)
    );
    assert!(!spacer.selectable);
    assert_eq!(spacer.row_index, stash_line.row_index);
    assert!(spacer.label.contains("stash@{0}"), "{}", spacer.label);
    assert!(
        joined.contains('╯') || joined.contains('╰'),
        "join elbow on stash^1: {joined}"
    );
    assert!(
        !stash_line.label.contains('◇'),
        "diamond is gutter-only: {}",
        stash_line.label
    );
}

#[test]
fn paints_merge_open_and_join() {
    let lines = render_lines(&merge_model(), 80, 16, false);
    let joined = lines.join("\n");
    assert!(
        joined.contains('╮') || joined.contains('╭'),
        "open: {joined}"
    );
    assert!(
        joined.contains('╯') || joined.contains('╰'),
        "join: {joined}"
    );
    assert!(joined.contains("merge"), "{joined}");
    assert!(joined.contains("left"), "{joined}");
    assert!(joined.contains("right"), "{joined}");
    let painted = paint_model(&merge_model(), &UNICODE, None);
    let merge = painted
        .iter()
        .find(|l| l.label.contains("merge"))
        .expect("merge row");
    let gutter = cells_text(&merge.gutter);
    assert!(gutter.contains('╮'), "merge open corner: {gutter}");
    let root = painted
        .iter()
        .find(|l| l.label.contains("root"))
        .expect("root row");
    let root_g = cells_text(&root.gutter);
    assert!(root_g.contains('╯'), "root join: {root_g}");
}

#[test]
fn paints_two_parent_join() {
    let lines = render_lines(&two_parent_join_model(), 80, 16, false);
    let joined = lines.join("\n");
    assert!(joined.contains("tip A"), "{joined}");
    assert!(joined.contains("tip B"), "{joined}");
    let painted = paint_model(&two_parent_join_model(), &UNICODE, None);
    let base = painted
        .iter()
        .find(|l| l.label.contains("shared parent"))
        .expect("base row");
    let gutter = cells_text(&base.gutter);
    assert!(
        gutter.contains('╯') || gutter.contains('┴'),
        "two-parent join: {gutter}"
    );
    let tip_a = painted.iter().find(|l| l.label.contains("tip A")).unwrap();
    let tip_b = painted.iter().find(|l| l.label.contains("tip B")).unwrap();
    let lane_a = tip_a
        .gutter
        .iter()
        .position(|c| c.role == crate::CellRole::Node)
        .unwrap();
    let lane_b = tip_b
        .gutter
        .iter()
        .position(|c| c.role == crate::CellRole::Node)
        .unwrap();
    assert_ne!(lane_a, lane_b, "sibling tips must keep distinct lanes");
}

#[test]
fn hidden_worktree_is_omitted() {
    let lines = render_lines(&sample_model(), 120, 16, false);
    let joined = lines.join("\n");
    assert!(
        !joined.contains("notes"),
        "hidden worktree leaked: {joined}"
    );
    assert!(!joined.contains("[hidden]"), "hidden mark leaked: {joined}");
}

#[test]
fn show_hidden_includes_hidden_worktree() {
    let mut model = sample_model();
    assert_eq!(model.dispatch(Action::SetShowHidden(true)), Effect::None);
    let lines = render_lines(&model, 120, 16, false);
    let joined = lines.join("\n");
    assert!(joined.contains("notes"), "expected notes path: {joined}");
    assert!(
        joined.contains("[hidden]"),
        "expected hidden mark: {joined}"
    );
}

#[test]
fn toggle_show_hidden_flips_visibility() {
    let mut model = sample_model();
    assert!(!model.show_hidden);
    assert_eq!(model.dispatch(Action::ToggleShowHidden), Effect::None);
    assert!(model.show_hidden);
    let shown = model.visible_rows();
    assert!(
        shown.iter().any(|row| match row {
            crate::GraphRow::Commit { worktrees, .. } => {
                worktrees.iter().any(|wt| wt.path == "notes")
            }
            _ => false,
        }),
        "toggle should attach the hidden worktree to HEAD"
    );
    model.dispatch(Action::ToggleShowHidden);
    assert!(!model.show_hidden);
    let hidden = model.visible_rows();
    assert!(hidden.iter().all(|row| match row {
        crate::GraphRow::Commit { worktrees, .. } => {
            worktrees.iter().all(|wt| wt.path != "notes")
        }
        crate::GraphRow::Worktree(wt) => wt.path != "notes",
        _ => true,
    }));
}

#[test]
fn ascii_glyphs_replace_unicode_nodes() {
    let lines = render_lines(&sample_model(), 120, 16, true);
    let joined = lines.join("\n");
    assert!(joined.contains("main ^1"), "{joined}");
    assert!(joined.contains("o uncommitted changes"), "{joined}");
    assert!(joined.contains('s'), "{joined}");
    assert!(joined.contains("stash@{0}"), "{joined}");
    assert!(joined.contains("@"), "{joined}");
    assert!(joined.contains("aaa1111"), "{joined}");
    assert!(joined.contains("*"), "{joined}");
    assert!(joined.contains("bbb2222"), "{joined}");
    assert!(joined.contains("L .worktrees/feature/graph"), "{joined}");
    assert!(!joined.contains("🔗"), "emoji desyncs the gutter: {joined}");
    assert!(!joined.contains("⊙"), "{joined}");
    assert!(!joined.contains("●"), "{joined}");
    let merge_lines = render_lines(&merge_model(), 80, 16, true);
    let merge_joined = merge_lines.join("\n");
    assert!(
        merge_joined.contains('\\') || merge_joined.contains('/'),
        "ASCII merge corners: {merge_joined}"
    );
    assert!(merge_joined.contains('*'), "{merge_joined}");
}

#[test]
fn empty_model_renders_no_rows() {
    let lines = render_lines(&GraphModel::default(), 20, 2, false);
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn gutter_cells_use_distinct_lane_colors() {
    let model = merge_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let colors = crate::default_lane_colors();
    let mut seen = std::collections::HashSet::new();
    for y in 0..16u16 {
        for x in 0..20u16 {
            let cell = &buffer[(x, y)];
            if cell.symbol() == " " || cell.symbol().is_empty() {
                continue;
            }
            if let Color::Rgb(r, g, b) = cell.fg {
                if colors.iter().any(
                    |c| matches!(c, Color::Rgb(cr, cg, cb) if *cr == r && *cg == g && *cb == b),
                ) {
                    seen.insert((r, g, b));
                }
            }
        }
    }
    assert!(
        seen.len() >= 2,
        "expected at least two lane colours on the merge gutter, got {seen:?}"
    );
}

#[test]
fn paints_two_line_selection_footer() {
    let model = sample_model();
    let lines = render_lines(&model, 80, 16, false);
    let joined = lines.join("\n");
    assert!(
        joined.contains("no selection")
            || joined.contains("Uncommitted changes")
            || joined.contains("add graph crate"),
        "selection footer: {joined}"
    );
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut last = String::new();
    let mut prev = String::new();
    for y in 0..16u16 {
        let mut line = String::new();
        for x in 0..80u16 {
            line.push_str(buffer[(x, y)].symbol());
        }
        let trimmed = line.trim_end().to_string();
        if !trimmed.is_empty() {
            prev = last;
            last = trimmed;
        }
    }
    assert!(
        prev.contains("Uncommitted changes"),
        "footer subject: {prev}"
    );
    assert!(
        last.contains("main"),
        "uncommitted footer lists HEAD refs: {last}"
    );
    assert!(
        !last.contains("not a commit"),
        "HEAD has refs so fallback is unused: {last}"
    );
}

#[test]
fn paints_clean_working_tree_row() {
    let mut model = sample_model();
    model.uncommitted = Some(false);
    let joined = render_lines(&model, 120, 16, false).join("\n");
    assert!(
        joined.contains("○ working tree clean"),
        "always-on clean row: {joined}"
    );
    assert!(
        !joined.contains("○ uncommitted changes"),
        "dirty label must not appear when clean: {joined}"
    );
}

#[test]
fn paints_stash_selection_footer() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(1))
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut last = String::new();
    let mut prev = String::new();
    for y in 0..16u16 {
        let mut line = String::new();
        for x in 0..80u16 {
            line.push_str(buffer[(x, y)].symbol());
        }
        let trimmed = line.trim_end().to_string();
        if !trimmed.is_empty() {
            prev = last;
            last = trimmed;
        }
    }
    assert!(prev.contains("WIP on main"), "stash footer subject: {prev}");
    assert!(
        last.contains("stash@{0}")
            && last.contains("ccc3333")
            && last.contains(&format_local_timestamp(NOW - 86400)),
        "stash footer meta ref · hash · date: {last}"
    );
    assert!(!last.contains("Ada"), "stash footer has no author: {last}");
}

#[test]
fn search_match_paints_bg_on_selectable_row_not_cursor() {
    let model = sample_model();
    let rows = model.visible_rows();
    let stash_idx = rows
        .iter()
        .position(|row| matches!(row, GraphRow::Stash(_)))
        .expect("stash row");
    let bg = Color::Rgb(187, 154, 247);
    let fg = Color::Rgb(26, 27, 38);
    let matches = [stash_idx];
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .search_matches(&matches, bg, fg)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut stash_line_y = None;
    let mut stash_spacer_y = None;
    for y in 0..16u16 {
        let mut line = String::new();
        for x in 0..80u16 {
            line.push_str(buffer[(x, y)].symbol());
        }
        if line.contains("WIP on main") {
            stash_line_y = Some(y);
            stash_spacer_y = Some(y.saturating_add(1));
            break;
        }
    }
    let y = stash_line_y.expect("stash subject line");
    let mut saw_match_bg = false;
    for x in 0..80u16 {
        if buffer[(x, y)].bg == bg {
            saw_match_bg = true;
            break;
        }
    }
    assert!(
        saw_match_bg,
        "search match should paint filter bg on the stash node"
    );
    // The label text paints in the search foreground, not its own colour.
    let label_x = (0..78u16)
        .find(|&x| {
            (x..x + 3)
                .map(|cx| buffer[(cx, y)].symbol())
                .collect::<String>()
                == "WIP"
        })
        .expect("stash subject column");
    assert_eq!(buffer[(label_x, y)].fg, fg, "match label fg");
    assert_eq!(buffer[(label_x, y)].bg, bg, "match label bg");
    if let Some(spacer_y) = stash_spacer_y {
        let spacer_has_match = (0..80u16).any(|x| buffer[(x, spacer_y)].bg == bg);
        assert!(
            !spacer_has_match,
            "stash spacer is not a searchMatchIds target"
        );
    }
}

#[test]
fn search_match_paints_comment_mark_in_search_fg() {
    let model = sample_model();
    let rows = model.visible_rows();
    let stash_idx = rows
        .iter()
        .position(|row| matches!(row, GraphRow::Stash(_)))
        .expect("stash row");
    let prior_idx = rows
            .iter()
            .position(|row| {
                matches!(row, GraphRow::Commit { commit, .. } if commit.subject == "prior commit")
            })
            .expect("prior commit row");
    // Cursor fg equals the search bg (a theme can share one accent).
    let bg = Color::Rgb(189, 147, 249);
    let fg = Color::Rgb(40, 42, 54);
    let cursor_fg = bg;
    let matches = [stash_idx];
    let commented = [stash_idx, prior_idx];
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .search_matches(&matches, bg, fg)
                .commented_rows(&commented)
                .cursor_style(cursor_fg, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let line_y = |needle: &str| {
        (0..16u16)
            .find(|&y| {
                (0..80u16)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap_or_else(|| panic!("line with {needle}"))
    };
    let mark_x = |y: u16| {
        (0..80u16)
            .find(|&x| buffer[(x, y)].symbol() == "\"")
            .unwrap_or_else(|| panic!("comment mark on row {y}"))
    };
    let match_y = line_y("WIP on main");
    let match_x = mark_x(match_y);
    assert_eq!(buffer[(match_x, match_y)].fg, fg, "match mark fg");
    assert_eq!(buffer[(match_x, match_y)].bg, bg, "match mark bg");
    let plain_y = line_y("prior commit");
    let plain_x = mark_x(plain_y);
    assert_eq!(
        buffer[(plain_x, plain_y)].fg,
        cursor_fg,
        "non-match commented mark keeps the cursor fg"
    );
}

#[test]
fn flash_paints_node_and_spacer_cursor_wins() {
    let model = sample_model();
    let rows = model.visible_rows();
    let stash_idx = rows
        .iter()
        .position(|row| matches!(row, GraphRow::Stash(_)))
        .expect("stash row");
    let flash = Color::Rgb(61, 82, 54);
    let cursor_bg = Color::Rgb(40, 52, 87);
    let flashes = [(stash_idx, flash)];
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(stash_idx))
                .flash_rows(&flashes)
                .cursor_style(Color::Cyan, cursor_bg)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut stash_line_y = None;
    let mut stash_spacer_y = None;
    for y in 0..16u16 {
        let mut line = String::new();
        for x in 0..80u16 {
            line.push_str(buffer[(x, y)].symbol());
        }
        if line.contains("WIP on main") {
            stash_line_y = Some(y);
            stash_spacer_y = Some(y.saturating_add(1));
            break;
        }
    }
    let y = stash_line_y.expect("stash subject line");
    assert!(
        (0..80u16).any(|x| buffer[(x, y)].bg == flash),
        "flash background paints the selected node"
    );
    assert_eq!(
        buffer[(0, y)].symbol(),
        "▌",
        "cursor bar stays on the selected node"
    );
    if let Some(spacer_y) = stash_spacer_y {
        assert!(
            (0..80u16).any(|x| buffer[(x, spacer_y)].bg == flash),
            "flash background follows the selected stash spacer"
        );
    }

    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .flash_rows(&flashes)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut stash_line_y = None;
    let mut stash_spacer_y = None;
    for y in 0..16u16 {
        let mut line = String::new();
        for x in 0..80u16 {
            line.push_str(buffer[(x, y)].symbol());
        }
        if line.contains("WIP on main") {
            stash_line_y = Some(y);
            stash_spacer_y = Some(y.saturating_add(1));
            break;
        }
    }
    let y = stash_line_y.expect("stash subject line");
    assert!(
        (0..80u16).any(|x| buffer[(x, y)].bg == flash),
        "flash paints the stash node"
    );
    if let Some(spacer_y) = stash_spacer_y {
        assert!(
            (0..80u16).any(|x| buffer[(x, spacer_y)].bg == flash),
            "flash follows the stash spacer"
        );
    }
}

#[test]
fn graph_scrollbar_thumb_matches_painted_handle() {
    let mut commits = Vec::new();
    for i in 0..24 {
        let id = format!("c{i:02}{}", "a".repeat(36));
        let parents = if i + 1 < 24 {
            vec![format!("c{:02}{}", i + 1, "a".repeat(36))]
        } else {
            Vec::new()
        };
        commits.push(Commit {
            id,
            subject: format!("commit {i}"),
            parents,
            author_name: "Ada".into(),
            author_date_unix: NOW - 3600,
            ..Commit::default()
        });
    }
    let head = commits[0].id.clone();
    let model = GraphModel {
        commits,
        head_id: Some(head),
        uncommitted: Some(false),
        window: 24,
        ..GraphModel::default()
    };
    let width = 40u16;
    let height = 16u16;
    let scroll = 8u16;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .ascii(true)
                .scroll(scroll)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let chrome = graph_chrome_budget(height, false);
    let list_top = u16::from(chrome.header);
    let painted = paint_model_with(
        &model,
        &ASCII,
        PaintOpts {
            now_unix: Some(NOW),
            line_width: Some(width.saturating_sub(2) as usize),
            ..PaintOpts::default()
        },
    );
    let (thumb_off, thumb_len) =
        graph_scrollbar_thumb(painted.len(), scroll, chrome.list_height).expect("thumb");
    let sb_x = width.saturating_sub(1);
    for i in 0..chrome.list_height {
        let y = list_top.saturating_add(i);
        let cell = buffer[(sb_x, y)].symbol();
        let on_thumb = i >= thumb_off && i < thumb_off.saturating_add(thumb_len);
        if on_thumb {
            assert_eq!(cell, "█", "thumb cell at y={y} off={i}");
        } else {
            assert_ne!(cell, "█", "track must not paint thumb at y={y} off={i}");
        }
    }
}

fn tall_linear_model(n: usize) -> GraphModel {
    let mut commits = Vec::new();
    for i in 0..n {
        let id = format!("c{i:02}{}", "a".repeat(36));
        let parents = if i + 1 < n {
            vec![format!("c{:02}{}", i + 1, "a".repeat(36))]
        } else {
            Vec::new()
        };
        commits.push(Commit {
            id,
            subject: format!("commit {i}"),
            parents,
            author_name: "Ada".into(),
            author_date_unix: NOW - 3600,
            ..Commit::default()
        });
    }
    let head = commits[0].id.clone();
    GraphModel {
        commits,
        head_id: Some(head),
        uncommitted: Some(false),
        window: n,
        ..GraphModel::default()
    }
}

fn render_graph(
    model: &GraphModel,
    width: u16,
    height: u16,
    scroll: u16,
    col_offset: u16,
) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(model)
                .ascii(true)
                .scroll(scroll)
                .col_offset(col_offset)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn vertical_scrollbar_shows_at_top_when_the_list_overflows() {
    let model = tall_linear_model(24);
    let width = 40u16;
    let height = 16u16;
    let chrome = graph_chrome_budget(height, false);
    let sb_x = width.saturating_sub(1);
    let painted = paint_model_with(
        &model,
        &ASCII,
        PaintOpts {
            now_unix: Some(NOW),
            line_width: Some(width.saturating_sub(2) as usize),
            ..PaintOpts::default()
        },
    );
    assert!(painted.len() > usize::from(chrome.list_height));
    for scroll in [0u16, 8] {
        let buf = render_graph(&model, width, height, scroll, 0);
        let (thumb_off, thumb_len) =
            graph_scrollbar_thumb(painted.len(), scroll, chrome.list_height).expect("thumb");
        let thumb_y = u16::from(chrome.header).saturating_add(thumb_off);
        assert_eq!(buf[(sb_x, thumb_y)].symbol(), "█", "scroll {scroll}");
        assert!(thumb_len >= 1);
    }
    // A list that fits paints no bar.
    let short = tall_linear_model(2);
    let fits = render_graph(&short, width, height, 0, 0);
    for i in 0..chrome.list_height {
        let y = u16::from(chrome.header).saturating_add(i);
        assert_ne!(fits[(sb_x, y)].symbol(), "█", "no thumb when it fits y={y}");
    }
    assert!(graph_vscroll_visible(10, 5, 0));
    assert!(!graph_vscroll_visible(5, 5, 0));
    assert!(graph_vscroll_visible(5, 5, 1));
}

#[test]
fn horizontal_scrollbar_hidden_at_left_shown_when_panned() {
    let marker = "UNIQUE_GRAPH_TAIL_xyz";
    let subject = format!("{}{marker}", "n".repeat(60));
    let model = GraphModel {
        commits: vec![commit(
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            &subject,
            &[],
        )],
        head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let width = 36u16;
    let height = 8u16;
    let chrome = graph_chrome_budget(height, false);
    let list_bottom = u16::from(chrome.header) + chrome.list_height;
    let h_y = list_bottom.saturating_sub(1);
    let at_left = render_graph(&model, width, height, 0, 0);
    let left_row: String = (0..width)
        .map(|x| at_left[(x, h_y)].symbol().to_string())
        .collect();
    assert!(
        !left_row.contains('█'),
        "no horizontal thumb at left edge: {left_row}"
    );
    let panned = render_graph(&model, width, height, 0, 50);
    let panned_row: String = (0..width)
        .map(|x| panned[(x, h_y)].symbol().to_string())
        .collect();
    assert!(
        panned_row.contains('█'),
        "panned viewport must paint a horizontal thumb: {panned_row}"
    );
}

#[test]
fn zero_area_does_not_panic() {
    let model = sample_model();
    let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
    GraphWidget::new(&model).render(buf.area, &mut buf);
}

#[test]
fn standalone_hidden_worktree_without_commit() {
    let mut model = GraphModel {
        worktrees: vec![Worktree {
            path: "vendor/secret".into(),
            head_id: None,
            branch: None,
            hidden: true,
            is_current: false,
        }],
        ..GraphModel::default()
    };
    assert!(model.visible_rows().is_empty());
    model.show_hidden = true;
    let rows = model.visible_rows();
    assert_eq!(rows.len(), 1);
    let lines = render_lines(&model, 40, 3, false);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("vendor/secret") && l.contains("[hidden]")),
        "{lines:?}"
    );
}

#[test]
fn capped_gutter_keeps_rail_columns_when_labels_clip() {
    let mut model = merge_model();
    for commit in &mut model.commits {
        commit.subject = format!("{} {}", commit.subject, "n".repeat(48));
    }
    let painted = paint_model_with(
        &model,
        &UNICODE,
        PaintOpts {
            gutter_width: Some(2),
            line_width: Some(24),
            now_unix: Some(NOW),
        },
    );
    let gutters: Vec<&[crate::GraphCell]> = painted
        .iter()
        .filter(|l| !l.gutter.is_empty())
        .map(|l| l.gutter.as_slice())
        .collect();
    assert!(gutters.len() >= 4, "merge rows");
    let width = gutters[0].len();
    assert_eq!(width, 2);
    assert!(
        gutters.iter().all(|g| g.len() == width),
        "shared clip width"
    );
    let left = painted
        .iter()
        .find(|l| l.selectable && l.label.contains("left"))
        .expect("left");
    let right = painted
        .iter()
        .find(|l| l.selectable && l.label.contains("right"))
        .expect("right");
    assert_eq!(left.gutter[0].role, crate::CellRole::Node);
    assert_eq!(
        right.gutter[0].ch,
        UNICODE.vertical,
        "side-lane node must not steal column 0: {}",
        cells_text(&right.gutter)
    );
    assert_ne!(right.gutter[0].role, crate::CellRole::Node);

    let width = 36u16;
    let height = 12u16;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .gutter_width(2)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let chrome = graph_chrome_budget(height, false);
    let list_bottom = u16::from(chrome.header) + chrome.list_height;
    let mut spine_x: Option<u16> = None;
    for y in u16::from(chrome.header)..list_bottom {
        for x in 0..width.saturating_sub(1) {
            let sym = buffer[(x, y)].symbol();
            if sym == UNICODE.vertical || sym == UNICODE.commit || sym == UNICODE.head_commit {
                match spine_x {
                    None => spine_x = Some(x),
                    Some(col) => {
                        assert_eq!(x, col, "rail/node at ({x},{y}) drifted from column {col}")
                    }
                }
                break;
            }
        }
    }
    assert!(spine_x.is_some(), "expected a spine column in the list");
    let joined = (0..height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..width {
                line.push_str(buffer[(x, y)].symbol());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !joined.contains("nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn"),
        "narrow pane must clip long subjects:\n{joined}"
    );
}

#[test]
fn gutter_width_caps_paint() {
    let model = merge_model();
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .gutter_width(2)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut line = String::new();
    for x in 0..40 {
        line.push_str(buffer[(x, 0)].symbol());
    }
    assert!(line.contains("merge"), "{line}");
}

#[test]
fn selected_row_paints_cursor_bar() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_bar = false;
    let mut saw_reversed = false;
    for y in 0..16u16 {
        for x in 0..80u16 {
            let cell = &buffer[(x, y)];
            if cell.symbol() == "▌" {
                saw_bar = true;
            }
            if cell.modifier.contains(Modifier::REVERSED) {
                saw_reversed = true;
            }
        }
    }
    assert!(saw_bar, "selected graph row should paint ▌");
    assert!(!saw_reversed, "graph cursor should not use reverse video");
}

/// Render `sample_model` with the commit `subject` selected. Returns the
/// bar column symbol of each `(node, spacer)` pair, keyed by subject.
fn commit_bar_cells(subject: &str, focused: bool) -> Vec<(String, String, String)> {
    let model = sample_model();
    let rows = model.visible_rows();
    let selected = rows
        .iter()
        .position(|row| matches!(row, GraphRow::Commit { commit, .. } if commit.subject == subject))
        .expect("selected commit row");
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(selected))
                .cursor_bar(focused)
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let line = |y: u16| -> String { (0..80u16).map(|x| buffer[(x, y)].symbol()).collect() };
    ["add graph crate", "prior commit", "WIP on main"]
        .iter()
        .map(|want| {
            let y = (0..30u16)
                .find(|&y| line(y).contains(want))
                .unwrap_or_else(|| panic!("{want} line"));
            (
                (*want).to_string(),
                buffer[(0, y)].symbol().to_string(),
                buffer[(0, y + 1)].symbol().to_string(),
            )
        })
        .collect()
}

#[test]
fn selected_commit_paints_focused_bar_on_node_and_spacer() {
    let cells = commit_bar_cells("add graph crate", true);
    assert_eq!(
        cells[0],
        ("add graph crate".into(), "▌".into(), "▌".into()),
        "focused bar on the selected node and its spacer"
    );
    assert_eq!(
        cells[1],
        ("prior commit".into(), " ".into(), " ".into()),
        "other commit node and spacer stay unmarked"
    );
    assert_eq!(
        cells[2],
        ("WIP on main".into(), " ".into(), " ".into()),
        "unselected stash node and spacer stay unmarked"
    );
}

#[test]
fn selected_commit_paints_inactive_bar_on_node_and_spacer() {
    let cells = commit_bar_cells("prior commit", false);
    assert_eq!(
        cells[1],
        ("prior commit".into(), "▏".into(), "▏".into()),
        "unfocused bar on the selected node and its spacer"
    );
    assert_eq!(
        cells[0],
        ("add graph crate".into(), " ".into(), " ".into()),
        "other commit node and spacer stay unmarked"
    );
}

#[test]
fn selected_stash_paints_bar_on_node_and_spacer() {
    let model = sample_model();
    let rows = model.visible_rows();
    let stash_idx = rows
        .iter()
        .position(|row| matches!(row, GraphRow::Stash(_)))
        .expect("stash row");
    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(stash_idx))
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let y = (0..30u16)
        .find(|&y| {
            (0..80u16)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("WIP on main")
        })
        .expect("stash line");
    assert_eq!(buffer[(0, y)].symbol(), "▌", "bar on the stash node");
    assert_eq!(buffer[(0, y + 1)].symbol(), "▌", "bar on the stash spacer");
    let other = (0..30u16)
        .find(|&y| {
            (0..80u16)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("add graph crate")
        })
        .expect("commit line");
    assert_eq!(
        buffer[(0, other)].symbol(),
        " ",
        "other commit node unmarked"
    );
    assert_eq!(
        buffer[(0, other + 1)].symbol(),
        " ",
        "other commit spacer unmarked"
    );
}

#[test]
fn selected_row_without_cursor_bar_paints_inactive_keeps_footer() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .cursor_bar(false)
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_bar = false;
    let mut saw_inactive = false;
    let mut last = String::new();
    let mut prev = String::new();
    for y in 0..16u16 {
        let mut line = String::new();
        for x in 0..80u16 {
            let cell = &buffer[(x, y)];
            if cell.symbol() == "▌" {
                saw_bar = true;
            }
            if cell.symbol() == "▏" {
                saw_inactive = true;
            }
            line.push_str(cell.symbol());
        }
        let trimmed = line.trim_end().to_string();
        if !trimmed.is_empty() {
            prev = last;
            last = trimmed;
        }
    }
    assert!(!saw_bar, "unfocused graph must not paint focused ▌");
    assert!(saw_inactive, "unfocused graph must paint inactive ▏");
    assert!(
        prev.contains("Uncommitted changes"),
        "selection footer subject stays: {prev}"
    );
    assert!(
        last.contains("main"),
        "selection footer still lists HEAD refs: {last}"
    );
}

#[test]
fn unselected_commented_row_paints_quote_mark() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .commented_rows(&[1, 2, 3, 4, 5])
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_quote = false;
    let mut quote_on_cursor = false;
    for y in 0..16u16 {
        let mut has_quote = false;
        let mut has_bar = false;
        for x in 0..80u16 {
            let sym = buffer[(x, y)].symbol();
            if sym == "\"" {
                has_quote = true;
                saw_quote = true;
            }
            if sym == "▌" {
                has_bar = true;
            }
        }
        if has_quote && has_bar {
            quote_on_cursor = true;
        }
    }
    assert!(saw_quote, "unselected commented graph row should paint \"");
    assert!(
        !quote_on_cursor,
        "unselected commented rows should not steal the cursor bar"
    );
}

/// One commit and one worktree row (its HEAD is outside the window).
fn badge_model() -> (GraphModel, usize) {
    let model = GraphModel {
        commits: vec![commit(
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "seed",
            &[],
        )],
        worktrees: vec![worktree("app/.worktrees/feat", "elsewhere", false)],
        ..GraphModel::default()
    };
    let worktree_row = model
        .visible_rows()
        .iter()
        .position(|row| matches!(row, GraphRow::Worktree(_)))
        .expect("worktree row");
    (model, worktree_row)
}

/// The [`GraphIconKind::Badge`] spans among `spans`.
fn badge_spans(spans: Vec<IconSpan>) -> Vec<IconSpan> {
    spans
        .into_iter()
        .filter(|span| span.kind == GraphIconKind::Badge)
        .collect()
}

fn render_badges(
    model: &GraphModel,
    width: u16,
    col_offset: u16,
    badges: &[(usize, &str, Color)],
) -> (Buffer, Vec<IconSpan>) {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 12));
    let spans = GraphWidget::new(model)
        .now_unix(NOW)
        .col_offset(col_offset)
        .row_badges(badges)
        .render_with_icon_spans(buf.area, &mut buf);
    (buf, badge_spans(spans))
}

fn buf_row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn row_badge_paints_at_the_right_edge_and_reports_its_cell() {
    let (model, worktree_row) = badge_model();
    let badges = [(worktree_row, "P", Color::Green)];
    let (plain, none) = render_badges(&model, 80, 0, &[]);
    assert!(none.is_empty());
    let (buf, spans) = render_badges(&model, 80, 0, &badges);
    assert_eq!(spans.len(), 1, "{spans:?}");
    let span = &spans[0];
    assert_eq!((span.row_index, span.width), (Some(worktree_row), 1));
    let line = buf_row(&buf, span.y);
    assert!(line.contains("app/.worktrees/feat feature/graph"), "{line}");
    // Flush right: the last column of the pane (no vertical bar here).
    assert_eq!(span.x, 79);
    assert_eq!(buf[(span.x, span.y)].symbol(), "P");
    assert_eq!(buf[(span.x, span.y)].fg, Color::Green);
    assert!(buf[(span.x, span.y)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(span.x - 1, span.y)].symbol(), " ");
    // Only the badge cell differs: gutter, label and other rows stay put.
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            if (x, y) != (span.x, span.y) {
                assert_eq!(buf[(x, y)], plain[(x, y)], "cell {x},{y}");
            }
        }
    }
}

#[test]
fn col_max_pans_the_label_end_into_view_left_of_the_badge() {
    let model = GraphModel {
        commits: vec![commit(
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "seed",
            &[],
        )],
        worktrees: vec![worktree(
            "app/.worktrees/a-worktree-path-longer-than-the-pane",
            "elsewhere",
            false,
        )],
        ..GraphModel::default()
    };
    let row = model
        .visible_rows()
        .iter()
        .position(|row| matches!(row, GraphRow::Worktree(_)))
        .expect("worktree row");
    let badges = [(row, "P", Color::Green)];
    let width = 40;
    let plain = graph_col_max(&model, false, width, false, &[]);
    let badged = graph_col_max(&model, false, width, false, &badges);
    assert!(plain > 0, "the worktree label overflows the pane");
    assert_eq!(badged, plain + 2, "the badge and space narrow the viewport");
    assert_eq!(
        graph_col_max(&model, false, width, false, &[(row + 9, "P", Color::Green)]),
        plain
    );
    // Panned to the max, the label end is in view left of the badge (the
    // limit counts the full gutter cap, so it may pan a little further).
    let (buf, spans) = render_badges(&model, width, badged as u16, &badges);
    assert_eq!(spans.len(), 1, "{spans:?}");
    let (x, y) = (spans[0].x, spans[0].y);
    assert_eq!(x, width - 1);
    assert_eq!(buf[(x, y)].symbol(), "P");
    assert_eq!(buf[(x - 1, y)].symbol(), " ");
    let line = buf_row(&buf, y);
    let label = line.strip_suffix('P').expect("badge last").trim_end();
    assert!(label.ends_with("feature/graph"), "{line}");
}

#[test]
fn row_badge_under_the_horizontal_bar_is_not_reported() {
    let (model, worktree_row) = badge_model();
    let badges = [(worktree_row, "P", Color::Green)];
    let width = 30;
    let max = graph_col_max(&model, false, width, false, &badges) as u16;
    assert!(max > 0, "the label overflows, so panning shows the bar");
    let render = |height: u16, col_offset: u16| {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let spans = GraphWidget::new(&model)
            .now_unix(NOW)
            .col_offset(col_offset)
            .row_badges(&badges)
            .render_with_icon_spans(buf.area, &mut buf);
        (buf, badge_spans(spans))
    };
    let mut covered = 0;
    for height in 3..16 {
        // Unpanned: no bar; find the worktree row.
        let (plain, _) = render(height, 0);
        let Some(y) = (0..height).find(|y| buf_row(&plain, *y).contains("app/")) else {
            continue;
        };
        // Panned to the max: the label tail and badge are in view unless
        // the bar paints over that row.
        let (buf, spans) = render(height, max);
        for span in &spans {
            assert_eq!(buf[(span.x, span.y)].symbol(), "P", "height {height}");
        }
        if buf_row(&buf, y).contains("graph") {
            assert_eq!(spans.len(), 1, "height {height}");
        } else {
            covered += 1;
            assert!(spans.is_empty(), "height {height}: {spans:?}");
        }
    }
    assert!(
        covered > 0,
        "some height puts the worktree row under the bar"
    );
}

#[test]
fn row_badge_skips_unlisted_rows() {
    let (model, worktree_row) = badge_model();
    let badges = [(worktree_row, "P", Color::Green)];
    let (buf, _) = render_badges(&model, 80, 0, &badges);
    let commit_y = (0..buf.area.height)
        .find(|y| buf_row(&buf, *y).contains("seed"))
        .expect("commit line");
    assert!(
        !buf_row(&buf, commit_y).contains('P'),
        "commit row unbadged"
    );
}

#[test]
fn row_badge_stays_at_the_right_edge_under_pan() {
    let (model, worktree_row) = badge_model();
    let badges = [(worktree_row, "P", Color::Green)];
    let width = 80;
    // Pan moves the label only, even past its end.
    for col_offset in [0, 4, 200] {
        let (buf, spans) = render_badges(&model, width, col_offset, &badges);
        assert_eq!(spans.len(), 1, "offset {col_offset}: {spans:?}");
        let span = &spans[0];
        assert_eq!((span.x, span.width), (width - 1, 1), "offset {col_offset}");
        assert_eq!(buf[(span.x, span.y)].symbol(), "P", "offset {col_offset}");
    }
    let (buf, spans) = render_badges(&model, width, 4, &badges);
    let line = buf_row(&buf, spans[0].y);
    assert!(line.contains("worktrees/feat"), "{line}");
    assert!(!line.contains("app/"), "{line}");
}

#[test]
fn row_badge_on_a_narrow_pane_clips_the_label_not_the_badge() {
    let (model, worktree_row) = badge_model();
    let badges = [(worktree_row, "P", Color::Green)];
    let width = 20;
    let (plain, _) = render_badges(&model, width, 0, &[]);
    let (buf, spans) = render_badges(&model, width, 0, &badges);
    assert_eq!(spans.len(), 1, "{spans:?}");
    let span = &spans[0];
    assert_eq!((span.x, span.width), (width - 1, 1));
    assert_eq!(buf[(span.x, span.y)].symbol(), "P");
    assert_eq!(buf[(span.x - 1, span.y)].symbol(), " ");
    let line = buf_row(&buf, span.y);
    assert!(line.contains("app/"), "{line}");
    assert!(!line.contains("graph"), "the label clips: {line}");
    // Left of the space the label is the unbadged label, cut earlier.
    for x in 0..span.x - 1 {
        assert_eq!(buf[(x, span.y)], plain[(x, span.y)], "cell {x}");
    }
    assert_ne!(plain[(span.x, span.y)].symbol(), " ", "label ran on");
}

#[test]
fn row_badge_gap_keeps_the_row_background() {
    let (model, worktree_row) = badge_model();
    let badges = [(worktree_row, "P", Color::Green)];
    for width in [20, 80] {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 12));
        let spans = badge_spans(
            GraphWidget::new(&model)
                .now_unix(NOW)
                .selected(Some(worktree_row))
                .cursor_style(Color::Cyan, Color::Blue)
                .row_badges(&badges)
                .render_with_icon_spans(buf.area, &mut buf),
        );
        let span = &spans[0];
        for x in 0..width {
            assert_eq!(buf[(x, span.y)].bg, Color::Blue, "width {width} cell {x}");
        }
    }
}

#[test]
fn fit_badge_glyph_keeps_the_right_edge() {
    assert_eq!(fit_badge_glyph("P", 1), ("P", 1));
    assert_eq!(fit_badge_glyph("P", 0), ("", 0));
    assert_eq!(fit_badge_glyph("", 4), ("", 0));
    assert_eq!(fit_badge_glyph("ab", 1), ("b", 1));
    assert_eq!(fit_badge_glyph("\u{754c}", 1), ("", 0));
}

#[test]
fn selected_commented_row_keeps_bar_and_quote() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(1))
                .commented_rows(&[1])
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_both = false;
    for y in 0..16u16 {
        let mut has_quote = false;
        let mut has_bar = false;
        for x in 0..80u16 {
            let sym = buffer[(x, y)].symbol();
            if sym == "\"" {
                has_quote = true;
            }
            if sym == "▌" {
                has_bar = true;
            }
        }
        if has_quote && has_bar {
            saw_both = true;
            break;
        }
    }
    assert!(
        saw_both,
        "selected commented graph row should paint ▌ and \""
    );
}

#[test]
fn uncommented_row_does_not_paint_quote_mark() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    for y in 0..16u16 {
        for x in 0..80u16 {
            assert_ne!(
                buffer[(x, y)].symbol(),
                "\"",
                "uncommented graph must not paint ICON_COMMENT"
            );
        }
    }
}

#[test]
fn commented_row_uses_supplied_glyph() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .commented_rows(&[1])
                .comment_glyph("\u{f075}")
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_nerd = false;
    for y in 0..16u16 {
        for x in 0..80u16 {
            if buffer[(x, y)].symbol() == "\u{f075}" {
                saw_nerd = true;
                break;
            }
        }
    }
    assert!(
        saw_nerd,
        "commented graph row should paint the supplied glyph"
    );
}

#[test]
fn resolved_comment_row_uses_resolved_glyph() {
    let model = sample_model();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .selected(Some(0))
                .commented_rows(&[1])
                .resolved_comment_rows(&[1, 2])
                .resolved_comment_glyph("'")
                .cursor_style(Color::Cyan, Color::DarkGray)
                .now_unix(NOW)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_open = false;
    let mut saw_resolved = false;
    for y in 0..16u16 {
        for x in 0..80u16 {
            match buffer[(x, y)].symbol() {
                "\"" => saw_open = true,
                "'" => saw_resolved = true,
                _ => {}
            }
        }
    }
    assert!(
        saw_open,
        "open commented_rows must win when a row is also in resolved_comment_rows"
    );
    assert!(
        saw_resolved,
        "resolved-only graph row should paint the resolved glyph"
    );
}

fn many_ref_commit() -> Commit {
    let mut refs = vec![
        GraphRef::local("main"),
        GraphRef::local("feature/long-topic"),
    ];
    for i in 0..6 {
        refs.push(GraphRef::tag(format!("v1.{i}.0")));
    }
    Commit {
        id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        subject: "tag soup".into(),
        refs,
        author_name: "Ada Lovelace".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    }
}

fn test_label_palette() -> GraphLabelPalette {
    GraphLabelPalette {
        subject: hex_color("#c0caf5"),
        meta: hex_color("#565f89"),
        branch_local: hex_color("#7aa2f7"),
        branch_default: hex_color("#9ece6a"),
        remote: hex_color("#7dcfff"),
        tag: hex_color("#e0af68"),
        head_mark: hex_color("#ff9e64"),
        overflow: hex_color("#7dcfff"),
        worktree: hex_color("#bb9af7"),
        dirty: hex_color("#e0af69"),
        comment: hex_color("#bb9af8"),
        comment_resolved: hex_color("#565f8a"),
        ahead: hex_color("#9ece6b"),
        behind: hex_color("#f7768e"),
    }
}

#[test]
fn narrow_row_shows_overflow_chip_for_hidden_branches_and_tags() {
    let commit = many_ref_commit();
    let model = GraphModel {
        commits: vec![commit.clone()],
        head_id: Some(commit.id.clone()),
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let width = 42u16;
    let painted = paint_model_with(
        &model,
        &crate::ASCII,
        PaintOpts {
            now_unix: Some(NOW),
            line_width: Some(width as usize),
            ..PaintOpts::default()
        },
    );
    let spacer = painted
        .iter()
        .find(|l| !l.selectable && l.label.contains("[main]"))
        .or_else(|| painted.iter().find(|l| !l.selectable))
        .expect("commit spacer");
    assert!(
        spacer.label.chars().count() <= width as usize,
        "row must not grow past pane: {}",
        spacer.label
    );
    let overflow = spacer
        .parts
        .iter()
        .find(|p| p.kind == crate::LabelKind::Overflow)
        .expect("overflow part");
    assert!(
        overflow.text.starts_with("[+") && overflow.text.ends_with(']'),
        "overflow chip: {}",
        overflow.text
    );
    assert!(
        !spacer.label.contains("[v1.5.0]") || !spacer.label.contains("[feature/long-topic]"),
        "some branch/tag chips must hide: {}",
        spacer.label
    );

    let row = GraphRow::Commit {
        commit: commit.clone(),
        is_head: true,
        worktrees: Vec::new(),
    };
    let [_, footer] = selection_detail_lines(
        &model,
        GraphFooterSelection::Row(&row),
        &crate::ASCII,
        200,
        NOW,
    );
    assert!(footer.contains("[main]"), "{footer}");
    assert!(footer.contains("[feature/long-topic]"), "{footer}");
    for i in 0..6 {
        assert!(
            footer.contains(&format!("[v1.{i}.0]")),
            "footer keeps tag v1.{i}.0: {footer}"
        );
    }
    assert!(
        !footer.contains(&overflow_chip_text(1)),
        "footer lists refs instead of collapsing them: {footer}"
    );
    assert!(
        footer.contains("[feature/long-topic]"),
        "pan/footer keep the full name: {footer}"
    );
}

#[test]
fn narrow_row_truncates_last_visible_chip_without_overflow_count() {
    let commit = Commit {
        id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        subject: "topic".into(),
        refs: vec![
            GraphRef::local("main"),
            GraphRef::local("feature/long-name"),
        ],
        author_name: "Ada".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    };
    let model = GraphModel {
        commits: vec![commit.clone()],
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let width = 28u16;
    let painted = paint_model_with(
        &model,
        &crate::ASCII,
        PaintOpts {
            now_unix: Some(NOW),
            line_width: Some(width as usize),
            ..PaintOpts::default()
        },
    );
    let spacer = painted
        .iter()
        .find(|l| !l.selectable && l.label.contains("[main]"))
        .expect("commit spacer");
    assert!(
        spacer.label.contains('…') && spacer.label.contains("…]"),
        "next chip name truncates in brackets: {}",
        spacer.label
    );
    assert!(
        spacer
            .parts
            .iter()
            .all(|p| p.kind != crate::LabelKind::Overflow),
        "truncated last chip is not [+N]: {}",
        spacer.label
    );
    let row = GraphRow::Commit {
        commit,
        is_head: false,
        worktrees: Vec::new(),
    };
    let [_, footer] = selection_detail_lines(
        &model,
        GraphFooterSelection::Row(&row),
        &crate::ASCII,
        200,
        NOW,
    );
    assert!(
        footer.contains("[feature/long-name]"),
        "footer lists the full name: {footer}"
    );
}

#[test]
fn overflow_chip_uses_overflow_colour_not_muted_meta() {
    let commit = many_ref_commit();
    let model = GraphModel {
        commits: vec![commit],
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let pal = test_label_palette();
    let backend = TestBackend::new(48, 10);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .ascii(true)
                .now_unix(NOW)
                .label_palette(pal)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut saw_overflow = false;
    let mut overflow_uses_meta = false;
    for y in 0..10u16 {
        let mut x = 0u16;
        while x < 48 {
            let cell = &buffer[(x, y)];
            if cell.symbol() != "[" {
                x += 1;
                continue;
            }
            let next = buffer[(x.saturating_add(1), y)].symbol();
            if next != "+" {
                x += 1;
                continue;
            }
            saw_overflow = true;
            assert_eq!(cell.fg, pal.overflow, "overflow chip colour");
            assert!(
                cell.modifier.contains(Modifier::BOLD),
                "overflow chip must be bold"
            );
            if cell.fg == pal.meta {
                overflow_uses_meta = true;
            }
            x += 1;
        }
    }
    assert!(saw_overflow, "expected [+N] on the narrow graph row");
    assert!(
        !overflow_uses_meta,
        "overflow must not use muted meta colour"
    );
}

#[test]
fn date_and_author_cells_paint_italic_hash_does_not() {
    let commit = Commit {
        id: "abc1234ffffffffffffffffffffffffffffffff".into(),
        subject: "topic".into(),
        author_name: "Ada Lovelace".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    };
    let model = GraphModel {
        commits: vec![commit],
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let pal = test_label_palette();
    let (w, h) = (80u16, 10u16);
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .ascii(true)
                .now_unix(NOW)
                .label_palette(pal)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let mut seen = [0usize; 3];
    for y in 0..h {
        let cells: Vec<(u16, &str)> = (0..w)
            .map(|x| (x, buffer[(x, y)].symbol()))
            .filter(|(_, s)| !s.is_empty())
            .collect();
        let text: String = cells.iter().map(|(_, s)| *s).collect();
        for (i, (needle, italic)) in [("abc1234", false), ("2m", true), ("Ada Lovelace", true)]
            .into_iter()
            .enumerate()
        {
            let Some(byte) = text.find(needle) else {
                continue;
            };
            seen[i] += 1;
            let start = text[..byte].chars().count();
            for (x, _) in &cells[start..start + needle.chars().count()] {
                let cell = &buffer[(*x, y)];
                assert_eq!(
                    cell.modifier.contains(Modifier::ITALIC),
                    italic,
                    "{needle:?} at ({x},{y}) in {text:?}"
                );
                assert_eq!(cell.fg, pal.meta, "{needle:?} keeps the meta colour");
            }
        }
    }
    assert!(
        seen.iter().all(|n| *n > 0),
        "hash/date/author painted: {seen:?}"
    );
}

fn row_text_and_fg(buffer: &Buffer, y: u16, width: u16) -> (String, Vec<Color>) {
    let mut text = String::new();
    let mut colors = Vec::new();
    for x in 0..width {
        let cell = &buffer[(x, y)];
        let sym = cell.symbol();
        if sym.is_empty() {
            continue;
        }
        for _ in 0..sym.chars().count() {
            colors.push(cell.fg);
        }
        text.push_str(sym);
    }
    (text, colors)
}

fn fg_at(text: &str, colors: &[Color], needle: &str, offset: usize) -> Color {
    let byte = text
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in {text:?}"));
    // `colors` is per char; a multi-byte cursor bar would skew a byte index.
    let i = text[..byte].chars().count();
    colors[i + offset]
}

#[test]
fn footer_ref_chips_reuse_row_chip_palette() {
    let commit = Commit {
        id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        subject: "palette footer".into(),
        refs: vec![
            GraphRef::local("main"),
            GraphRef::local("topic"),
            GraphRef::remote("origin/other"),
            GraphRef::tag("v1"),
        ],
        author_name: "Ada".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    };
    let model = GraphModel {
        commits: vec![commit],
        head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
        sync: Some(SyncState {
            branch: "main".into(),
            status: SyncStatus::UpToDate,
            ahead: 0,
            behind: 0,
        }),
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let pal = test_label_palette();
    let width = 100u16;
    let height = 16u16;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .ascii(true)
                .selected(Some(1))
                .now_unix(NOW)
                .label_palette(pal)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();

    let mut spacer: Option<(String, Vec<Color>)> = None;
    for y in 0..height.saturating_sub(2) {
        let row = row_text_and_fg(buffer, y, width);
        if row.0.contains("[+main]") && row.0.contains("[v1]") {
            spacer = Some(row);
            break;
        }
    }
    let (spacer_text, spacer_fg) = spacer.expect("commit spacer with chips");
    let (footer_text, footer_fg) = row_text_and_fg(buffer, height - 1, width);
    assert!(
        footer_text.contains("[+main]") && footer_text.contains("[topic]"),
        "footer lists branch chips: {footer_text}"
    );
    assert!(
        footer_text.contains("[v1]") && footer_text.contains("[origin/other]"),
        "footer lists tag and remote chips: {footer_text}"
    );

    let head = fg_at(&footer_text, &footer_fg, "[+main]", 1);
    let default = fg_at(&footer_text, &footer_fg, "[+main]", 2);
    let local = fg_at(&footer_text, &footer_fg, "[topic]", 1);
    let remote = fg_at(&footer_text, &footer_fg, "[origin/other]", 1);
    let tag = fg_at(&footer_text, &footer_fg, "[v1]", 1);
    assert_eq!(head, pal.head_mark, "HEAD checkout mark");
    assert_eq!(default, pal.branch_default, "default branch chip");
    assert_eq!(local, pal.branch_local, "feature branch chip");
    assert_eq!(remote, pal.remote, "remote chip");
    assert_eq!(tag, pal.tag, "tag chip");
    assert_ne!(head, default);
    assert_ne!(default, local);
    assert_ne!(local, tag);
    assert_ne!(tag, remote);
    assert_ne!(default, pal.meta);
    assert_eq!(
        head,
        fg_at(&spacer_text, &spacer_fg, "[+main]", 1),
        "footer HEAD mark must match the row chip"
    );
    assert_eq!(
        default,
        fg_at(&spacer_text, &spacer_fg, "[+main]", 2),
        "footer default branch must match the row chip"
    );
    assert_eq!(
        local,
        fg_at(&spacer_text, &spacer_fg, "[topic]", 1),
        "footer feature branch must match the row chip"
    );
    assert_eq!(
        tag,
        fg_at(&spacer_text, &spacer_fg, "[v1]", 1),
        "footer tag must match the row chip"
    );
    assert_eq!(
        remote,
        fg_at(&spacer_text, &spacer_fg, "[origin/other]", 1),
        "footer remote must match the row chip"
    );

    terminal
        .draw(|frame| {
            GraphWidget::new(&model)
                .ascii(true)
                .selected(Some(0))
                .now_unix(NOW)
                .label_palette(pal)
                .render(frame.area(), frame.buffer_mut());
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let (wt_footer, wt_fg) = row_text_and_fg(buffer, height - 1, width);
    assert_eq!(
        fg_at(&wt_footer, &wt_fg, "[+main]", 1),
        pal.head_mark,
        "working-tree footer HEAD mark: {wt_footer}"
    );
    assert_eq!(
        fg_at(&wt_footer, &wt_fg, "[v1]", 1),
        pal.tag,
        "working-tree footer tag: {wt_footer}"
    );
}

fn merged_head_model() -> GraphModel {
    let id = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    GraphModel {
        commits: vec![Commit {
            id: id.into(),
            subject: "merged head chip".into(),
            refs: vec![GraphRef::local("main"), GraphRef::remote("origin/main")],
            author_name: "Ada".into(),
            author_date_unix: NOW - 120,
            ..Commit::default()
        }],
        head_id: Some(id.into()),
        sync: Some(SyncState {
            branch: "main".into(),
            status: SyncStatus::UpToDate,
            ahead: 0,
            behind: 0,
        }),
        worktrees: vec![Worktree {
            path: ".worktrees/recon".into(),
            head_id: Some(id.into()),
            branch: Some("main".into()),
            hidden: false,
            is_current: true,
        }],
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    }
}

fn first_bracket_chip(line: &str) -> &str {
    let start = line
        .find('[')
        .unwrap_or_else(|| panic!("no chip in {line}"));
    let rel_end = line[start..]
        .find(']')
        .unwrap_or_else(|| panic!("unclosed chip in {line}"));
    &line[start..=start + rel_end]
}

#[test]
fn merged_head_chip_paints_one_chip_matching_footer() {
    let model = merged_head_model();
    let pal = test_label_palette();
    let width = 80u16;
    let height = 12u16;

    let paint_chip = |ascii: bool, want: &str| {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(ascii)
                    .selected(Some(1))
                    .now_unix(NOW)
                    .label_palette(pal)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut spacer = None;
        for y in 0..height.saturating_sub(2) {
            let (text, _) = row_text_and_fg(buffer, y, width);
            if text.contains("aaa1111") && text.contains(want) {
                spacer = Some(text);
                break;
            }
        }
        let spacer = spacer.unwrap_or_else(|| {
            panic!("commit spacer must paint {want}");
        });
        let (footer, _) = row_text_and_fg(buffer, height - 1, width);
        assert!(footer.contains(want), "footer must paint {want}: {footer}");
        assert_eq!(
            first_bracket_chip(&spacer),
            first_bracket_chip(&footer),
            "painted spacer chip must equal footer chip\nspacer={spacer}\nfooter={footer}"
        );
        assert!(
            !spacer.contains(".worktrees"),
            "worktree path must not be a second chip on the row: {spacer}"
        );
        let wt_glyph = if ascii {
            ASCII.worktree
        } else {
            UNICODE.worktree
        };
        assert!(
            !spacer.contains(wt_glyph),
            "worktree glyph must not prefix the footer chip: {spacer}"
        );
        assert!(
            !spacer.contains("[]") && !spacer.contains("[]") && !spacer.contains("[=]"),
            "marks must not be separate chips on the row: {spacer}"
        );
        let painted = paint_model_with(
            &model,
            if ascii { &ASCII } else { &UNICODE },
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(width as usize),
                ..PaintOpts::default()
            },
        );
        let spacer_line = painted
            .iter()
            .find(|l| !l.selectable && l.label.contains(want))
            .expect("painted spacer");
        assert!(
            spacer_line.label.contains(want),
            "painted row label: {}",
            spacer_line.label
        );
        let mark_runs: Vec<&str> = spacer_line
            .parts
            .iter()
            .filter(|p| {
                p.kind == crate::LabelKind::ChipHead || p.kind == crate::LabelKind::ChipRemote
            })
            .filter(|p| !p.text.contains('[') && !p.text.contains(']'))
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(
            mark_runs.len(),
            1,
            "checkout+sync must be one painted run: {:?}",
            spacer_line.parts
        );
    };

    paint_chip(false, "[main]");
    paint_chip(true, "[+=main]");
}

#[test]
fn col_offset_reveals_clipped_subject() {
    let marker = "UNIQUE_GRAPH_TAIL_xyz";
    let subject = format!("{}{marker}", "n".repeat(60));
    let model = GraphModel {
        commits: vec![commit(
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            &subject,
            &[],
        )],
        head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
        uncommitted: Some(false),
        window: 1,
        ..GraphModel::default()
    };
    let width = 36u16;
    let height = 8u16;
    let clipped = {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(true)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        !clipped.contains(marker),
        "narrow pane must clip the subject tail: {clipped}"
    );
    let panned = {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(true)
                    .now_unix(NOW)
                    .col_offset(50)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        panned.contains(marker),
        "panning must reveal the clipped tail: {panned}"
    );
    assert!(
        panned
            .lines()
            .all(|l| l.chars().count() <= width as usize + 4),
        "panning must not grow the row past the pane"
    );
}

/// Sample model plus an orphan stash and a detached worktree, so every
/// row kind is present.
fn every_row_kind_model() -> GraphModel {
    let mut model = sample_model();
    model.stashes.push(Stash {
        id: "ddd4444ddddddddddddddddddddddddddddddddd".into(),
        stash_ref: "stash@{1}".into(),
        subject: "WIP on gone".into(),
        body: String::new(),
        author_name: "Ada Lovelace".into(),
        author_date_unix: NOW - 7200,
        parent_id: Some("eee5555".into()),
        ..Stash::default()
    });
    model.worktrees.push(worktree("detached", "fff6666", false));
    model
}

#[test]
fn painted_line_count_matches_paint_for_every_row_kind() {
    for model in [
        GraphModel::default(),
        sample_model(),
        merge_model(),
        two_parent_join_model(),
        every_row_kind_model(),
    ] {
        for width in [None, Some(4)] {
            let painted = paint_model(&model, &UNICODE, width);
            assert_eq!(painted_line_count(&model), painted.len(), "{width:?}");
        }
    }
}

#[test]
fn painted_line_of_row_matches_paint_for_every_row_kind() {
    for model in [
        GraphModel::default(),
        sample_model(),
        merge_model(),
        two_parent_join_model(),
        every_row_kind_model(),
    ] {
        let painted = paint_model(&model, &UNICODE, None);
        let rows = model.visible_rows().len();
        for row in 0..rows {
            let line = painted
                .iter()
                .position(|line| line.row_index == Some(row) && line.selectable);
            assert_eq!(crate::painted_line_of_row(&model, row), line, "row {row}");
        }
        assert_eq!(crate::painted_line_of_row(&model, rows), None);
    }
}

#[test]
fn widget_paints_the_model_once_per_render() {
    let model = every_row_kind_model();
    // Tall (list fits, no bar) and short (bar) panes.
    for height in [40u16, 6] {
        let before = crate::paint::paint_calls();
        render_lines(&model, 60, height, false);
        assert_eq!(crate::paint::paint_calls() - before, 1, "height {height}");
    }
}

/// Icon spans of `model` in ASCII on a `width` × `height` buffer.
fn render_icons(
    model: &GraphModel,
    width: u16,
    height: u16,
    col_offset: u16,
    commented: &[usize],
    resolved: &[usize],
) -> (Buffer, Vec<IconSpan>) {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let spans = GraphWidget::new(model)
        .ascii(true)
        .now_unix(NOW)
        .col_offset(col_offset)
        .commented_rows(commented)
        .resolved_comment_rows(resolved)
        .render_with_icon_spans(buf.area, &mut buf);
    (buf, spans.into_iter().filter(is_row_glyph).collect())
}

/// A node, row glyph, comment mark, or badge span: not a chip, the
/// sync header, the footer hint, or a commit meta text run.
fn is_row_glyph(span: &IconSpan) -> bool {
    span.target.is_none()
        && !matches!(
            span.kind,
            GraphIconKind::SyncHeader
                | GraphIconKind::MoreBelow
                | GraphIconKind::CommitHash
                | GraphIconKind::CommitDate
                | GraphIconKind::CommitAuthor
        )
}

/// ASCII glyph each icon kind paints.
fn ascii_icon_glyph(kind: GraphIconKind) -> &'static str {
    match kind {
        GraphIconKind::CommitNode => ASCII.commit,
        GraphIconKind::HeadNode => ASCII.head_commit,
        GraphIconKind::StashNode => ASCII.stash,
        GraphIconKind::Uncommitted => ASCII.uncommitted,
        GraphIconKind::Worktree => ASCII.worktree,
        GraphIconKind::Comment => "\"",
        GraphIconKind::ResolvedComment => "'",
        GraphIconKind::Badge => "P",
        other => panic!("{other:?} is not a one-glyph icon"),
    }
}

/// Sample model with the hidden `notes` worktree shown: rows are
/// uncommitted (0), stash (1), HEAD commit with two worktree marks on
/// its spacer (2), and the root commit (3).
fn icon_model() -> GraphModel {
    GraphModel {
        show_hidden: true,
        ..sample_model()
    }
}

#[test]
fn icon_spans_cover_nodes_label_glyphs_comment_and_spacer_worktree_marks() {
    let model = icon_model();
    let (buf, spans) = render_icons(&model, 70, 12, 0, &[2], &[]);
    let got: Vec<(GraphIconKind, Option<usize>, Option<usize>)> = spans
        .iter()
        .map(|span| (span.kind, span.row_index, span.part))
        .collect();
    assert_eq!(
        got,
        vec![
            (GraphIconKind::Uncommitted, Some(0), None),
            (GraphIconKind::StashNode, Some(1), None),
            (GraphIconKind::HeadNode, Some(2), None),
            (GraphIconKind::Comment, Some(2), None),
            (GraphIconKind::Worktree, Some(2), Some(0)),
            (GraphIconKind::Worktree, Some(2), Some(1)),
            (GraphIconKind::CommitNode, Some(3), None),
        ]
    );
    for span in &spans {
        assert_eq!(span.width, 1, "{span:?}");
        assert_eq!(
            buf[(span.x, span.y)].symbol(),
            ascii_icon_glyph(span.kind),
            "{span:?}"
        );
    }
    // The worktree marks sit on the HEAD spacer, after the gutter.
    let marks: Vec<&IconSpan> = spans
        .iter()
        .filter(|span| span.kind == GraphIconKind::Worktree)
        .collect();
    let spacer = buf_row(&buf, marks[0].y);
    assert!(spacer.contains("L .worktrees/feature/graph feature/graph L notes"));
    assert_eq!(marks[0].y, marks[1].y);
    // Rails, junctions, the stash spacer, and the root's meta spacer
    // carry no icon.
    let rails = ["|", "-", "/", "\\", "+"];
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            if rails.contains(&buf[(x, y)].symbol()) {
                assert!(
                    !spans.iter().any(|span| span.y == y && span.x == x),
                    "rail cell {x},{y} is chrome"
                );
            }
        }
    }
    let stash_spacer = (0..12)
        .find(|y| buf_row(&buf, *y).contains("stash@{0}"))
        .expect("stash spacer");
    assert!(!spans.iter().any(|span| span.y == stash_spacer));
}

#[test]
fn node_spans_stay_put_while_label_glyphs_pan_and_clip() {
    let model = icon_model();
    let (_, plain) = render_icons(&model, 70, 12, 0, &[], &[]);
    let (buf, panned) = render_icons(&model, 70, 12, 1, &[], &[]);
    let nodes = |spans: &[IconSpan]| -> Vec<IconSpan> {
        spans
            .iter()
            .filter(|span| {
                matches!(
                    span.kind,
                    GraphIconKind::CommitNode | GraphIconKind::HeadNode | GraphIconKind::StashNode
                )
            })
            .cloned()
            .collect()
    };
    assert_eq!(nodes(&plain).len(), 3);
    assert_eq!(nodes(&plain), nodes(&panned), "the gutter does not pan");
    // The uncommitted glyph and the first worktree mark start the label:
    // one column of pan cuts them. The second mark moves left by one.
    let kinds: Vec<(GraphIconKind, Option<usize>)> = panned
        .iter()
        .filter(|span| !nodes(&plain).contains(span))
        .map(|span| (span.kind, span.part))
        .collect();
    assert_eq!(kinds, vec![(GraphIconKind::Worktree, Some(1))]);
    let before = plain
        .iter()
        .find(|span| span.part == Some(1))
        .expect("second mark");
    let after = panned
        .iter()
        .find(|span| span.part == Some(1))
        .expect("second mark panned");
    assert_eq!((after.x, after.y), (before.x - 1, before.y));
    assert_eq!(buf[(after.x, after.y)].symbol(), ASCII.worktree);
    // A narrow pane cuts the second mark off the right edge.
    let (_, narrow) = render_icons(&model, 40, 12, 0, &[], &[]);
    assert!(!narrow.iter().any(|span| span.part == Some(1)));
}

#[test]
fn a_node_clipped_by_the_gutter_cap_has_no_span() {
    let model = merge_model();
    let render = |cap: Option<u16>| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
        let mut widget = GraphWidget::new(&model).ascii(true).now_unix(NOW);
        if let Some(cap) = cap {
            widget = widget.gutter_width(cap);
        }
        let spans = widget.render_with_icon_spans(buf.area, &mut buf);
        spans
            .iter()
            .filter(|span| is_row_glyph(span))
            .filter_map(|span| span.row_index)
            .collect::<Vec<usize>>()
    };
    // `right` (row 2) is on the second lane, gutter column 2.
    assert_eq!(render(None), vec![0, 1, 2, 3]);
    assert_eq!(render(Some(2)), vec![0, 1, 3]);
}

#[test]
fn icon_spans_under_the_horizontal_bar_are_dropped() {
    let mut model = icon_model();
    model.commits[1].subject = "a root subject long enough to overflow the pane".into();
    let width = 40;
    assert!(graph_col_max(&model, true, width, true, &[]) > 0);
    let mut covered = 0;
    for height in 4..14 {
        let (_, plain) = render_icons(&model, width, height, 0, &[], &[]);
        let (buf, panned) = render_icons(&model, width, height, 1, &[], &[]);
        for span in &panned {
            assert_eq!(
                buf[(span.x, span.y)].symbol(),
                ascii_icon_glyph(span.kind),
                "height {height}: {span:?}"
            );
        }
        for node in plain.iter().filter(|span| {
            span.kind == GraphIconKind::CommitNode
                || span.kind == GraphIconKind::HeadNode
                || span.kind == GraphIconKind::StashNode
        }) {
            if !panned.contains(node) {
                covered += 1;
                assert_ne!(
                    buf[(node.x, node.y)].symbol(),
                    ascii_icon_glyph(node.kind),
                    "height {height}: a dropped node is under the bar"
                );
            }
        }
    }
    assert!(covered > 0, "some height puts a node under the bar");
}

#[test]
fn comment_and_row_glyphs_paint_their_palette_colours() {
    let pal = test_label_palette();
    let model = GraphModel {
        commits: vec![commit(
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "seed",
            &[],
        )],
        worktrees: vec![Worktree {
            path: "notes".into(),
            head_id: Some("elsewhere".into()),
            branch: None,
            hidden: true,
            is_current: true,
        }],
        uncommitted: Some(true),
        show_hidden: true,
        ..GraphModel::default()
    };
    let render = |model: &GraphModel| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10));
        let spans = GraphWidget::new(model)
            .ascii(true)
            .now_unix(NOW)
            .commented_rows(&[1])
            .resolved_comment_rows(&[2])
            .label_palette(pal)
            .render_with_icon_spans(buf.area, &mut buf);
        (buf, spans)
    };
    let (buf, spans) = render(&model);
    let at = |kind: GraphIconKind| {
        spans
            .iter()
            .find(|span| span.kind == kind)
            .cloned()
            .unwrap_or_else(|| panic!("{kind:?} in {spans:?}"))
    };
    let open = at(GraphIconKind::Comment);
    assert_eq!(buf[(open.x, open.y)].fg, pal.comment);
    assert!(!buf[(open.x, open.y)].modifier.contains(Modifier::BOLD));
    let resolved = at(GraphIconKind::ResolvedComment);
    assert_eq!(buf[(resolved.x, resolved.y)].fg, pal.comment_resolved);
    let dirty = at(GraphIconKind::Uncommitted);
    assert_eq!(buf[(dirty.x, dirty.y)].fg, pal.dirty);
    assert_eq!(buf[(dirty.x + 2, dirty.y)].fg, pal.subject, "row text");
    let wt = at(GraphIconKind::Worktree);
    assert_eq!(buf[(wt.x, wt.y)].fg, pal.worktree);
    assert_eq!(buf[(wt.x + 2, wt.y)].fg, pal.subject, "path");
    let row = buf_row(&buf, wt.y);
    let col = |needle: &str| row[..row.find(needle).expect(needle)].chars().count() as u16;
    assert_eq!(buf[(col("[hidden]") + 1, wt.y)].fg, pal.meta);
    assert_eq!(buf[(col("[HEAD]") + 1, wt.y)].fg, pal.head_mark);

    let clean = GraphModel {
        uncommitted: Some(false),
        ..model
    };
    let (buf, spans) = render(&clean);
    let glyph = spans
        .iter()
        .find(|span| span.kind == GraphIconKind::Uncommitted)
        .expect("clean glyph");
    assert_eq!(buf[(glyph.x, glyph.y)].fg, pal.meta);
}

/// HEAD on `main` (synced with `origin/main`) plus a local branch and a
/// tag, ahead by one. Selected: the commit (index 0).
fn chip_model() -> GraphModel {
    let commit = Commit {
        id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        subject: "chips".into(),
        refs: vec![
            GraphRef::local("main"),
            GraphRef::remote("origin/main"),
            GraphRef::local("feat"),
            GraphRef::tag("v1"),
        ],
        author_name: "Ada".into(),
        author_date_unix: NOW - 120,
        ..Commit::default()
    };
    GraphModel {
        head_id: Some(commit.id.clone()),
        commits: vec![commit],
        sync: Some(SyncState {
            branch: "main".into(),
            status: SyncStatus::Ahead,
            ahead: 1,
            behind: 0,
        }),
        window: 1,
        ..GraphModel::default()
    }
}

fn render_chips(
    model: &GraphModel,
    width: u16,
    col_offset: u16,
    palette: Option<GraphLabelPalette>,
) -> (Buffer, Vec<IconSpan>) {
    render_chips_in(model, Rect::new(0, 0, width, 12), col_offset, palette)
}

/// [`render_chips`] into `area`.
fn render_chips_in(
    model: &GraphModel,
    area: Rect,
    col_offset: u16,
    palette: Option<GraphLabelPalette>,
) -> (Buffer, Vec<IconSpan>) {
    let mut buf = Buffer::empty(area);
    let mut widget = GraphWidget::new(model)
        .ascii(true)
        .now_unix(NOW)
        .selected(Some(0))
        .col_offset(col_offset);
    if let Some(palette) = palette {
        widget = widget.label_palette(palette);
    }
    let spans = widget.render_with_icon_spans(buf.area, &mut buf);
    (buf, spans)
}

fn span_text(buf: &Buffer, span: &IconSpan) -> String {
    (span.x..span.x + span.width)
        .map(|x| buf[(x, span.y)].symbol())
        .collect()
}

#[test]
fn chip_spans_cover_each_chip_on_the_spacer_and_the_footer() {
    let model = chip_model();
    let (buf, spans) = render_chips(&model, 80, 0, None);
    let merged = Some(PartTarget::Chip(vec![
        GraphRef::local("main"),
        GraphRef::remote("origin/main"),
    ]));
    let marks = Some(PartTarget::Marks {
        branch: "main".into(),
        checkout: true,
        remote: Some("origin/main".into()),
    });
    let expected = vec![
        (GraphIconKind::DefaultChip, "[", merged.clone()),
        (GraphIconKind::ChipMarks, "+=", marks),
        (GraphIconKind::DefaultChip, "main]", merged),
        (
            GraphIconKind::LocalChip,
            "[feat]",
            Some(PartTarget::Chip(vec![GraphRef::local("feat")])),
        ),
        (
            GraphIconKind::TagChip,
            "[v1]",
            Some(PartTarget::Chip(vec![GraphRef::tag("v1")])),
        ),
    ];
    let chips_on = |y: u16| -> Vec<(GraphIconKind, String, Option<PartTarget>)> {
        spans
            .iter()
            .filter(|span| span.y == y && span.target.is_some())
            .map(|span| (span.kind, span_text(&buf, span), span.target.clone()))
            .collect()
    };
    let as_owned = |list: &[(GraphIconKind, &str, Option<PartTarget>)]| {
        list.iter()
            .map(|(kind, text, target)| (*kind, text.to_string(), target.clone()))
            .collect::<Vec<_>>()
    };
    // Header (0), the commit (1), its spacer (2); the footer's meta row
    // is the last one.
    assert_eq!(chips_on(2), as_owned(&expected), "spacer chips");
    assert_eq!(chips_on(11), as_owned(&expected), "footer chips");
    for span in spans.iter().filter(|span| span.target.is_some()) {
        assert_eq!(span.row_index, Some(0), "{span:?}");
        assert_eq!(span.part, None);
    }
    // The sync header is one span over its text, on no row.
    let header: Vec<&IconSpan> = spans
        .iter()
        .filter(|span| span.kind == GraphIconKind::SyncHeader)
        .collect();
    assert_eq!(header.len(), 1);
    assert_eq!((header[0].row_index, header[0].y), (None, 0));
    assert_eq!(span_text(&buf, header[0]), "main ^1");
}

#[test]
fn chip_spans_follow_the_pan_and_stop_at_the_pane_edge() {
    let model = chip_model();
    let (plain_buf, plain) = render_chips(&model, 80, 0, None);
    let tag = |spans: &[IconSpan]| {
        spans
            .iter()
            .find(|span| span.kind == GraphIconKind::TagChip && span.y == 2)
            .cloned()
    };
    let before = tag(&plain).expect("tag chip");
    assert_eq!(span_text(&plain_buf, &before), "[v1]");
    // A pan of 3 moves the spacer chips left; the first chip's `[` and
    // marks scroll out, the rest of it keeps a span.
    let (buf, panned) = render_chips(&model, 80, 3, None);
    let after = tag(&panned).expect("tag chip panned");
    assert_eq!((after.x, after.width), (before.x - 3, before.width));
    let first: Vec<String> = panned
        .iter()
        .filter(|span| span.y == 2 && span.kind == GraphIconKind::DefaultChip)
        .map(|span| span_text(&buf, span))
        .collect();
    assert_eq!(first, vec!["main]"]);
    assert!(!panned
        .iter()
        .any(|span| span.y == 2 && span.kind == GraphIconKind::ChipMarks));
    // The spacer refits a narrow pane (`[+N]`); the footer truncates its
    // meta line instead. A footer cut inside `[v1]` keeps a span over the
    // shown cells, and the `…` has none.
    // Meta capped to one row: on a 3-row pane (rows 9..12, the meta row
    // stays row 11) the footer has room for one meta row, so the meta line
    // is still cut.
    let (buf, cut) = render_chips_in(&model, Rect::new(0, 9, 19, 3), 0, None);
    let footer: Vec<(GraphIconKind, String)> = cut
        .iter()
        .filter(|span| span.y == 11 && span.target.is_some())
        .map(|span| (span.kind, span_text(&buf, span)))
        .collect();
    assert_eq!(
        footer.last(),
        Some(&(GraphIconKind::TagChip, "[v".to_string())),
        "{footer:?}"
    );
    assert!(buf_row(&buf, 11).starts_with("[+=main] [feat] [v…"));
}

/// A merge commit (two parents, an absolute date) with a stash parked
/// above it, and its two parents.
fn commit_meta_model() -> GraphModel {
    let merge = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
    let left = "1111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let right = "2222222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    GraphModel {
        commits: vec![
            Commit {
                id: merge.into(),
                subject: "merge topic".into(),
                parents: vec![left.into(), right.into()],
                author_name: "Ada Lovelace".into(),
                author_date_unix: NOW - 5 * 86_400,
                ..Commit::default()
            },
            Commit {
                author_name: "Bob Builder".into(),
                ..commit(left, "left", &[])
            },
            Commit {
                author_name: "Cy Coder".into(),
                ..commit(right, "right", &[])
            },
        ],
        stashes: vec![Stash {
            id: "5555555ccccccccccccccccccccccccccccccccc".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            author_name: "Ada Lovelace".into(),
            author_date_unix: NOW - 120,
            parent_id: Some(merge.into()),
            ..Stash::default()
        }],
        head_id: Some(merge.into()),
        window: 3,
        ..GraphModel::default()
    }
}

/// Commit meta spans (kind, part, painted text) on buffer row `y`, by
/// column.
fn meta_spans_on(
    buf: &Buffer,
    spans: &[IconSpan],
    y: u16,
) -> Vec<(GraphIconKind, Option<usize>, String)> {
    let mut found: Vec<&IconSpan> = spans
        .iter()
        .filter(|span| {
            span.y == y
                && matches!(
                    span.kind,
                    GraphIconKind::CommitHash
                        | GraphIconKind::CommitDate
                        | GraphIconKind::CommitAuthor
                )
        })
        .collect();
    found.sort_by_key(|span| span.x);
    found
        .into_iter()
        .map(|span| (span.kind, span.part, span_text(buf, span)))
        .collect()
}

#[test]
fn icon_spans_cover_spacer_hash_date_author_and_footer_parents() {
    use crate::format::{format_relative_date, short_id};
    let model = commit_meta_model();
    let rows = model.visible_rows();
    let index_of = |id: &str| {
        rows.iter()
            .position(|row| match row {
                GraphRow::Commit { commit, .. } => commit.id == id,
                GraphRow::Stash(stash) => stash.id == id,
                _ => false,
            })
            .unwrap_or_else(|| panic!("row {id}: {rows:?}"))
    };
    let merge = &model.commits[0];
    let stash = &model.stashes[0];
    let merge_row = index_of(&merge.id);
    let stash_row = index_of(&stash.id);
    let render_rows = |width: u16, height: u16, col_offset: u16| {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let spans = GraphWidget::new(&model)
            .ascii(true)
            .now_unix(NOW)
            .selected(Some(merge_row))
            .col_offset(col_offset)
            .render_with_icon_spans(buf.area, &mut buf);
        (buf, spans)
    };
    let render = |width: u16, col_offset: u16| render_rows(width, 16, col_offset);
    let (buf, spans) = render(100, 0);
    // The spacer line of a row: the row's meta spans below its node.
    let spacer_y = |row: usize| {
        spans
            .iter()
            .filter(|span| span.row_index == Some(row))
            .filter(|span| span.kind == GraphIconKind::CommitHash && span.part.is_none())
            .map(|span| span.y)
            .min()
            .unwrap_or_else(|| panic!("row {row} hash: {spans:?}"))
    };
    let date = |unix: i64| format_relative_date(unix, NOW);
    let expect = |hash: &str, date: String, author: &str| {
        vec![
            (GraphIconKind::CommitHash, None, short_id(hash).to_string()),
            (GraphIconKind::CommitDate, None, date),
            (GraphIconKind::CommitAuthor, None, author.to_string()),
        ]
    };
    let merge_y = spacer_y(merge_row);
    assert_eq!(
        meta_spans_on(&buf, &spans, merge_y),
        expect(&merge.id, date(merge.author_date_unix), &merge.author_name),
        "commit spacer: {}",
        buf_row(&buf, merge_y)
    );
    let stash_y = spacer_y(stash_row);
    assert_eq!(
        meta_spans_on(&buf, &spans, stash_y),
        expect(&stash.id, date(stash.author_date_unix), &stash.author_name),
        "stash spacer: {}",
        buf_row(&buf, stash_y)
    );
    // The stash's own date ("2m") is shorter than the date column: the
    // span covers the value, not the pad.
    assert!(date(stash.author_date_unix).len() < date(merge.author_date_unix).len());

    // Footer of the selected merge: own hash, both parents in git
    // order, author, date. All carry the selected row.
    let footer_y = spans
        .iter()
        .find(|span| span.part == Some(1))
        .map(|span| span.y)
        .unwrap_or_else(|| panic!("footer parent span: {spans:?}"));
    assert!(footer_y > merge_y);
    assert_eq!(
        meta_spans_on(&buf, &spans, footer_y),
        vec![
            (GraphIconKind::CommitHash, None, short_id(&merge.id).into()),
            (GraphIconKind::CommitHash, Some(0), "1111111".into()),
            (GraphIconKind::CommitHash, Some(1), "2222222".into()),
            (GraphIconKind::CommitAuthor, None, merge.author_name.clone()),
            (
                GraphIconKind::CommitDate,
                None,
                date(merge.author_date_unix)
            ),
        ],
        "footer: {}",
        buf_row(&buf, footer_y)
    );
    assert!(spans
        .iter()
        .filter(|span| span.y == footer_y)
        .all(|span| span.row_index == Some(merge_row)));

    // A pan that cuts the spacer hash drops its span; the date and the
    // author keep theirs, moved left by the pan.
    let hash_x = |spans: &[IconSpan]| {
        spans
            .iter()
            .find(|span| {
                span.y == merge_y
                    && span.kind == GraphIconKind::CommitHash
                    && span.row_index == Some(merge_row)
            })
            .map(|span| span.x)
    };
    let date_x = |spans: &[IconSpan]| {
        spans
            .iter()
            .find(|span| span.y == merge_y && span.kind == GraphIconKind::CommitDate)
            .map(|span| span.x)
    };
    let plain_hash = hash_x(&spans).expect("hash span");
    let plain_date = date_x(&spans).expect("date span");
    let mut cut_at = None;
    for offset in 1..100u16 {
        let (_, panned) = render(100, offset);
        match hash_x(&panned) {
            Some(x) => assert_eq!(x, plain_hash - offset, "offset {offset}"),
            None => {
                assert_eq!(date_x(&panned), Some(plain_date - offset));
                cut_at = Some(offset);
                break;
            }
        }
    }
    assert!(cut_at.is_some(), "a pan cuts the hash");

    // A narrow footer cuts the date: its span covers the shown cells,
    // and the `…` has none.
    let full = selection_detail_lines(
        &model,
        GraphFooterSelection::Row(&rows[merge_row]),
        &ASCII,
        200,
        NOW,
    )[1]
    .clone();
    let width = u16::try_from(full.chars().count() - 5).unwrap();
    // Meta capped to one row: a 3-row pane has room for one meta row, so
    // the narrow footer still cuts its meta line.
    let (buf, spans) = render_rows(width, 3, 0);
    let footer_y = spans
        .iter()
        .find(|span| span.part == Some(1))
        .map(|span| span.y)
        .expect("footer parent span");
    let cut = meta_spans_on(&buf, &spans, footer_y);
    let shown: String = date(merge.author_date_unix).chars().take(10).collect();
    assert_eq!(
        cut.last(),
        Some(&(GraphIconKind::CommitDate, None, shown)),
        "{cut:?}: {}",
        buf_row(&buf, footer_y)
    );
    assert!(buf_row(&buf, footer_y).trim_end().ends_with('…'));
}

#[test]
fn overflow_chip_span_names_the_hidden_chips() {
    let commit = many_ref_commit();
    let model = GraphModel {
        commits: vec![commit.clone()],
        head_id: Some(commit.id.clone()),
        window: 1,
        ..GraphModel::default()
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 44, 10));
    let spans = GraphWidget::new(&model)
        .ascii(true)
        .now_unix(NOW)
        .render_with_icon_spans(buf.area, &mut buf);
    let overflow = spans
        .iter()
        .find(|span| span.kind == GraphIconKind::OverflowChip)
        .unwrap_or_else(|| panic!("[+N] span: {spans:?}"));
    let Some(PartTarget::Overflow(hidden)) = &overflow.target else {
        panic!("overflow target: {overflow:?}");
    };
    assert_eq!(span_text(&buf, overflow), overflow_chip_text(hidden.len()));
    let shown: Vec<&PartTarget> = spans
        .iter()
        .filter(|span| span.kind != GraphIconKind::OverflowChip)
        .filter_map(|span| span.target.as_ref())
        .collect();
    for chip in hidden {
        assert!(
            matches!(chip, PartTarget::Chip(refs) if refs.len() == 1),
            "{chip:?}"
        );
        assert!(!shown.contains(&chip), "{chip:?} is not painted");
    }
}

#[test]
fn sync_header_paints_branch_and_marks_in_the_palette() {
    let mut model = chip_model();
    model.sync = Some(SyncState {
        branch: "main".into(),
        status: SyncStatus::Diverged,
        ahead: 2,
        behind: 3,
    });
    let pal = test_label_palette();
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    GraphWidget::new(&model)
        .now_unix(NOW)
        .label_palette(pal)
        .render(buf.area, &mut buf);
    let header = buf_row(&buf, 0);
    assert!(header.starts_with("main \u{f062}2 \u{f063}3"), "{header:?}");
    assert_eq!(buf[(0, 0)].fg, pal.branch_default);
    assert_eq!(buf[(5, 0)].fg, pal.ahead, "ahead glyph");
    assert_eq!(buf[(6, 0)].fg, pal.ahead, "ahead count");
    assert_eq!(buf[(8, 0)].fg, pal.behind, "behind glyph");
    // A feature branch with no upstream: local colour, muted word.
    model.sync = Some(SyncState {
        branch: "feat".into(),
        status: SyncStatus::NoUpstream,
        ahead: 0,
        behind: 0,
    });
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    GraphWidget::new(&model)
        .now_unix(NOW)
        .label_palette(pal)
        .render(buf.area, &mut buf);
    assert!(buf_row(&buf, 0).starts_with("feat no-upstream"));
    assert_eq!(buf[(0, 0)].fg, pal.branch_local);
    assert_eq!(buf[(5, 0)].fg, pal.meta);
    // No palette keeps the terminal default colour.
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    GraphWidget::new(&model)
        .now_unix(NOW)
        .render(buf.area, &mut buf);
    assert_eq!(buf[(0, 0)].fg, Color::Reset);
}

/// The first header chip candidate that fits paints flush right, two
/// columns clear of the sync text, and reports its cells; a narrow pane
/// takes a shorter one, then none. The sync text never moves.
#[test]
fn header_chip_paints_the_widest_candidate_that_fits_flush_right() {
    let mut model = chip_model();
    model.sync = Some(SyncState {
        branch: "main".into(),
        status: SyncStatus::Ahead,
        ahead: 2,
        behind: 0,
    });
    let red = Style::default().fg(Color::Red);
    let candidates = [
        Line::from(vec![
            Span::styled("x #9", red),
            Span::raw(" CI failed  > fix CI"),
        ]),
        Line::from(vec![Span::styled("x #9", red), Span::raw(" CI failed")]),
        Line::from(Span::styled("x #9", red)),
    ];
    // `main ^2` is 7 columns wide (`^` is the Nerd arrow).
    let sync = "main \u{f062}2";
    let header = |width: u16| {
        let mut buf = Buffer::empty(Rect::new(3, 1, width, 12));
        let spans = GraphWidget::new(&model)
            .now_unix(NOW)
            .header_chip(&candidates)
            .render_with_icon_spans(buf.area, &mut buf);
        let row: String = (3..3 + width).map(|x| buf[(x, 1)].symbol()).collect();
        let chip: Vec<IconSpan> = spans
            .into_iter()
            .filter(|span| span.kind == GraphIconKind::HeaderChip)
            .collect();
        (buf, row, chip)
    };
    let full = candidates[0].width() as u16;
    for (width, want) in [
        (60, Some("x #9 CI failed  > fix CI")),
        (7 + 2 + full, Some("x #9 CI failed  > fix CI")),
        (7 + 2 + full - 1, Some("x #9 CI failed")),
        (7 + 2 + 4, Some("x #9")),
        (7 + 2 + 3, None),
    ] {
        let (buf, row, chip) = header(width);
        assert!(row.starts_with(sync), "{width}: {row:?}");
        match want {
            Some(text) => {
                assert!(row.ends_with(text), "{width}: {row:?}");
                let w = text.chars().count() as u16;
                assert_eq!(
                    chip,
                    vec![IconSpan {
                        kind: GraphIconKind::HeaderChip,
                        row_index: None,
                        x: 3 + width - w,
                        y: 1,
                        width: w,
                        part: None,
                        target: None,
                    }],
                    "{width}"
                );
                assert_eq!(buf[(3 + width - w, 1)].fg, Color::Red, "{width}");
            }
            None => {
                assert!(chip.is_empty(), "{width}");
                assert_eq!(row.trim_end(), sync, "{width}");
            }
        }
    }
    // No sync header, no chip.
    model.sync = None;
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    let spans = GraphWidget::new(&model)
        .now_unix(NOW)
        .header_chip(&candidates)
        .render_with_icon_spans(buf.area, &mut buf);
    assert!(spans
        .iter()
        .all(|span| span.kind != GraphIconKind::HeaderChip));
}

#[test]
fn footer_hint_span_sits_on_the_more_below_glyph() {
    let model = fixed_footer_model();
    // One-row meta: the 47-column meta line fits 48 columns.
    let (width, height) = (48u16, 30u16);
    for (ascii, glyph) in [(false, UNICODE.more_below), (true, ASCII.more_below)] {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let spans = GraphWidget::new(&model)
            .ascii(ascii)
            .now_unix(NOW)
            .selected(Some(1))
            .commit_msg_expand(true)
            .commit_msg_scroll(10)
            .render_with_icon_spans(buf.area, &mut buf);
        let hint: Vec<&IconSpan> = spans
            .iter()
            .filter(|span| span.kind == GraphIconKind::MoreBelow)
            .collect();
        assert_eq!(hint.len(), 1, "{spans:?}");
        assert_eq!(hint[0].row_index, Some(1));
        assert_eq!(hint[0].y, height - 2);
        assert_eq!(span_text(&buf, hint[0]), format!("{glyph}18"));
    }
    // Every line shown: no hint span.
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let spans = GraphWidget::new(&model)
        .now_unix(NOW)
        .selected(Some(1))
        .commit_msg_expand(true)
        .commit_msg_scroll(999)
        .render_with_icon_spans(buf.area, &mut buf);
    assert!(!spans
        .iter()
        .any(|span| span.kind == GraphIconKind::MoreBelow));
}

/// [`fixed_footer_model`] with no HEAD, so no row has a `[HEAD]` chip:
/// row `i`'s meta is `(no refs) · c0<i>aaaa · parent c0<i+1>aaaa · Ada ·
/// 1h` (47 columns).
fn no_head_footer_model() -> GraphModel {
    GraphModel {
        head_id: None,
        ..fixed_footer_model()
    }
}

/// Expanded (default N) [`no_head_footer_model`] footer at 40x30 with row
/// `selected`: the 47-column meta wraps to two rows.
fn wrapped_meta_footer(selected: usize) -> (Buffer, Vec<IconSpan>) {
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 30));
    let spans = GraphWidget::new(&no_head_footer_model())
        .ascii(true)
        .now_unix(NOW)
        .selected(Some(selected))
        .commit_msg_expand(true)
        .render_with_icon_spans(buf.area, &mut buf);
    (buf, spans)
}

#[test]
fn wrapped_meta_rows_sit_under_the_message_and_the_list_gives_them_up() {
    for i in [0usize, 1] {
        let (buf, _) = wrapped_meta_footer(i);
        // Rule, 4 message rows, 2 meta rows: the list gives up one row.
        assert_eq!(painted_list_rows(&buf).len(), 23, "{i}");
        assert_eq!(buf_row(&buf, 23), "_".repeat(40), "{i}");
        assert!(buf_row(&buf, 24).starts_with(&format!("commit {i}")), "{i}");
        assert_eq!(
            buf_row(&buf, 28).trim_end(),
            format!("(no refs) · c0{i}aaaa · parent c0{}aaaa", i + 1)
        );
        assert_eq!(buf_row(&buf, 29).trim_end(), "Ada · 1h");
        for y in [28, 29] {
            assert!(!buf_row(&buf, y).contains('…'), "{i}: row {y}");
        }
    }
}

#[test]
fn wrapped_meta_rows_keep_hash_parent_author_and_date_spans_on_their_rows() {
    let (_, spans) = wrapped_meta_footer(0);
    let footer_span = |kind: GraphIconKind, part: Option<usize>| {
        spans
            .iter()
            .find(|span| span.y >= 28 && span.kind == kind && span.part == part)
            .map(|span| (span.x, span.y, span.width, span.row_index))
            .unwrap_or_else(|| panic!("{kind:?} {part:?}: {spans:?}"))
    };
    assert_eq!(
        footer_span(GraphIconKind::CommitHash, None),
        (12, 28, 7, Some(0))
    );
    assert_eq!(
        footer_span(GraphIconKind::CommitHash, Some(0)),
        (29, 28, 7, Some(0))
    );
    assert_eq!(
        footer_span(GraphIconKind::CommitAuthor, None),
        (0, 29, 3, Some(0))
    );
    assert_eq!(
        footer_span(GraphIconKind::CommitDate, None),
        (6, 29, 2, Some(0))
    );
}

#[test]
fn collapsed_footer_wraps_meta_under_the_subject() {
    let buf = render_footer(
        &no_head_footer_model(),
        40,
        30,
        FooterRender {
            selected: 0,
            expand: false,
            msg_scroll: 0,
            msg_lines: None,
            ascii: true,
        },
    );
    assert_eq!(painted_list_rows(&buf).len(), 26);
    assert_eq!(buf_row(&buf, 26), "_".repeat(40));
    assert_eq!(buf_row(&buf, 27).trim_end(), "commit 0");
    assert_eq!(
        buf_row(&buf, 28).trim_end(),
        "(no refs) · c00aaaa · parent c01aaaa"
    );
    assert_eq!(buf_row(&buf, 29).trim_end(), "Ada · 1h");
}

#[test]
fn meta_cut_on_a_tiny_pane_matches_the_one_row_truncation() {
    let buf = render_footer(
        &no_head_footer_model(),
        16,
        3,
        FooterRender {
            selected: 0,
            expand: false,
            msg_scroll: 0,
            msg_lines: None,
            ascii: true,
        },
    );
    assert_eq!(painted_list_rows(&buf), vec![0]);
    assert_eq!(buf_row(&buf, 1), "_".repeat(16));
    assert_eq!(buf_row(&buf, 2), "(no refs) · c00…");
}

#[test]
fn meta_chip_split_over_rows_keeps_one_target_per_row() {
    let name = "feature/a-very-long-branch-name";
    let mut tip = commit(&format!("c00{}", "a".repeat(37)), "long ref", &[]);
    tip.refs = vec![GraphRef::local(name)];
    let model = GraphModel {
        commits: vec![tip],
        uncommitted: None,
        window: 1,
        ..GraphModel::default()
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 12));
    let spans = GraphWidget::new(&model)
        .ascii(true)
        .now_unix(NOW)
        .selected(Some(0))
        .render_with_icon_spans(buf.area, &mut buf);
    // Meta rows 7..12: `[feature/a-very-long`, `-branch-name]`, `c00aaaa`,
    // `root commit · Ada`, `1h`.
    assert_eq!(buf_row(&buf, 7), "[feature/a-very-long");
    assert_eq!(buf_row(&buf, 8).trim_end(), "-branch-name]");
    let chips: Vec<(u16, String, Option<PartTarget>)> = spans
        .iter()
        .filter(|span| span.y >= 7 && span.target.is_some())
        .map(|span| (span.y, span_text(&buf, span), span.target.clone()))
        .collect();
    let target = Some(PartTarget::Chip(vec![GraphRef::local(name)]));
    assert_eq!(
        chips,
        vec![
            (7, "[feature/a-very-long".to_string(), target.clone()),
            (8, "-branch-name]".to_string(), target),
        ]
    );
}

#[test]
fn meta_chip_after_a_split_chip_keeps_its_blank_and_target() {
    let name = "feature/a-very-long-branch-name";
    let mut tip = commit(&format!("c00{}", "a".repeat(37)), "long ref", &[]);
    tip.refs = vec![GraphRef::local(name), GraphRef::tag("v1")];
    let model = GraphModel {
        commits: vec![tip],
        uncommitted: None,
        window: 1,
        ..GraphModel::default()
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 20, 12));
    let spans = GraphWidget::new(&model)
        .ascii(true)
        .now_unix(NOW)
        .selected(Some(0))
        .render_with_icon_spans(buf.area, &mut buf);
    // Meta rows 7..12: `[feature/a-very-long`, `-branch-name] [v1]`,
    // `c00aaaa`, `root commit · Ada`, `1h`.
    assert_eq!(buf_row(&buf, 8).trim_end(), "-branch-name] [v1]");
    let chips: Vec<(u16, u16, String, Option<PartTarget>)> = spans
        .iter()
        .filter(|span| span.y == 8 && span.target.is_some())
        .map(|span| (span.y, span.x, span_text(&buf, span), span.target.clone()))
        .collect();
    assert_eq!(
        chips,
        vec![
            (
                8,
                0,
                "-branch-name]".to_string(),
                Some(PartTarget::Chip(vec![GraphRef::local(name)]))
            ),
            (
                8,
                14,
                "[v1]".to_string(),
                Some(PartTarget::Chip(vec![GraphRef::tag("v1")]))
            ),
        ]
    );
}
