use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::super::super::command_palette::command_for;
use super::super::super::event_pump::overlay_blocks_background_ticks;
use super::super::super::keys::event_to_action;
use super::super::super::pr_status::PrState;
use super::super::super::pull_request::{PrLookup, PullRequest};
use super::super::super::remote_view::{RemoteTarget, NOTHING_TO_VIEW, UNCOMMITTED_LOCAL_ONLY};
use super::super::super::render::draw;
use super::super::FocusPane;
use super::*;
use crate::helpers::{DETACHED_HEAD_BRANCH, STATUS_FAILED_NOTE};
use crate::snapshot::{
    build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
};
use crate::testutil::{app_state, key, repo_snapshot, unstaged_change};
use crate::tui::comments::{put_comment, CommentKey};
use crate::tui::tree::SegRole;
use workspace_status_graph::{
    format_local_timestamp, format_relative_date, Commit, GraphModel, GraphRef, PartTarget, Stash,
    SyncState, SyncStatus as GraphSyncStatus, Worktree,
};

const PR_REMOTE: &str = "https://github.com/octo/demo.git";

fn repo(name: &str, status: SyncStatus, note: &str) -> RepoSnapshot {
    RepoSnapshot {
        branch: "feature".into(),
        sync_status: status,
        sync_note: note.into(),
        head: "abc".into(),
        has_unstaged: true,
        changes: vec![unstaged_change("src/lib.rs")],
        ..repo_snapshot(name)
    }
}

/// `app` two commits behind, `lib` one ahead; cursor on `lib`.
fn app() -> AppState {
    let mut app = app_state(&[
        repo("app", SyncStatus::Behind, "behind by 2 commits"),
        repo("lib", SyncStatus::Ahead, "ahead by 1 commits"),
    ]);
    focus(&mut app, "repo:lib");
    app
}

fn focus(app: &mut AppState, id: &str) {
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing row {id}"));
}

fn paint(app: &mut AppState) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
}

/// The sync mark hit on tree row `id`.
fn sync_hit(app: &AppState, id: &str) -> IconHit {
    app.layout
        .icon_hits
        .iter()
        .find(|hit| {
            hit.target == IconTarget::TreeRow(id.into())
                && matches!(hit.kind, IconKind::Ahead | IconKind::Behind)
        })
        .cloned()
        .unwrap_or_else(|| panic!("no icon hit on {id}: {:?}", app.layout.icon_hits))
}

fn focused_id(app: &AppState) -> String {
    app.focused_row().expect("focused row").id.clone()
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Open a peek over `hit` with the pointer resting on it from `at`.
fn peek_at(app: &mut AppState, hit: &IconHit, at: Instant) {
    app.set_pointer_at(Some((hit.x, hit.y)), at);
    assert!(app.expire_peek(at + ms(PEEK_DWELL_MS)));
}

fn section_lines(app: &AppState) -> Vec<Vec<PopoverLine>> {
    app.open_popover_sections()
        .into_iter()
        .map(|section| section.lines)
        .collect()
}

/// Pin the popover of the sync mark on tree row `id`, as a click on it
/// does.
fn pin_sync(app: &mut AppState, id: &str) {
    focus(app, id);
    paint(app);
    let hit = sync_hit(app, id);
    app.pin_icon(&hit);
    assert!(app.popover_pinned());
}

fn copy(text: &str) -> Effect {
    Effect::CopyClipboard {
        text: text.into(),
        announce: true,
    }
}

#[test]
fn sync_mark_records_a_hit_on_its_glyph_cells() {
    let mut app = app();
    let terminal = paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    assert_eq!(hit.kind, IconKind::Behind);
    assert_eq!(hit.width, 2);
    let buf = terminal.backend().buffer();
    assert_eq!(buf[(hit.x, hit.y)].symbol(), "v");
    assert_eq!(buf[(hit.x + 1, hit.y)].symbol(), "2");
    assert_eq!(sync_hit(&app, "repo:lib").kind, IconKind::Ahead);
    assert!(
        app.layout.pr_badge_hits().is_empty(),
        "sync marks are not PRs"
    );
}

#[test]
fn dwell_opens_a_peek_that_is_not_a_mode() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    let t0 = Instant::now();
    assert!(!app.set_pointer_at(Some((0, 0)), t0), "off every icon");
    assert_eq!(app.peek_remaining_ms(t0), None);
    assert!(
        app.set_pointer_at(Some((hit.x, hit.y)), t0),
        "onto the icon"
    );
    assert!(
        !app.set_pointer_at(Some((hit.x + 1, hit.y)), t0),
        "motion on the same icon does not redraw"
    );
    assert_eq!(app.peek_remaining_ms(t0), Some(PEEK_DWELL_MS));
    assert!(!app.expire_peek(t0 + ms(PEEK_DWELL_MS - 1)));
    assert!(app.popover.is_none());
    assert!(app.expire_peek(t0 + ms(PEEK_DWELL_MS)));
    let popover = app.popover.as_ref().expect("peek");
    assert_eq!(popover.origin, PopoverOrigin::Peek);
    assert_eq!(popover.anchor, Some(hit.rect()));
    assert_eq!(
        app.input_mode(),
        InputMode::Normal {
            search_active: false
        },
        "a peek takes no keys"
    );
    assert_eq!(focused_id(&app), "repo:lib", "a peek selects nothing");
    assert_eq!(app.peek_remaining_ms(t0), None, "nothing left to time");
}

#[test]
fn leaving_starts_the_grace_and_the_peek_itself_keeps_it() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    let t0 = Instant::now();
    peek_at(&mut app, &hit, t0);
    paint(&mut app);
    let rect = app.layout.popover.as_ref().expect("painted peek").rect;

    let t1 = t0 + ms(1_000);
    app.set_pointer_at(Some((0, 0)), t1);
    assert_eq!(app.peek_remaining_ms(t1), Some(PEEK_GRACE_MS));
    app.set_pointer_at(Some((rect.x + 1, rect.y + 1)), t1 + ms(100));
    assert_eq!(app.peek_remaining_ms(t1), None, "pointer on the peek");
    assert!(!app.expire_peek(t1 + ms(PEEK_GRACE_MS)));
    assert!(app.popover.is_some());

    let t2 = t1 + ms(2_000);
    app.set_pointer_at(Some((0, 0)), t2);
    assert!(!app.expire_peek(t2 + ms(PEEK_GRACE_MS - 1)));
    assert!(app.expire_peek(t2 + ms(PEEK_GRACE_MS)));
    assert!(app.popover.is_none(), "grace ended");
}

/// A pinned popover has no leave grace: the pointer leaving starts no
/// timer, and no time past the grace closes it.
#[test]
fn a_pinned_popover_has_no_leave_grace() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    app.dispatch(Action::Click {
        col: hit.x,
        row: hit.y,
    });
    app.dispatch(Action::Release);
    paint(&mut app);
    assert_eq!(app.input_mode(), InputMode::Popover, "pinned");

    let t0 = Instant::now();
    app.set_pointer_at(Some((0, 0)), t0);
    assert_eq!(app.peek_remaining_ms(t0), None, "no grace timer");
    assert!(!app.expire_peek(t0 + ms(PEEK_GRACE_MS * 10)));
    assert!(app.popover.is_some(), "the pinned popover stays");
}

#[test]
fn icons_under_the_search_dialog_do_not_hover_or_arm_a_peek() {
    let mut app = app();
    app.dispatch(Action::ToggleSearchFiles);
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    let t0 = Instant::now();
    assert!(
        !app.set_pointer_at(Some((hit.x, hit.y)), t0),
        "no icon hover under the dialog"
    );
    assert!(app.hovered_icon().is_none());
    assert_eq!(app.peek_remaining_ms(t0), None, "no dwell armed");

    app.dispatch(Action::SearchFilesEscape);
    assert!(app.hovered_icon().is_some(), "closed: the icon hovers");
}

#[test]
fn mouse_off_and_overlays_drop_the_peek() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    peek_at(&mut app, &hit, Instant::now());
    app.dispatch(Action::ToggleMouse);
    assert!(app.popover.is_none(), "mouse off");
    assert!(!app.set_pointer_at(Some((hit.x, hit.y)), Instant::now()));
    assert_eq!(app.peek_remaining_ms(Instant::now()), None);

    app.dispatch(Action::ToggleMouse);
    paint(&mut app);
    peek_at(&mut app, &hit, Instant::now());
    app.dispatch(Action::ToggleHelp);
    paint(&mut app);
    assert!(app.popover.is_none(), "help hides the peek");
}

#[test]
fn click_on_an_icon_selects_its_row_and_pins_without_a_double_click() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    assert_eq!(
        app.dispatch(Action::Click {
            col: hit.x,
            row: hit.y
        }),
        Effect::LoadRightPane
    );
    assert_eq!(focused_id(&app), "repo:app");
    assert_eq!(app.input_mode(), InputMode::Popover);
    assert!(
        app.last_click.is_none(),
        "an icon click arms no double-click"
    );
    assert_eq!(app.dispatch(Action::Release), Effect::None);
    paint(&mut app);
    assert!(app.layout.popover.is_some());

    // A second click on the icon is outside the popover: it only closes.
    assert_eq!(
        app.dispatch(Action::Click {
            col: hit.x,
            row: hit.y
        }),
        Effect::None
    );
    assert!(app.popover.is_none());
    assert_eq!(app.focus, FocusPane::Left, "no drill");
    assert!(app.last_click.is_none());
}

#[test]
fn click_outside_only_closes_and_double_click_elsewhere_still_drills() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    app.dispatch(Action::Click {
        col: hit.x,
        row: hit.y,
    });
    paint(&mut app);
    let lib = sync_hit(&app, "repo:lib");
    // A press on another row while pinned closes and selects nothing.
    assert_eq!(
        app.dispatch(Action::Click { col: 8, row: lib.y }),
        Effect::None
    );
    assert!(app.popover.is_none());
    assert_eq!(focused_id(&app), "repo:app");

    paint(&mut app);
    app.dispatch(Action::Click { col: 8, row: lib.y });
    app.dispatch(Action::Click { col: 8, row: lib.y });
    assert_eq!(focused_id(&app), "repo:lib");
    assert_eq!(
        app.focus,
        FocusPane::Right,
        "double-click on the label drills"
    );
    assert!(app.popover.is_none());
}

#[test]
fn click_inside_a_peek_pins_it_on_its_row() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    peek_at(&mut app, &hit, Instant::now());
    paint(&mut app);
    let rect = app.layout.popover.as_ref().expect("peek").rect;
    assert_eq!(
        app.dispatch(Action::Click {
            col: rect.x + 2,
            row: rect.y + 1
        }),
        Effect::LoadRightPane
    );
    assert_eq!(focused_id(&app), "repo:app");
    assert!(app.popover_pinned());
    assert_eq!(app.input_mode(), InputMode::Popover);
}

#[test]
fn esc_closes_the_pinned_popover_before_the_esc_chain() {
    let mut app = app();
    app.search_active = true;
    app.search_query = "app".into();
    focus(&mut app, "repo:app");
    app.dispatch(Action::PopoverOpenFocused);
    let mode = app.input_mode();
    assert_eq!(mode, InputMode::Popover);
    let esc = event_to_action(&key(KeyCode::Esc), mode, false, false);
    assert_eq!(esc, Action::PopoverClose);
    assert_eq!(app.dispatch(esc), Effect::None);
    assert!(app.popover.is_none());
    assert!(app.search_active, "the armed search outlives the first Esc");
    assert_eq!(focused_id(&app), "repo:app");
    assert_eq!(
        event_to_action(&key(KeyCode::Esc), app.input_mode(), false, false),
        Action::NavEsc,
        "the next Esc is the usual chain"
    );
}

#[test]
fn focus_change_closes_the_pinned_popover() {
    let mut app = app();
    focus(&mut app, "repo:app");
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned());
    focus(&mut app, "repo:lib");
    assert!(!app.popover_pinned(), "another row");
    assert_ne!(app.input_mode(), InputMode::Popover);
    paint(&mut app);
    assert!(app.popover.is_none(), "paint drops it");

    focus(&mut app, "repo:app");
    app.dispatch(Action::PopoverOpenFocused);
    app.focus = FocusPane::Right;
    assert!(!app.popover_pinned(), "another pane");

    app.focus = FocusPane::Left;
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned());
    app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
    assert!(app.is_compare_tab());
    assert!(!app.popover_pinned(), "another tab");
    assert_ne!(app.input_mode(), InputMode::Popover);

    // The tab alone is enough: same list and row, other tab.
    app.dispatch(Action::JumpToTab(1));
    focus(&mut app, "repo:app");
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned());
    app.popover.as_mut().unwrap().owner.tab += 1;
    assert!(!app.popover_pinned(), "pinned on another tab");
}

#[test]
fn ticks_keep_running_under_a_pinned_popover() {
    assert!(!overlay_blocks_background_ticks(InputMode::Popover));
    let mut app = app();
    focus(&mut app, "repo:app");
    app.dispatch(Action::PopoverOpenFocused);
    app.dispatch(Action::WatchTick);
    assert!(app.popover_pinned(), "a tick on the same row keeps it");
}

#[test]
fn gh_lists_every_icon_of_the_row_in_paint_order() {
    let mut app = app();
    assert_eq!(app.due_pr_lookups().len(), 2);
    assert!(app.apply_pr_lookup(
        Path::new("app"),
        "feature",
        Some(PR_REMOTE.into()),
        PrLookup::Found(PullRequest::fixture(
            7,
            "https://github.com/octo/demo/pull/7",
            PrState::InReview,
        )),
    ));
    focus(&mut app, "repo:app");
    assert_eq!(
        app.dispatch(Action::PopoverOpenFocused),
        Effect::LookupPullRequestDetail {
            repo: PathBuf::from("app"),
            branch: "feature".into(),
            remote: PR_REMOTE.into(),
            number: 7,
            request: 1,
        },
        "the pin starts the PR detail fetch"
    );
    let sections = app.open_popover_sections();
    let icons: Vec<IconKind> = sections.iter().map(|section| section.icon).collect();
    assert_eq!(
        icons,
        vec![
            IconKind::Branch,
            IconKind::Repo,
            IconKind::Behind,
            IconKind::ChangeCount,
            IconKind::PrInReview,
        ]
    );
    let titles = |lines: &[PopoverLine]| -> Vec<String> {
        lines.iter().map(PopoverLine::copy_text).collect()
    };
    let lines = section_lines(&app);
    assert_eq!(
        titles(&lines[4]),
        vec![
            "#7".to_string(),
            "loading…".into(),
            "> get review".into(),
            "just now".into(),
            "https://github.com/octo/demo/pull/7".into(),
            String::new(),
            "Open PR".into(),
            "Open checks".into(),
            "Copy URL".into(),
            "Refresh this PR".into(),
        ]
    );
    assert_eq!(sections[4].heading(), "PR · in review");
    assert_eq!(
        titles(&lines[2]),
        vec![
            IconKind::Behind.spec().meaning.to_string(),
            "2 commits to pull".into(),
            "Pull behind".into(),
            "Fetch remotes".into(),
        ]
    );
    // The landing line is the first action: the branch picker.
    let focus = app.popover.as_ref().unwrap().focus_line;
    assert_eq!(flat_lines(&sections)[focus].copy_text(), "Branch picker");

    app.close_popover();
    app.status.clear();
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    app.dispatch(Action::PopoverOpenFocused);
    let kinds: Vec<IconKind> = app
        .popover
        .as_ref()
        .expect("file row popover")
        .targets
        .iter()
        .map(|(kind, _)| *kind)
        .collect();
    assert_eq!(kinds, vec![IconKind::FileType, IconKind::StatusModified]);

    // The right pane with no graph paints no icon.
    app.close_popover();
    app.focus = FocusPane::Right;
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover.is_none());
    assert_eq!(app.status, NO_ICONS_ON_ROW);
}

#[test]
fn popover_keys_move_copy_and_run() {
    let mut app = app();
    pin_sync(&mut app, "repo:app");
    assert_eq!(app.dispatch(Action::PopoverCopyLine), copy("Pull behind"));
    app.dispatch(Action::PopoverMove(1));
    assert_eq!(app.dispatch(Action::PopoverCopyLine), copy("Fetch remotes"));
    app.dispatch(Action::PopoverMove(1));
    assert_eq!(
        app.dispatch(Action::PopoverCopyLine),
        copy("Fetch remotes"),
        "stops at the last line"
    );
    app.dispatch(Action::PopoverMove(-5));
    assert_eq!(
        app.dispatch(Action::PopoverCopyLine),
        copy("2 commits to pull"),
        "the meaning never focuses"
    );
    assert_eq!(app.dispatch(Action::PopoverRun), Effect::None, "a field");
    assert!(app.popover_pinned(), "Enter on a field keeps it");
    app.dispatch(Action::PopoverMove(1));
    assert_eq!(app.popover_run_action(), Some(Action::Pull));

    let mut twin = app.clone();
    twin.close_popover();
    let want = twin.dispatch(Action::Pull);
    assert_eq!(app.dispatch(Action::PopoverRun), want);
    assert!(app.popover.is_none(), "Enter closes, then runs");
    assert_eq!(app.status, twin.status);
}

#[test]
fn release_on_an_action_line_runs_it_and_a_drag_copies_instead() {
    let mut app = app();
    pin_sync(&mut app, "repo:app");
    paint(&mut app);
    let painted = app.layout.popover.clone().expect("painted");
    let sections = app.open_popover_sections();
    let lines = flat_lines(&sections);
    let (fetch_y, fetch) = *painted
        .lines
        .iter()
        .find(|(_, index)| lines[*index].copy_text() == "Fetch remotes")
        .expect("fetch line");

    // Drag across the Fetch line: copies, runs nothing, stays open.
    let x = painted.inner.x;
    app.dispatch(Action::Click {
        col: x,
        row: fetch_y,
    });
    app.dispatch(Action::Drag {
        col: x + 6,
        row: fetch_y,
    });
    match app.dispatch(Action::Release) {
        Effect::CopyClipboard { text, .. } => assert!(text.contains("Fetch"), "{text}"),
        other => panic!("drag should copy, got {other:?}"),
    }
    assert!(app.popover_pinned());

    // A plain click on the line focuses it; the release runs it.
    assert_eq!(
        app.dispatch(Action::Click {
            col: x + 2,
            row: fetch_y
        }),
        Effect::None
    );
    assert_eq!(app.popover.as_ref().unwrap().focus_line, fetch);
    let mut twin = app.clone();
    twin.close_popover();
    let want = twin.dispatch(Action::Fetch);
    assert_eq!(app.dispatch(Action::Release), want);
    assert!(app.popover.is_none());
}

/// A pinned popover takes the mouse, so the pane divider does not
/// hover, even with the pointer outside the popover box; it hovers again
/// once the popover closes.
#[test]
fn a_pinned_popover_clears_the_resize_edge_hover() {
    use super::super::super::split::ResizeEdge;
    let mut app = app();
    pin_sync(&mut app, "repo:app");
    paint(&mut app);
    assert_eq!(app.input_mode(), InputMode::Popover);
    let painted = app.layout.popover.clone().expect("painted");
    // The left pane's right border, on a body row under the popover.
    let divider = (app.layout.outer_tree_width - 1, painted.rect.bottom() + 1);
    assert!(
        !painted
            .rect
            .contains(ratatui::layout::Position::new(divider.0, divider.1)),
        "the pointer is outside the popover: {painted:?}"
    );
    app.set_pointer(Some(divider));
    assert_eq!(app.hovered_resize_edge(), None, "pinned popover");
    app.close_popover();
    assert_eq!(
        app.hovered_resize_edge(),
        Some(ResizeEdge::Pane),
        "the same cell hovers once the popover closes"
    );
}

#[test]
fn a_dialog_that_opens_closes_the_pinned_popover() {
    let mut app = app();
    pin_sync(&mut app, "repo:app");
    paint(&mut app);
    assert!(app.layout.popover.is_some());
    // A dialog that opens without a key (a picker that loaded, a
    // confirm from an effect) outranks the popover and owns the keys.
    app.help_open = true;
    assert_eq!(app.input_mode(), InputMode::Help);
    paint(&mut app);
    assert!(app.popover.is_none(), "paint closes the pinned popover");
    assert!(app.layout.popover.is_none());
    app.help_open = false;
    assert_ne!(app.input_mode(), InputMode::Popover);
}

#[test]
fn a_press_on_an_action_line_exposes_the_action_the_release_runs() {
    use super::super::super::event_pump::{classify_busy_dispatch, BusyAction};
    let mut app = app();
    pin_sync(&mut app, "repo:app");
    paint(&mut app);
    let painted = app.layout.popover.clone().expect("painted");
    let sections = app.open_popover_sections();
    let lines = flat_lines(&sections);
    let row_of = |title: &str| {
        painted
            .lines
            .iter()
            .find(|(_, index)| lines[*index].copy_text() == title)
            .map(|(y, _)| *y)
            .expect("line")
    };
    let (pull_y, field_y) = (row_of("Pull behind"), row_of("2 commits to pull"));
    let x = painted.inner.x + 2;
    assert_eq!(app.popover_release_action(), None, "no press");
    app.dispatch(Action::Click {
        col: x,
        row: pull_y,
    });
    assert_eq!(app.popover_release_action(), Some(Action::Pull));
    // The busy loop classifies that release as the line's action.
    assert_eq!(
        classify_busy_dispatch(&Action::Release, app.popover_release_action().as_ref()),
        BusyAction::Handle
    );
    app.dispatch(Action::Drag {
        col: x + 4,
        row: pull_y,
    });
    assert_eq!(app.popover_release_action(), None, "a drag runs nothing");
    app.dispatch(Action::Release);
    app.dispatch(Action::Click {
        col: x,
        row: field_y,
    });
    assert_eq!(app.popover_release_action(), None, "a field runs nothing");
}

/// A family (`app` primary merged and dirty, linked `feat` merged and
/// behind), a repo whose status failed, a hidden dirty repo shown,
/// and an idle repo under No updates.
fn branch_rows_app() -> AppState {
    let mut primary = repo("app", SyncStatus::NoUpstream, "");
    primary.branch = "feature/landed".into();
    primary.merged_into_default = Some(true);
    primary.default_tip_ref = Some("origin/main".into());
    primary.local_branches = vec!["main".into(), "feature/landed".into()];
    primary.has_staged = true;
    primary.has_untracked = true;
    primary.changes.extend([
        FileChange {
            path: "src/staged.rs".into(),
            staged_status: Some("M".into()),
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        },
        FileChange {
            path: "new.txt".into(),
            staged_status: None,
            unstaged_status: None,
            untracked: true,
            old_path: None,
        },
    ]);
    let mut linked = repo(
        "app/.worktrees/feat",
        SyncStatus::Behind,
        "behind by 3 commits",
    );
    linked.branch = "feature/side".into();
    linked.checkout_kind = CheckoutKind::Linked;
    linked.primary_repo = Some("app".into());
    linked.merged_into_default = Some(true);
    linked.changes.clear();
    linked.has_unstaged = false;
    let mut broken = repo("broken", SyncStatus::NoUpstream, STATUS_FAILED_NOTE);
    broken.changes.clear();
    broken.has_unstaged = false;
    let notes = repo("notes", SyncStatus::NoUpstream, "");
    let mut idle = repo("idle", SyncStatus::UpToDate, "");
    idle.branch = "main".into();
    idle.changes.clear();
    idle.has_unstaged = false;
    let snapshot = build_workspace_snapshot(
        &[primary, linked, broken, notes, idle],
        &["notes".into()],
        true,
        &[],
    );
    let mut app = AppState::new(PathBuf::from("/tmp/ws"), snapshot, true);
    app.show_hidden = true;
    app.rebuild_rows();
    app
}

/// `gh` on row `id`: each section's icon and the copy text of each line.
fn gh_on(app: &mut AppState, id: &str) -> Vec<(IconKind, Vec<String>)> {
    app.close_popover();
    focus(app, id);
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned(), "gh pins on {id}");
    app.open_popover_sections()
        .into_iter()
        .map(|section| {
            let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
            (section.icon, lines)
        })
        .collect()
}

fn texts(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_string()).collect()
}

fn meaning(kind: IconKind) -> &'static str {
    kind.spec().meaning
}

#[test]
fn every_branch_row_icon_has_its_fields_and_actions() {
    use IconKind as K;
    let mut app = branch_rows_app();
    let diff = "Diff vs default in new tab";
    assert_eq!(
        gh_on(&mut app, "workspace"),
        vec![(
            K::Workspace,
            texts(&[
                meaning(K::Workspace),
                "ws",
                "4 changed",
                "1 behind, 1 attention",
                "4 repos",
                "Fetch remotes",
                "Refresh",
                "Show hidden",
            ])
        )]
    );
    assert_eq!(
        gh_on(&mut app, "repo:app"),
        vec![
            (
                K::Repo,
                texts(&[
                    meaning(K::Repo),
                    "app",
                    "family of 2 checkouts",
                    "1 staged · 2 unstaged · 1 untracked",
                    "Fetch remotes",
                    "Pull behind",
                    "Push",
                    "Branch picker",
                    "View remotely",
                ])
            ),
            (
                K::Behind,
                texts(&[
                    meaning(K::Behind),
                    "3 commits to pull",
                    "Pull behind",
                    "Fetch remotes"
                ])
            ),
            (
                K::WorktreeCount,
                texts(&[
                    meaning(K::WorktreeCount),
                    "feature/landed",
                    "feature/side",
                    "Fold row",
                    "Fold subtree",
                ])
            ),
            (
                K::ChangeCount,
                texts(&[meaning(K::ChangeCount), "1", "2", "1", "Stash menu", diff])
            ),
        ]
    );
    assert_eq!(
        gh_on(&mut app, "checkout:app"),
        vec![
            (
                K::Branch,
                texts(&[
                    meaning(K::Branch),
                    "feature/landed",
                    "feature branch",
                    "2 branches",
                    "Branch picker",
                    "Default branch",
                    "Graph focus branches",
                    diff,
                    "View remotely",
                ])
            ),
            (
                K::MergedIntoDefault,
                texts(&[
                    meaning(K::MergedIntoDefault),
                    "origin/main",
                    diff,
                    "Default branch"
                ])
            ),
            (
                K::NoUpstream,
                texts(&[meaning(K::NoUpstream), "Push", "Fetch remotes"])
            ),
            (
                K::ChangeCount,
                texts(&[meaning(K::ChangeCount), "1", "2", "1", "Stash menu", diff])
            ),
        ]
    );
    assert_eq!(
        gh_on(&mut app, "checkout:app/.worktrees/feat"),
        vec![
            (
                K::LinkedWorktree,
                texts(&[
                    meaning(K::LinkedWorktree),
                    "app/.worktrees/feat",
                    "app",
                    "feature/side",
                    "Remove worktree",
                    diff,
                    "Copy entity reference",
                    "View remotely",
                ])
            ),
            (
                K::MergedIntoDefault,
                texts(&[
                    meaning(K::MergedIntoDefault),
                    "the default branch",
                    diff,
                    "Default branch",
                    "Remove worktree",
                ])
            ),
            (
                K::Behind,
                texts(&[
                    meaning(K::Behind),
                    "3 commits to pull",
                    "Pull behind",
                    "Fetch remotes"
                ])
            ),
        ]
    );
    let broken = gh_on(&mut app, "repo:broken");
    assert_eq!(
        broken.last(),
        Some(&(
            K::StatusFailed,
            texts(&[meaning(K::StatusFailed), "Refresh"])
        ))
    );
    let notes = gh_on(&mut app, "repo:notes");
    assert_eq!(
        notes.first(),
        Some(&(K::Hidden, texts(&[meaning(K::Hidden), "Show hidden"])))
    );
    assert_eq!(
        gh_on(&mut app, "group:no-updates"),
        vec![(
            K::Clean,
            texts(&[
                meaning(K::Clean),
                "1 repo",
                "idle",
                "Fold row",
                "Fold subtree"
            ])
        )]
    );
}

fn change(path: &str, staged: Option<&str>, unstaged: Option<&str>) -> FileChange {
    FileChange {
        path: path.into(),
        staged_status: staged.map(str::to_string),
        unstaged_status: unstaged.map(str::to_string),
        untracked: false,
        old_path: None,
    }
}

/// `app` with staged `src/a.rs` (M) and `src/c.rs` (renamed from
/// `old.rs`), unstaged `src/b.rs` (M, reviewed, four comments: one
/// resolved), and untracked `new.ts`.
fn file_rows_app() -> AppState {
    let mut snap = repo("app", SyncStatus::NoUpstream, "");
    let mut renamed = change("src/c.rs", Some("R"), None);
    renamed.old_path = Some("old.rs".into());
    let mut untracked = change("new.ts", None, None);
    untracked.untracked = true;
    snap.changes = vec![
        change("src/a.rs", Some("M"), None),
        change("src/b.rs", None, Some("M")),
        renamed,
        untracked,
    ];
    snap.has_staged = true;
    snap.has_untracked = true;
    let mut app = app_state(&[snap]);
    for (line, body, resolved) in [
        (1, "first\nsecond line", false),
        (2, "fixed", true),
        (3, "third", false),
        (4, "fourth", false),
    ] {
        let key = crate::tui::comments::CommentKey::CommitLine {
            repo: "app".into(),
            sha: "abc".into(),
            path: "src/b.rs".into(),
            line,
            end_line: line,
        };
        app.comment_store =
            crate::tui::comments::put_comment_entry(&app.comment_store, key, body, resolved);
    }
    app.reviewed.insert("file:app:src/b.rs".into());
    app
}

#[test]
fn every_file_row_icon_has_its_fields_and_actions() {
    use IconKind as K;
    let mut app = file_rows_app();
    let actions = ["Stage", "Unstage", "Revert", "Copy entity reference"];
    assert_eq!(
        gh_on(&mut app, "section:app:staged"),
        vec![(
            K::Staged,
            texts(&[meaning(K::Staged), "2 files", "Unstage", "Stash menu"])
        )]
    );
    assert_eq!(
        gh_on(&mut app, "section:app:changes"),
        vec![(
            K::Changes,
            texts(&[meaning(K::Changes), "2 files", "Stage", "Stash menu"])
        )]
    );
    let mut folder = texts(&[meaning(K::Folder), "src", "2 files"]);
    folder.extend(texts(&actions));
    assert_eq!(gh_on(&mut app, "dir:app:src"), vec![(K::Folder, folder)]);
    let devicon = |path: &str, kind: &str| {
        (
            K::FileType,
            texts(&[
                meaning(K::FileType),
                kind,
                path,
                "Open in editor",
                "Open in diff tool",
                "Copy entity reference",
                "View remotely",
            ]),
        )
    };
    assert_eq!(
        gh_on(&mut app, "file:app:src/b.rs"),
        vec![
            devicon("src/b.rs", "rs file"),
            (K::Viewed, texts(&[meaning(K::Viewed), "Mark reviewed"])),
            (
                K::Comment,
                texts(&[
                    meaning(K::Comment),
                    "3 open · 1 resolved",
                    "first",
                    "fixed",
                    "third",
                    "Comment",
                    "Copy comments",
                ])
            ),
            (
                K::StatusModified,
                texts(&[
                    meaning(K::StatusModified),
                    "src/b.rs",
                    "no change",
                    "modified",
                    "Stage",
                    "Revert",
                    "Mark reviewed",
                ])
            ),
        ]
    );
    assert_eq!(
        gh_on(&mut app, "file:app:src/a.rs")[1],
        (
            K::StatusStaged,
            texts(&[
                meaning(K::StatusStaged),
                "src/a.rs",
                "modified",
                "no change",
                "Unstage",
                "Revert",
                "Mark reviewed",
            ])
        )
    );
    assert_eq!(
        gh_on(&mut app, "file:app:src/c.rs")[1],
        (
            K::StatusRenamed,
            texts(&[
                meaning(K::StatusRenamed),
                "old.rs → src/c.rs",
                "renamed",
                "no change",
                "Unstage",
                "Revert",
                "Mark reviewed",
            ])
        )
    );
    assert_eq!(
        gh_on(&mut app, "file:app:new.ts"),
        vec![
            devicon("new.ts", "ts file"),
            (
                K::StatusAdded,
                texts(&[
                    meaning(K::StatusAdded),
                    "new.ts",
                    "no change",
                    "untracked",
                    "Stage",
                    "Revert",
                    "Mark reviewed",
                ])
            ),
        ]
    );
}

/// Resolving every comment turns the mark into the resolved one and
/// keeps the pinned section.
#[test]
fn a_resolved_comment_keeps_its_section() {
    let mut app = file_rows_app();
    focus(&mut app, "file:app:src/b.rs");
    paint(&mut app);
    let hit = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.kind == IconKind::Comment)
        .cloned()
        .expect("comment hit");
    app.pin_icon(&hit);
    for entry in app.comment_store.values_mut() {
        entry.resolved = true;
    }
    let sections = app.open_popover_sections();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].icon, IconKind::CommentResolved);
    assert_eq!(sections[0].lines[1].copy_text(), "0 open · 4 resolved");
}

fn commit_file(status: &str, path: &str) -> crate::tui::drill::CommitFile {
    crate::tui::drill::CommitFile {
        status: status.into(),
        path: path.into(),
        old_path: None,
        stat: None,
    }
}

/// A commit-file list row has the same popovers. Stage, Unstage, and
/// Revert show their gate reason; a focused-row change closes the
/// pinned popover.
#[test]
fn commit_file_rows_share_the_file_popovers_and_own_their_row() {
    use IconKind as K;
    let mut app = app();
    app.commit_tree_mode = true;
    app.open_commit_files(
        "app".into(),
        crate::tui::drill::CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        },
        vec![
            commit_file("A", "src/lib.rs"),
            commit_file("M", "src/main.rs"),
        ],
    );
    app.focus = FocusPane::Right;
    app.dispatch(Action::Move(1));
    assert_eq!(
        app.focused_commit_file_row().expect("row").id,
        "file:src/lib.rs"
    );
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned(), "gh pins on a commit-file row");
    assert_eq!(app.popover_owner().row, "file:src/lib.rs");
    let sections: Vec<(IconKind, Vec<String>)> = app
        .open_popover_sections()
        .into_iter()
        .map(|section| {
            let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
            (section.icon, lines)
        })
        .collect();
    assert_eq!(sections[0].0, K::FileType);
    assert_eq!(
        sections[1],
        (
            K::StatusAdded,
            texts(&[
                meaning(K::StatusAdded),
                "src/lib.rs",
                "added",
                "Stage",
                "Unstage",
                "Revert",
                "Mark reviewed",
            ])
        )
    );
    for action in [Action::Stage, Action::Unstage, Action::Revert] {
        let command = command_for(&action).expect("row");
        assert_eq!(
            command.scope,
            crate::tui::command_palette::CommandScope::NoHighlight
        );
        assert!(
            app.palette_disabled_reason(command).is_some(),
            "{action:?} is refused on a commit-file list"
        );
    }
    let terminal = paint(&mut app);
    let painted = app.layout.popover.clone().expect("painted");
    let buf = terminal.backend().buffer();
    let stage = command_for(&Action::Stage).unwrap();
    let reason = app.palette_disabled_reason(stage).unwrap();
    let text: String = (painted.rect.y..painted.rect.bottom())
        .map(|y| {
            (painted.rect.x..painted.rect.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains(&format!("Stage  {reason}")),
        "{reason:?} beside Stage:\n{text}"
    );
    assert!(app.popover_pinned(), "paint keeps it");

    app.dispatch(Action::PopoverClose);
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned());
    if let Some(tab_cursor) = match &mut app.drill {
        crate::tui::drill::DrillView::Files { cursor, .. } => Some(cursor),
        _ => None,
    } {
        *tab_cursor += 1;
    }
    assert!(
        !app.popover_pinned(),
        "another focused commit-file row closes the popover"
    );
    paint(&mut app);
    assert!(app.popover.is_none());
}

#[test]
fn a_disabled_action_line_shows_and_keeps_the_gate_reason() {
    let mut app = branch_rows_app();
    // No HEAD sha: the compare gate refuses Diff vs default.
    app.snapshot.repos[0].head.clear();
    app.rebuild_rows();
    focus(&mut app, "checkout:app");
    let diff = command_for(&Action::CompareVsDefault).expect("diff row");
    let reason = app.palette_disabled_reason(diff).expect("gate refuses");
    app.dispatch(Action::PopoverOpenFocused);
    let terminal = paint(&mut app);
    let painted = app.layout.popover.clone().expect("painted");
    let buf = terminal.backend().buffer();
    let text: String = (painted.rect.y..painted.rect.bottom())
        .map(|y| {
            (painted.rect.x..painted.rect.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains(&format!("{}  {reason}", diff.title)),
        "{reason:?} beside the line:\n{text}"
    );
    let sections = app.open_popover_sections();
    let line = flat_lines(&sections)
        .iter()
        .position(|line| line.copy_text() == diff.title)
        .expect("diff line");
    app.popover.as_mut().expect("pinned").focus_line = line;
    assert_eq!(app.dispatch(Action::PopoverRun), Effect::None);
    assert!(app.popover_pinned(), "a refused line keeps the popover");
    assert_eq!(app.status, reason.as_str());
}

#[test]
fn a_peek_on_the_second_segment_of_a_kind_stays_open() {
    let mut app = branch_rows_app();
    paint(&mut app);
    let workspace: Vec<IconHit> = app
        .layout
        .icon_hits
        .iter()
        .filter(|hit| hit.kind == IconKind::Workspace)
        .cloned()
        .collect();
    assert_eq!(workspace.len(), 2, "glyph and summary: {workspace:?}");
    let summary = &workspace[1];
    assert!(summary.width > 1, "the summary text is one hit");
    peek_at(&mut app, summary, Instant::now());
    paint(&mut app);
    let popover = app.popover.as_ref().expect("peek survives paint");
    assert_eq!(
        popover.anchor,
        Some(summary.rect()),
        "hangs from the summary"
    );
    assert_eq!(app.open_popover_sections().len(), 1);

    // Pinned from the summary, it keeps that anchor across paints.
    app.close_popover();
    app.pin_icon(summary);
    paint(&mut app);
    assert_eq!(app.popover.as_ref().unwrap().anchor, Some(summary.rect()));
}

#[test]
fn fold_lines_run_like_z_and_zz() {
    let mut app = branch_rows_app();
    focus(&mut app, "repo:app");
    let mut twin = app.clone();
    paint(&mut app);
    let hit = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.kind == IconKind::WorktreeCount)
        .cloned()
        .expect("N wt hit");
    app.pin_icon(&hit);
    assert_eq!(app.popover_run_action(), Some(Action::FoldToggle));
    app.dispatch(Action::PopoverRun);
    twin.dispatch(Action::FoldToggle);
    assert_eq!(app.rows, twin.rows, "Fold row folds like z");
    assert!(app.z_pending_at.is_none(), "and arms no zz");
    assert!(app.popover.is_none());

    let mut twin = app.clone();
    paint(&mut app);
    let hit = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.kind == IconKind::WorktreeCount)
        .cloned()
        .expect("N wt hit");
    app.pin_icon(&hit);
    app.dispatch(Action::PopoverMove(1));
    assert_eq!(app.popover_run_action(), Some(Action::FoldToggleSubtree));
    app.dispatch(Action::PopoverRun);
    twin.dispatch(Action::FoldToggle);
    twin.dispatch(Action::FoldToggleSubtree);
    assert_eq!(app.rows, twin.rows, "Fold subtree runs like zz");
}

#[test]
fn a_prior_ctrl_click_with_no_pr_never_turns_an_icon_click_into_a_drill() {
    let mut app = app();
    paint(&mut app);
    let hit = sync_hit(&app, "repo:app");
    // A badge on the same cell whose checkout has no PR target: the
    // Ctrl+click is a plain click that arms `last_click`.
    app.layout.icon_hits.insert(
        0,
        IconHit {
            kind: IconKind::PrInReview,
            target: IconTarget::PullRequest(PathBuf::from("gone")),
            ..hit.clone()
        },
    );
    app.dispatch(Action::CtrlClick {
        col: hit.x,
        row: hit.y,
    });
    assert!(app.last_click.is_some(), "the no-PR Ctrl+click armed it");
    assert!(app.popover.is_none(), "Ctrl+click pins nothing");
    app.layout.icon_hits.remove(0);
    assert_eq!(
        app.dispatch(Action::Click {
            col: hit.x,
            row: hit.y
        }),
        Effect::LoadRightPane
    );
    assert_eq!(app.focus, FocusPane::Left, "no drill");
    assert!(app.popover_pinned());
}

const G_HEAD: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const G_ROOT: &str = "bbb2222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const G_STASH: &str = "ccc3333ccccccccccccccccccccccccccccccccc";
/// Author date of every graph fixture entry: old enough to paint as a
/// local timestamp.
const G_WHEN: i64 = 1_700_000_000;

fn g_worktree(path: &str, head: &str, branch: &str, current: bool) -> Worktree {
    Worktree {
        path: path.into(),
        head_id: Some(head.into()),
        branch: Some(branch.into()),
        hidden: false,
        is_current: current,
    }
}

fn g_stash(id: &str, stash_ref: &str) -> Stash {
    Stash {
        id: id.into(),
        stash_ref: stash_ref.into(),
        subject: "WIP on main".into(),
        body: String::new(),
        author_name: "Ada".into(),
        author_date_unix: G_WHEN,
        parent_id: Some(G_HEAD.into()),
        ..Stash::default()
    }
}

/// Graph of `app`: uncommitted (0), a stash on HEAD (1), HEAD (2) with
/// the linked `app/.worktrees/feat` mark on its spacer and an open
/// comment, the root (3), and the worktree row `app/.worktrees/old`
/// (4). The graph pane is focused on HEAD.
fn graph_app() -> AppState {
    let mut app = app();
    let commit = |id: &str, subject: &str, parents: &[&str], refs: &[&str]| Commit {
        id: id.into(),
        subject: subject.into(),
        parents: parents.iter().map(|p| (*p).to_string()).collect(),
        refs: refs.iter().map(|r| (*r).into()).collect(),
        author_name: "Ada".into(),
        author_date_unix: G_WHEN,
        ..Commit::default()
    };
    app.graph = Some(GraphModel {
        commits: vec![
            commit(G_HEAD, "add graph", &[G_ROOT], &["main"]),
            commit(G_ROOT, "root", &[], &[]),
        ],
        stashes: vec![g_stash(G_STASH, "stash@{0}")],
        worktrees: vec![
            g_worktree("app", G_HEAD, "main", true),
            g_worktree("app/.worktrees/feat", G_HEAD, "feat", false),
            g_worktree("app/.worktrees/old", "elsewhere", "old", false),
        ],
        head_id: Some(G_HEAD.into()),
        sync: Some(SyncState {
            branch: "main".into(),
            status: GraphSyncStatus::Ahead,
            ahead: 1,
            behind: 0,
        }),
        uncommitted: Some(true),
        ..GraphModel::default()
    });
    app.graph_identity = Some(("app".into(), G_HEAD.into()));
    app.comment_store = put_comment(
        &app.comment_store,
        CommentKey::Commit {
            repo: "app".into(),
            sha: G_HEAD.into(),
        },
        "check this\nsecond line",
    );
    app.focus = FocusPane::Right;
    app.graph_cursor = 2;
    app
}

/// `gh` on graph row `index`: each section's icon and line texts.
fn gh_on_graph(app: &mut AppState, index: usize) -> Vec<(IconKind, Vec<String>)> {
    app.close_popover();
    app.graph_cursor = index;
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned(), "gh pins on graph row {index}");
    app.open_popover_sections()
        .into_iter()
        .map(|section| {
            let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
            (section.icon, lines)
        })
        .collect()
}

fn title(action: Action) -> &'static str {
    command_for(&action).expect("palette row").title
}

/// `gh` lists the row's icons, not its commit hash / date / author
/// text: commit popovers open by mouse only.
#[test]
fn gh_on_a_graph_row_lists_no_commit_popover() {
    let mut app = graph_app();
    let rows = app.graph.as_ref().expect("graph").visible_rows().len();
    assert!(rows > 3);
    for index in 0..rows {
        app.graph_cursor = index;
        let icons = app.focused_row_icons();
        assert!(
            !icons
                .iter()
                .any(|(_, target)| matches!(target, IconTarget::Commit { .. })),
            "row {index}: {icons:?}"
        );
    }
}

#[test]
fn every_graph_row_icon_has_its_fields_and_actions() {
    use IconKind as K;
    let mut app = graph_app();
    let date = format_local_timestamp(G_WHEN);
    let date = date.as_str();
    let head = gh_on_graph(&mut app, 2);
    assert_eq!(
        head,
        vec![
            (
                K::GraphHeadCommit,
                texts(&[
                    meaning(K::GraphHeadCommit),
                    "aaa1111",
                    "add graph",
                    "Ada",
                    date,
                    "bbb2222",
                    "main",
                    "on main",
                    "^1 v0",
                    title(Action::CompareCommitVsParent),
                    title(Action::GraphCreateBranch),
                    title(Action::ViewRemotely),
                ])
            ),
            (
                K::Comment,
                texts(&[
                    meaning(K::Comment),
                    "1 open · 0 resolved",
                    "check this",
                    title(Action::CommentStart),
                    title(Action::ExportComments),
                ])
            ),
            // A mark on the HEAD spacer: fields only. Its keys would act
            // on the focused commit row, not on the worktree.
            (
                K::LinkedWorktree,
                texts(&[meaning(K::LinkedWorktree), "app/.worktrees/feat", "feat"])
            ),
            // The spacer's `[+main]` chip: the chip, then its checkout
            // mark run.
            (
                K::ChipDefault,
                texts(&[
                    meaning(K::ChipDefault),
                    "main",
                    "local branch",
                    "yes",
                    title(Action::GraphFocusBranches),
                    title(Action::CompareVsBranch),
                    title(Action::GraphCheckout),
                    title(Action::ViewRemotely),
                ])
            ),
            (
                K::ChipCheckout,
                texts(&[
                    meaning(K::ChipCheckout),
                    "on main",
                    title(Action::Pull),
                    title(Action::Push),
                    title(Action::Fetch),
                    title(Action::ViewRemotely),
                ])
            ),
        ]
    );
    assert_eq!(app.popover_owner().row, format!("commit:{G_HEAD}"));
    assert_eq!(
        gh_on_graph(&mut app, 1),
        vec![(
            K::GraphStash,
            texts(&[
                meaning(K::GraphStash),
                "stash@{0}",
                "WIP on main",
                "Ada",
                date,
                "aaa1111",
                title(Action::GraphStashApply),
                title(Action::GraphStashPop),
                title(Action::GraphStashDrop),
            ])
        )]
    );
    assert_eq!(
        gh_on_graph(&mut app, 0),
        vec![(
            K::GraphUncommitted,
            texts(&[
                meaning(K::GraphUncommitted),
                "uncommitted changes",
                "0",
                "1",
                "0",
                title(Action::StashMenu),
                title(Action::Refresh),
            ])
        )]
    );
    assert_eq!(
        gh_on_graph(&mut app, 3),
        vec![(
            K::GraphCommit,
            texts(&[
                meaning(K::GraphCommit),
                "bbb2222",
                "root",
                "Ada",
                date,
                "none (root commit)",
                title(Action::CompareCommitVsParent),
                title(Action::GraphCreateBranch),
                title(Action::GraphCheckout),
                title(Action::GraphMerge),
                title(Action::ViewRemotely),
            ])
        )]
    );
    // Its own worktree row keeps the worktree actions.
    let row = gh_on_graph(&mut app, 4);
    assert_eq!(
        row,
        vec![(
            K::LinkedWorktree,
            texts(&[
                meaning(K::LinkedWorktree),
                "app/.worktrees/old",
                "old",
                title(Action::OpenPullRequest),
                title(Action::ViewRemotely),
                title(Action::RemoveWorktree),
                title(Action::CopyEntityReference),
            ])
        )]
    );
    // Disabled lines keep their gate reason: Drop stash off a stash row.
    let drop = command_for(&Action::GraphStashDrop).unwrap();
    assert!(app.palette_disabled_reason(drop).is_some());
    app.graph_cursor = 1;
    app.close_popover();
    assert_eq!(app.palette_disabled_reason(drop), None);
}

#[test]
fn graph_dates_read_the_given_clock() {
    let app = graph_app();
    let date = |now: i64| -> String {
        let sections = popover_sections(
            &app,
            &[(
                IconKind::GraphHeadCommit,
                IconTarget::GraphRow(format!("commit:{G_HEAD}")),
            )],
            now,
        );
        let lines = flat_lines(&sections);
        match lines
            .iter()
            .find(|line| matches!(line, PopoverLine::Field { label, .. } if *label == "date"))
        {
            Some(PopoverLine::Field { value, .. }) => value.clone(),
            _ => panic!("no date field"),
        }
    };
    let local = format_local_timestamp(G_WHEN);
    // Two minutes old: the local time, then the relative age.
    let relative = format_relative_date(G_WHEN, G_WHEN + 120);
    assert_ne!(relative, local);
    assert_eq!(date(G_WHEN + 120), format!("{local} ({relative})"));
    // A day old: the graph paints the local time only.
    assert_eq!(date(G_WHEN + 86_400), local);
}

#[test]
fn a_graph_popover_belongs_to_its_row_not_the_cursor_index() {
    let mut app = graph_app();
    app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned());
    // A reload adds a stash: HEAD moves to index 3 and the cursor
    // follows it. The popover stays on HEAD.
    let model = app.graph.as_mut().unwrap();
    model.stashes.insert(0, g_stash("ddd4444", "stash@{0}"));
    model.stashes[1].stash_ref = "stash@{1}".into();
    app.graph_cursor = 3;
    assert!(app.popover_pinned(), "same row, new index");
    paint(&mut app);
    assert!(app.popover.is_some());
    // The cursor index stays put but names another row now: closed.
    app.graph_cursor = 2;
    assert!(!app.popover_pinned(), "index 2 is a stash now");
    paint(&mut app);
    assert!(app.popover.is_none());
}

#[test]
fn graph_icons_record_hits_and_a_click_pins_the_node_alone() {
    use IconKind as K;
    let mut app = graph_app();
    let terminal = paint(&mut app);
    let graph: Vec<(IconKind, IconTarget)> = app
        .layout
        .icon_hits
        .iter()
        .filter(|hit| hit.x >= app.layout.right_x)
        .filter(|hit| {
            // Chips, the sync header, and the footer hint: see
            // `graph_chips_header_and_footer_record_hits_and_pin`.
            matches!(
                hit.target,
                IconTarget::GraphRow(_) | IconTarget::GraphWorktree(_)
            )
        })
        .map(|hit| (hit.kind, hit.target.clone()))
        .collect();
    let row = |id: &str| IconTarget::GraphRow(id.into());
    assert_eq!(
        graph,
        vec![
            (K::GraphUncommitted, row("uncommitted")),
            (K::GraphStash, row(&format!("stash:{G_STASH}"))),
            (K::GraphHeadCommit, row(&format!("commit:{G_HEAD}"))),
            (K::Comment, row(&format!("commit:{G_HEAD}"))),
            (
                K::LinkedWorktree,
                IconTarget::GraphWorktree("app/.worktrees/feat".into())
            ),
            (K::GraphCommit, row(&format!("commit:{G_ROOT}"))),
            (
                K::LinkedWorktree,
                IconTarget::GraphWorktree("app/.worktrees/old".into())
            ),
        ]
    );
    let buf = terminal.backend().buffer();
    let hit_of = |app: &AppState, kind: IconKind| {
        app.layout
            .icon_hits
            .iter()
            .find(|hit| hit.kind == kind)
            .cloned()
            .expect("hit")
    };
    let comment = hit_of(&app, K::Comment);
    let palette = app.theme.palette();
    assert_eq!(buf[(comment.x, comment.y)].symbol(), "\"");
    assert_eq!(buf[(comment.x, comment.y)].fg, palette.heading);
    let dirty = hit_of(&app, K::GraphUncommitted);
    assert_eq!(buf[(dirty.x, dirty.y)].symbol(), "o");
    assert_eq!(buf[(dirty.x, dirty.y)].fg, palette.modified);
    let mark = hit_of(&app, K::LinkedWorktree);
    assert_eq!(buf[(mark.x, mark.y)].symbol(), "L");
    assert_eq!(buf[(mark.x, mark.y)].fg, palette.heading);

    // A click on the stash node selects its row and pins its popover.
    let stash = hit_of(&app, K::GraphStash);
    assert_eq!(buf[(stash.x, stash.y)].symbol(), "s");
    app.dispatch(Action::Click {
        col: stash.x,
        row: stash.y,
    });
    assert_eq!(app.graph_cursor, 1);
    assert!(app.last_click.is_none(), "an icon click never drills");
    assert!(app.popover_pinned());
    assert_eq!(
        app.popover.as_ref().unwrap().targets,
        vec![(K::GraphStash, row(&format!("stash:{G_STASH}")))]
    );
    // Enter on the landing line (Apply stash) runs the existing action.
    assert_eq!(
        app.popover_run_action(),
        Some(Action::GraphStashApply),
        "landing line"
    );
    app.dispatch(Action::PopoverClose);
    // The worktree mark on the HEAD spacer selects HEAD.
    paint(&mut app);
    let mark = hit_of(&app, K::LinkedWorktree);
    app.dispatch(Action::Click {
        col: mark.x,
        row: mark.y,
    });
    assert_eq!(app.graph_cursor, 2);
    assert_eq!(
        app.open_popover_sections()[0].target,
        IconTarget::GraphWorktree("app/.worktrees/feat".into())
    );
}

/// [`graph_app`] with HEAD on `main` synced with `origin/main`, a
/// `feat` branch, and a `v0` tag on the root.
fn chip_graph_app() -> AppState {
    let mut app = graph_app();
    let model = app.graph.as_mut().unwrap();
    model.commits[0].refs = vec![
        "main".into(),
        GraphRef::remote("origin/main"),
        "feat".into(),
    ];
    model.commits[1].refs = vec![GraphRef::tag("v0")];
    app
}

fn chip_hit(app: &AppState, kind: IconKind, row: &str) -> IconHit {
    app.layout
        .icon_hits
        .iter()
        .find(|hit| {
            hit.kind == kind
                && matches!(&hit.target, IconTarget::GraphChip { row: id, .. } if id == row)
        })
        .cloned()
        .unwrap_or_else(|| panic!("no {kind:?} chip on {row}: {:?}", app.layout.icon_hits))
}

/// Text rows of the painted popover box, trailing spaces trimmed.
fn popover_rows(app: &AppState) -> Vec<String> {
    let rect = app.layout.popover.as_ref().expect("painted popover").rect;
    (rect.y..rect.bottom())
        .map(|y| {
            (rect.x..rect.right())
                .map(|x| app.painted_frame[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn section_texts(app: &AppState) -> Vec<(IconKind, Vec<String>)> {
    app.open_popover_sections()
        .into_iter()
        .map(|section| {
            let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
            (section.icon, lines)
        })
        .collect()
}

#[test]
fn graph_chips_header_and_footer_record_hits_and_pin() {
    use IconKind as K;
    let mut app = chip_graph_app();
    let terminal = paint(&mut app);
    let buf = terminal.backend().buffer();
    let head = format!("commit:{G_HEAD}");
    let root = format!("commit:{G_ROOT}");
    let palette = app.theme.palette();

    // The header is one hit, coloured with the theme.
    let header = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.target == IconTarget::GraphSync)
        .cloned()
        .expect("sync header hit");
    assert_eq!(header.kind, K::Ahead);
    let text: String = (header.x..header.x + header.width)
        .map(|x| buf[(x, header.y)].symbol())
        .collect();
    assert_eq!(text, "main ^1");
    assert_eq!(buf[(header.x, header.y)].fg, palette.branch_default);
    assert_eq!(buf[(header.x + 5, header.y)].fg, palette.added);

    // The marks run on the HEAD spacer is one hit; a click selects HEAD
    // and pins two sections.
    app.graph_cursor = 3;
    paint(&mut app);
    let marks = chip_hit(&app, K::ChipCheckout, &head);
    assert_eq!(buf[(marks.x, marks.y)].symbol(), "+");
    assert_eq!(marks.width, 2, "checkout + sync: one hit");
    app.dispatch(Action::Click {
        col: marks.x,
        row: marks.y,
    });
    assert_eq!(app.graph_cursor, 2, "the spacer chip selects its commit");
    assert!(app.last_click.is_none(), "an icon click never drills");
    assert_eq!(
        section_texts(&app),
        vec![
            (
                K::ChipCheckout,
                texts(&[
                    meaning(K::ChipCheckout),
                    "on main",
                    title(Action::Pull),
                    title(Action::Push),
                    title(Action::Fetch),
                    title(Action::ViewRemotely),
                ])
            ),
            (
                K::ChipSynced,
                texts(&[meaning(K::ChipSynced), "main", "in sync with origin/main"])
            ),
        ]
    );
    // Off the tree the gates refuse Pull: the painted line keeps the
    // reason beside the title.
    let pull = command_for(&Action::Pull).unwrap();
    let reason = app
        .palette_disabled_reason(pull)
        .expect("gated off the tree");
    assert!(reason.contains("to pull"), "{reason}");
    paint(&mut app);
    let line = popover_rows(&app)
        .into_iter()
        .find(|line| line.contains(pull.title))
        .expect("painted Pull line");
    assert!(line.contains(&reason), "{line:?}");
    app.dispatch(Action::PopoverClose);

    // A tag chip on the root spacer: its own commit, so Checkout
    // commit refs acts on it.
    paint(&mut app);
    let tag = chip_hit(&app, K::ChipTag, &root);
    app.dispatch(Action::Click {
        col: tag.x,
        row: tag.y,
    });
    assert_eq!(app.graph_cursor, 3);
    assert_eq!(
        section_texts(&app),
        vec![(
            K::ChipTag,
            texts(&[
                meaning(K::ChipTag),
                "v0",
                "tag",
                title(Action::GraphCheckout),
                title(Action::ViewRemotely),
            ])
        )]
    );
    assert_eq!(app.popover_run_action(), Some(Action::GraphCheckout));
    app.dispatch(Action::PopoverClose);

    // A click on the header pins the sync section.
    paint(&mut app);
    app.dispatch(Action::Click {
        col: header.x + 1,
        row: header.y,
    });
    assert_eq!(
        section_texts(&app),
        vec![(
            K::Ahead,
            texts(&[
                meaning(K::Ahead),
                "main",
                "1 commit not pushed",
                "0 commits to pull",
                "ahead",
                title(Action::Pull),
                title(Action::Push),
                title(Action::Fetch),
                title(Action::ViewRemotely),
            ])
        )]
    );
    app.dispatch(Action::PopoverClose);

    // The uncommitted row's footer lists HEAD's chips: they name HEAD's
    // refs, and no line acts on the focused commit.
    app.graph_cursor = 0;
    paint(&mut app);
    let footer = chip_hit(&app, K::ChipLocal, "uncommitted");
    app.pin_icon(&footer);
    assert_eq!(
        section_texts(&app),
        vec![(
            K::ChipLocal,
            texts(&[
                meaning(K::ChipLocal),
                "feat",
                "local branch",
                title(Action::GraphFocusBranches),
                title(Action::CompareVsBranch),
                title(Action::ViewRemotely),
            ])
        )]
    );
}

fn view(checkout: &str, target: RemoteTarget) -> Effect {
    Effect::ViewRemotely {
        checkout: PathBuf::from(checkout),
        target,
    }
}

/// Focus the View remotely line of the pinned popover.
fn focus_view_remotely_line(app: &mut AppState) {
    let sections = app.open_popover_sections();
    let line = flat_lines(&sections)
        .iter()
        .position(|line| line.copy_text() == title(Action::ViewRemotely))
        .expect("View remotely line");
    app.popover.as_mut().expect("pinned").focus_line = line;
    assert_eq!(app.popover_run_action(), Some(Action::ViewRemotely));
}

/// The View remotely line opens its section's entity: a tag chip on a
/// commit row opens the tag, not the commit. Enter closes the popover.
#[test]
fn a_tag_chip_view_remotely_line_opens_the_tag_not_the_commit() {
    let mut app = chip_graph_app();
    focus(&mut app, "repo:app");
    app.graph_cursor = 3;
    paint(&mut app);
    let root = format!("commit:{G_ROOT}");
    let tag = chip_hit(&app, IconKind::ChipTag, &root);
    app.pin_icon(&tag);
    assert_eq!(title(Action::ViewRemotely), "View remotely");
    focus_view_remotely_line(&mut app);
    let mut twin = app.clone();
    twin.close_popover();
    assert_eq!(
        twin.dispatch(Action::ViewRemotely),
        view("app", RemoteTarget::Commit(G_ROOT.into())),
        "the key on the row opens the commit"
    );
    assert_eq!(
        app.dispatch(Action::PopoverRun),
        view("app", RemoteTarget::Tag("v0".into()))
    );
    assert!(app.popover.is_none(), "Enter closes, then runs");
}

/// A release on the View remotely line runs it on the line's section,
/// as a press then release on any action line does.
#[test]
fn a_release_on_the_view_remotely_line_opens_the_section_target() {
    use super::super::super::event_pump::{classify_busy_dispatch, BusyAction};
    let mut app = graph_app();
    focus(&mut app, "repo:app");
    // The own row of worktree `app/.worktrees/old` on branch `old`.
    app.graph_cursor = 4;
    app.dispatch(Action::PopoverOpenFocused);
    paint(&mut app);
    let painted = app.layout.popover.clone().expect("painted");
    let sections = app.open_popover_sections();
    let lines = flat_lines(&sections);
    let (y, _) = *painted
        .lines
        .iter()
        .find(|(_, index)| lines[*index].copy_text() == title(Action::ViewRemotely))
        .expect("View remotely line");
    app.dispatch(Action::Click {
        col: painted.inner.x + 2,
        row: y,
    });
    assert_eq!(app.popover_release_action(), Some(Action::ViewRemotely));
    assert_eq!(
        classify_busy_dispatch(&Action::Release, app.popover_release_action().as_ref()),
        BusyAction::Handle
    );
    assert_eq!(
        app.dispatch(Action::Release),
        view("app/.worktrees/old", RemoteTarget::Branch("old".into()))
    );
    assert!(app.popover.is_none());
}

/// The uncommitted row's footer shows HEAD's chips. The row itself is
/// local only, but a chip's View remotely line opens the chip's branch
/// and paints no gate reason.
#[test]
fn a_footer_chip_line_is_not_refused_as_the_uncommitted_row() {
    let mut app = chip_graph_app();
    focus(&mut app, "repo:app");
    app.graph_cursor = 0;
    paint(&mut app);
    let footer = chip_hit(&app, IconKind::ChipLocal, "uncommitted");
    app.pin_icon(&footer);
    let command = command_for(&Action::ViewRemotely).expect("palette row");
    assert_eq!(
        app.palette_disabled_reason(command).as_deref(),
        Some(UNCOMMITTED_LOCAL_ONLY),
        "the list focus refuses"
    );
    assert_eq!(
        app.popover_line_refusal(command, footer.kind, &footer.target),
        None
    );
    paint(&mut app);
    let line = popover_rows(&app)
        .into_iter()
        .find(|line| line.contains(command.title))
        .expect("painted View remotely line");
    assert!(!line.contains(UNCOMMITTED_LOCAL_ONLY), "{line:?}");
    focus_view_remotely_line(&mut app);
    assert_eq!(
        app.dispatch(Action::PopoverRun),
        view("app", RemoteTarget::Branch("feat".into()))
    );

    // A highlight still refuses it, like every NoHighlight line.
    app.diff_visual_anchor = Some(0);
    assert_eq!(
        app.popover_line_refusal(command, footer.kind, &footer.target)
            .as_deref(),
        Some("exit highlight first (Esc)")
    );
}

#[test]
fn every_icon_target_names_its_own_remote_entity() {
    use IconKind as K;
    let mut app = chip_graph_app();
    focus(&mut app, "repo:app");
    app.graph.as_mut().unwrap().commits[0]
        .refs
        .push(GraphRef::remote("origin/side"));
    let head = format!("commit:{G_HEAD}");
    let root = format!("commit:{G_ROOT}");
    let chip = |chip: PartTarget, row: &str| IconTarget::GraphChip {
        row: row.to_string(),
        chip,
    };
    let at = |app: &mut AppState, kind: IconKind, target: IconTarget| {
        app.status = StatusMessage::default();
        app.view_remotely_at(kind, &target)
    };
    let remote = chip(
        PartTarget::Chip(vec![GraphRef::remote("origin/side")]),
        &head,
    );
    assert_eq!(
        at(&mut app, K::ChipRemote, remote),
        view("app", RemoteTarget::RemoteBranch("origin/side".into())),
        "a remote-tracking chip"
    );
    let merged = chip(
        PartTarget::Chip(vec![
            GraphRef::local("main"),
            GraphRef::remote("origin/main"),
        ]),
        &head,
    );
    assert_eq!(
        at(&mut app, K::ChipDefault, merged),
        view("app", RemoteTarget::Branch("main".into())),
        "a local and remote chip opens the local branch"
    );
    let marks = PartTarget::Marks {
        branch: "main".into(),
        checkout: true,
        remote: Some("origin/main".into()),
    };
    assert_eq!(
        at(&mut app, K::ChipCheckout, chip(marks, &head)),
        view("app", RemoteTarget::Branch("main".into())),
        "a checkout mark run"
    );
    assert_eq!(
        at(
            &mut app,
            K::ChipDetachedHead,
            chip(PartTarget::DetachedHead, "uncommitted")
        ),
        view("app", RemoteTarget::Commit(G_HEAD.into())),
        "[HEAD] in the uncommitted footer is HEAD's commit"
    );
    assert_eq!(
        at(
            &mut app,
            K::ChipDetachedHead,
            chip(PartTarget::DetachedHead, &root)
        ),
        view("app", RemoteTarget::Commit(G_ROOT.into())),
        "[HEAD] on a commit row is that row's commit"
    );
    assert_eq!(
        at(
            &mut app,
            K::LinkedWorktree,
            IconTarget::GraphWorktree("app/.worktrees/feat".into())
        ),
        view("app/.worktrees/feat", RemoteTarget::Branch("feat".into())),
        "a worktree mark opens that worktree's branch"
    );
    assert_eq!(
        at(&mut app, K::Ahead, IconTarget::GraphSync),
        view("app", RemoteTarget::Branch("main".into()))
    );
    // A target on the row itself is the focused row: HEAD's commit.
    assert_eq!(
        at(
            &mut app,
            K::GraphHeadCommit,
            IconTarget::GraphRow(head.clone())
        ),
        view("app", RemoteTarget::Commit(G_HEAD.into()))
    );
    assert!(app.status.is_empty());
    assert_eq!(
        at(
            &mut app,
            K::LinkedWorktree,
            IconTarget::GraphWorktree("gone".into())
        ),
        Effect::None
    );
    assert_eq!(app.status, NOTHING_TO_VIEW);
    assert_eq!(
        at(
            &mut app,
            K::ChipDetachedHead,
            chip(PartTarget::DetachedHead, "stash:gone")
        ),
        Effect::None,
        "[HEAD] on a row with no commit"
    );

    // A detached sync header opens HEAD's commit.
    app.graph.as_mut().unwrap().sync.as_mut().unwrap().branch = DETACHED_HEAD_BRANCH.into();
    assert_eq!(
        at(&mut app, K::Ahead, IconTarget::GraphSync),
        view("app", RemoteTarget::Commit(G_HEAD.into()))
    );
}

/// Run the View remotely line of the pinned section painted as `kind`.
fn run_view_remotely_in(app: &mut AppState, kind: IconKind) -> Effect {
    let sections = app.open_popover_sections();
    let line = super::section_lines(&sections)
        .position(|(_, section, line)| {
            section.icon == kind && line.copy_text() == title(Action::ViewRemotely)
        })
        .unwrap_or_else(|| panic!("no View remotely line in {kind:?}"));
    app.popover.as_mut().expect("pinned").focus_line = line;
    app.dispatch(Action::PopoverRun)
}

/// A one-checkout repo row paints its checkout's Branch glyph next to
/// the Repo glyph: the Branch line opens the branch, the Repo line the
/// repository home.
#[test]
fn a_one_checkout_repo_row_opens_the_branch_or_the_repo_by_section() {
    use IconKind as K;
    let mut app = app();
    let icons: Vec<IconKind> = gh_on(&mut app, "repo:lib")
        .into_iter()
        .map(|(icon, _)| icon)
        .collect();
    assert!(
        icons.contains(&K::Branch) && icons.contains(&K::Repo),
        "{icons:?}"
    );
    assert_eq!(
        run_view_remotely_in(&mut app, K::Branch),
        view("lib", RemoteTarget::Branch("feature".into()))
    );
    assert!(app.popover.is_none());
    gh_on(&mut app, "repo:lib");
    assert_eq!(
        run_view_remotely_in(&mut app, K::Repo),
        view("lib", RemoteTarget::Repo)
    );
    let command = command_for(&Action::ViewRemotely).expect("palette row");
    let target = IconTarget::TreeRow("repo:lib".into());
    assert_eq!(app.popover_line_refusal(command, K::Branch, &target), None);

    // A repo row whose one checkout is a linked worktree paints the
    // Linked worktree glyph: its line opens that checkout's branch.
    let mut wt = repo("wt", SyncStatus::Ahead, "ahead by 1 commits");
    wt.branch = "side".into();
    wt.checkout_kind = CheckoutKind::Linked;
    wt.primary_repo = Some("elsewhere".into());
    let mut app = app_state(&[wt]);
    let row = app
        .rows
        .iter()
        .find(|row| row.repo.as_deref() == Some("wt"));
    let row = row.expect("wt row");
    assert_eq!(row.kind, NodeKind::Repo);
    let id = row.id.clone();
    let sections = gh_on(&mut app, &id);
    assert!(
        sections.iter().any(|(icon, _)| *icon == K::LinkedWorktree),
        "{id}: {sections:?}"
    );
    assert_eq!(
        run_view_remotely_in(&mut app, K::LinkedWorktree),
        view("wt", RemoteTarget::Branch("side".into()))
    );
}

/// Each mark, `[HEAD]`, and sync section ends with View remotely, which
/// opens the branch or commit it names.
#[test]
fn marks_head_and_sync_sections_open_their_own_entity() {
    use IconKind as K;
    let mut app = chip_graph_app();
    focus(&mut app, "repo:app");
    app.graph_cursor = 2;
    paint(&mut app);
    let head = format!("commit:{G_HEAD}");
    let checkout = chip_hit(&app, K::ChipCheckout, &head);
    app.pin_icon(&checkout);
    assert_eq!(
        run_view_remotely_in(&mut app, K::ChipCheckout),
        view("app", RemoteTarget::Branch("main".into()))
    );
    paint(&mut app);
    let sync = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.target == IconTarget::GraphSync)
        .cloned()
        .expect("sync header hit");
    app.pin_icon(&sync);
    assert_eq!(
        run_view_remotely_in(&mut app, sync.kind),
        view("app", RemoteTarget::Branch("main".into()))
    );
}

#[test]
fn overflow_detached_and_more_lines_popovers_read_live_state() {
    use IconKind as K;
    let mut app = chip_graph_app();
    let head = format!("commit:{G_HEAD}");
    let chip = |name: &str| PartTarget::Chip(vec![GraphRef::local(name)]);
    let sections = |app: &AppState, kind: IconKind, chip: PartTarget, row: &str| {
        popover_sections(
            app,
            &[(
                kind,
                IconTarget::GraphChip {
                    row: row.to_string(),
                    chip,
                },
            )],
            0,
        )
        .into_iter()
        .map(|section| {
            let lines: Vec<String> = section.lines.iter().map(PopoverLine::copy_text).collect();
            (section.icon, lines)
        })
        .collect::<Vec<_>>()
    };
    // `[+N]` lists every hidden chip with its kind.
    let hidden = PartTarget::Overflow(vec![
        PartTarget::Chip(vec![
            GraphRef::local("main"),
            GraphRef::remote("origin/main"),
        ]),
        chip("feat"),
    ]);
    assert_eq!(
        sections(&app, K::ChipOverflow, hidden.clone(), &head),
        vec![(
            K::ChipOverflow,
            texts(&[
                meaning(K::ChipOverflow),
                "main + origin/main",
                "feat",
                title(Action::GraphCheckout),
            ])
        )]
    );
    // A chip whose ref left the commit drops its section.
    assert!(sections(&app, K::ChipLocal, chip("gone"), &head).is_empty());
    // The hidden list is the painted one: a ref gone since drops out,
    // and `[HEAD]` lists only while HEAD is detached here. With none
    // left, the section goes.
    let stale = |extra: Vec<PartTarget>| {
        let mut hidden = vec![chip("gone"), PartTarget::DetachedHead];
        hidden.extend(extra);
        PartTarget::Overflow(hidden)
    };
    assert_eq!(
        sections(&app, K::ChipOverflow, stale(vec![chip("feat")]), &head),
        vec![(
            K::ChipOverflow,
            texts(&[
                meaning(K::ChipOverflow),
                "feat",
                title(Action::GraphCheckout),
            ])
        )]
    );
    assert!(sections(&app, K::ChipOverflow, stale(Vec::new()), &head).is_empty());
    // Detached HEAD.
    app.graph.as_mut().unwrap().sync = None;
    assert_eq!(
        sections(&app, K::ChipOverflow, stale(Vec::new()), &head),
        vec![(
            K::ChipOverflow,
            texts(&[
                meaning(K::ChipOverflow),
                "HEAD",
                title(Action::GraphCheckout),
            ])
        )]
    );
    assert_eq!(
        sections(&app, K::ChipDetachedHead, PartTarget::DetachedHead, &head),
        vec![(
            K::ChipDetachedHead,
            texts(&[
                meaning(K::ChipDetachedHead),
                "detached at aaa1111",
                title(Action::Branch),
                title(Action::GraphCreateBranch),
                title(Action::ViewRemotely),
            ])
        )]
    );

    // A long message on HEAD: the expanded footer hints the lines below.
    let mut app = chip_graph_app();
    app.graph.as_mut().unwrap().commits[0].body = (0..30)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.commit_msg_expand = true;
    paint(&mut app);
    let hint = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.kind == K::GraphMoreBelow)
        .cloned()
        .unwrap_or_else(|| panic!("hint hit: {:?}", app.layout.icon_hits));
    assert_eq!(hint.target, IconTarget::GraphMoreLines(head.clone()));
    // K as the footer paints it: `v<K>` under the hint hit.
    let painted: String = (hint.x..hint.x + hint.width)
        .map(|x| app.painted_frame[(x, hint.y)].symbol().to_string())
        .collect();
    let below: usize = painted
        .strip_prefix('v')
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("footer hint {painted:?}"));
    assert!(below > 0);
    assert_eq!(app.graph_footer_lines_below(), below);
    app.pin_icon(&hint);
    assert_eq!(
        section_texts(&app),
        vec![(
            K::GraphMoreBelow,
            texts(&[
                meaning(K::GraphMoreBelow),
                &format!("{below} more lines"),
                title(Action::ToggleCommitMsgExpand),
                title(Action::ResizeCommitMsg(1)),
            ])
        )]
    );
    // Collapsing the message (M) clears the hint and its section.
    app.dispatch(Action::PopoverRun);
    assert!(!app.commit_msg_expand);
    assert!(app.popover.is_none());
    assert_eq!(app.graph_footer_lines_below(), 0);
}

/// The depth-2 commit-files footer hint hits and pins like the graph
/// footer hint: same icon, meaning, count, and actions, with the count
/// of that footer.
#[test]
fn commit_detail_footer_hint_records_a_hit_and_pins_the_more_lines_section() {
    use IconKind as K;
    let mut app = graph_app();
    app.graph.as_mut().unwrap().commits[0].body = (0..30)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.commit_msg_expand = true;
    app.open_commit_diff(
        "app".into(),
        CommitFileSource::Commit {
            commit_id: G_HEAD.into(),
        },
        vec![commit_file("M", "src/main.rs")],
        0,
        "src/main.rs".into(),
        crate::tui::diff::DiffContent::from_lines(vec!["@@ -1,1 +1,1 @@".into()]),
    );
    paint(&mut app);
    let id = app.commit_detail_msg_identity().expect("commit identity");
    let hint = app
        .layout
        .icon_hits
        .iter()
        .find(|hit| hit.kind == K::GraphMoreBelow)
        .cloned()
        .unwrap_or_else(|| panic!("hint hit: {:?}", app.layout.icon_hits));
    assert_eq!(hint.target, IconTarget::CommitDetailMoreLines(id));
    // K as the footer paints it: `v<K>` under the hint hit.
    let painted: String = (hint.x..hint.x + hint.width)
        .map(|x| app.painted_frame[(x, hint.y)].symbol().to_string())
        .collect();
    let below: usize = painted
        .strip_prefix('v')
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("footer hint {painted:?}"));
    assert!(below > 0);
    assert_eq!(app.commit_detail_footer_lines_below(), below);
    app.pin_icon(&hint);
    assert_eq!(
        section_texts(&app),
        vec![(
            K::GraphMoreBelow,
            texts(&[
                meaning(K::GraphMoreBelow),
                &format!("{below} more lines"),
                title(Action::ToggleCommitMsgExpand),
                title(Action::ResizeCommitMsg(1)),
            ])
        )]
    );
    // Collapsing the message (M) clears the hint and its section.
    app.dispatch(Action::PopoverRun);
    assert!(!app.commit_msg_expand);
    assert!(app.popover.is_none());
    assert_eq!(app.commit_detail_footer_lines_below(), 0);
}

/// Enter or a click on an action line that a diff highlight disables
/// keeps the pinned popover open and puts the gate reason on the
/// status line, like the palette. The highlight stays.
#[test]
fn a_line_the_highlight_disables_keeps_the_popover_on_enter_and_click() {
    const EXIT_HIGHLIGHT: &str = "exit highlight first (Esc)";
    let mut app = app();
    pin_sync(&mut app, "repo:app");
    app.diff_visual_anchor = Some(0);
    let fetch = command_for(&Action::Fetch).unwrap();
    assert_eq!(
        app.palette_disabled_reason(fetch).as_deref(),
        Some(EXIT_HIGHLIGHT)
    );
    let sections = app.open_popover_sections();
    let line = flat_lines(&sections)
        .iter()
        .position(|line| line.copy_text() == fetch.title)
        .expect("fetch line");
    app.popover.as_mut().expect("pinned").focus_line = line;
    assert_eq!(app.popover_run_action(), Some(Action::Fetch));
    assert_eq!(app.dispatch(Action::PopoverRun), Effect::None);
    assert!(app.popover_pinned(), "Enter keeps the popover");
    assert_eq!(app.status, EXIT_HIGHLIGHT);
    assert_eq!(app.diff_visual_anchor, Some(0), "the highlight stays");

    app.status = StatusMessage::default();
    paint(&mut app);
    let painted = app.layout.popover.clone().expect("painted");
    let (y, _) = *painted
        .lines
        .iter()
        .find(|(_, index)| *index == line)
        .expect("painted fetch line");
    app.dispatch(Action::Click {
        col: painted.inner.x + 2,
        row: y,
    });
    assert_eq!(app.dispatch(Action::Release), Effect::None);
    assert!(app.popover_pinned(), "a click keeps the popover");
    assert_eq!(app.status, EXIT_HIGHLIGHT);
}

/// The `:` palette row Icon popover (`gh`) pins the same popover.
#[test]
fn the_palette_icon_popover_row_pins_like_gh() {
    use crate::tui::action::QuickOpenEntry;
    let mut app = app();
    focus(&mut app, "repo:app");
    app.dispatch(Action::PopoverOpenFocused);
    let want = section_lines(&app);
    app.close_popover();

    let row = command_for(&Action::PopoverOpenFocused).expect("palette row");
    assert_eq!((row.title, row.keys), ("Icon popover", "gh"));
    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
    for c in "icon popover".chars() {
        app.dispatch(Action::QuickOpenChar(c));
    }
    let palette = app.command_palette().expect("palette open");
    assert_eq!(
        palette.selected().map(|row| row.title),
        Some("Icon popover")
    );
    app.dispatch(Action::QuickOpenSubmit);
    assert!(app.command_palette().is_none(), "the palette closes");
    assert!(app.popover_pinned(), "the row pins the popover");
    assert_eq!(section_lines(&app), want);
}

/// A detached checkout says `detached HEAD` once. The merge mark
/// names a local main, master, or develop when no tip ref or override
/// is known.
#[test]
fn detached_branch_says_it_once_and_the_merge_mark_finds_the_default_name() {
    use IconKind as K;
    let mut app = branch_rows_app();
    let linked = app
        .snapshot
        .repos
        .iter_mut()
        .find(|repo| repo.repo == "app/.worktrees/feat")
        .expect("linked checkout");
    linked.local_branches = vec!["develop".into(), "master".into()];
    app.rebuild_rows();
    let sections = gh_on(&mut app, "checkout:app/.worktrees/feat");
    let merge = sections
        .iter()
        .find(|(kind, _)| *kind == K::MergedIntoDefault)
        .expect("merge section");
    assert_eq!(merge.1[1], "master", "main, then master, then develop");

    app.snapshot.repos[0].branch = "HEAD".into();
    app.rebuild_rows();
    let sections = gh_on(&mut app, "checkout:app");
    let branch = sections
        .iter()
        .find(|(kind, _)| *kind == K::Branch)
        .expect("branch section");
    let detached = branch
        .1
        .iter()
        .filter(|line| line.as_str() == "detached HEAD")
        .count();
    assert_eq!(detached, 1, "{branch:?}");
}

/// Repo and checkout rows open the comment popover for their object
/// comments: open and resolved bodies in their colours.
#[test]
fn repo_and_checkout_rows_list_their_object_comments() {
    use IconKind as K;
    let mut app = branch_rows_app();
    app.comment_store = crate::tui::comments::put_comment_entry(
        &app.comment_store,
        CommentKey::Worktree {
            path: "app/.worktrees/feat".into(),
        },
        "clean up after merge",
        true,
    );
    app.comment_store = crate::tui::comments::put_comment_entry(
        &app.comment_store,
        CommentKey::Branch {
            repo: "notes".into(),
            branch: "feature".into(),
        },
        "release from here\nsecond line",
        false,
    );
    app.rebuild_rows();
    let comment = |kind: K, app: &mut AppState, id: &str| {
        gh_on(app, id)
            .into_iter()
            .find(|(icon, _)| *icon == kind)
            .unwrap_or_else(|| panic!("no {kind:?} on {id}"))
            .1
    };
    // The role a body field paints in: resolved bodies are muted.
    let body_role = |app: &AppState, label: &str| {
        app.open_popover_sections()
            .into_iter()
            .flat_map(|section| section.lines)
            .find_map(|line| match line {
                PopoverLine::Field {
                    label: found, role, ..
                } if found == label => Some(role),
                _ => None,
            })
    };
    assert_eq!(
        comment(K::CommentResolved, &mut app, "checkout:app/.worktrees/feat"),
        texts(&[
            meaning(K::CommentResolved),
            "0 open · 1 resolved",
            "clean up after merge",
            "Comment",
            "Copy comments",
        ])
    );
    assert_eq!(body_role(&app, "resolved"), Some(SegRole::Muted));
    assert_eq!(
        comment(K::Comment, &mut app, "repo:notes"),
        texts(&[
            meaning(K::Comment),
            "1 open · 0 resolved",
            "release from here",
            "Comment",
            "Copy comments",
        ])
    );
    assert_eq!(body_role(&app, "open"), Some(SegRole::File));
}

/// A commit-file folder row counts the files under it; a commit-file
/// row with a line comment on that commit opens the comment popover.
#[test]
fn commit_file_folders_count_their_files_and_rows_list_their_comments() {
    use IconKind as K;
    const SHA: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let mut app = app();
    app.comment_store = crate::tui::comments::put_comment_entry(
        &app.comment_store,
        CommentKey::CommitLine {
            repo: "app".into(),
            sha: SHA.into(),
            path: "src/main.rs".into(),
            line: 3,
            end_line: 3,
        },
        "why unwrap here?",
        false,
    );
    app.commit_tree_mode = true;
    app.open_commit_files(
        "app".into(),
        crate::tui::drill::CommitFileSource::Commit {
            commit_id: SHA.into(),
        },
        vec![
            commit_file("A", "src/lib.rs"),
            commit_file("M", "src/main.rs"),
            commit_file("M", "README.md"),
        ],
    );
    app.focus = FocusPane::Right;
    let gh_on_file_row = |app: &mut AppState, id: &str| {
        app.close_popover();
        let index = app
            .commit_file_rows()
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("no commit-file row {id}"));
        if let crate::tui::drill::DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = index;
        }
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned(), "gh pins on {id}");
        section_texts(app)
    };
    assert_eq!(
        gh_on_file_row(&mut app, "dir:src"),
        vec![(
            K::Folder,
            texts(&[
                meaning(K::Folder),
                "src",
                "2 files",
                "Stage",
                "Unstage",
                "Revert",
                "Copy entity reference",
            ])
        )]
    );
    let sections = gh_on_file_row(&mut app, "file:src/main.rs");
    assert_eq!(
        sections.iter().find(|(kind, _)| *kind == K::Comment),
        Some(&(
            K::Comment,
            texts(&[
                meaning(K::Comment),
                "1 open · 0 resolved",
                "why unwrap here?",
                "Comment",
                "Copy comments",
            ])
        )),
        "{sections:?}"
    );
}

fn commit_target(repo: &str, sha: &str) -> IconTarget {
    IconTarget::Commit {
        repo: repo.into(),
        sha: sha.into(),
    }
}

#[test]
fn commit_card_want_names_only_an_open_popover_target_missing_from_graph_and_cache() {
    let mut app = graph_app();
    assert_eq!(app.commit_card_want(), None, "no popover");
    let missing = "e".repeat(40);
    let peek = |app: &AppState, targets: Vec<(IconKind, IconTarget)>| PopoverState {
        origin: PopoverOrigin::Peek,
        targets,
        anchor: None,
        focus_line: 0,
        owner: app.popover_owner(),
    };
    // Graph commit and stash of the same checkout: no lookup.
    app.popover = Some(peek(
        &app,
        vec![
            (IconKind::CommitHash, commit_target("app", G_HEAD)),
            (IconKind::CommitDate, commit_target("app", G_STASH)),
            (IconKind::CommitAuthor, commit_target("app", "")),
        ],
    ));
    assert_eq!(app.commit_card_want(), None);
    // A graph commit of another checkout, then one off the graph.
    app.popover = Some(peek(
        &app,
        vec![
            (IconKind::CommitHash, commit_target("app", G_HEAD)),
            (IconKind::CommitHash, commit_target("lib", G_HEAD)),
            (IconKind::CommitDate, commit_target("app", &missing)),
        ],
    ));
    assert_eq!(
        app.commit_card_want(),
        Some(("lib".to_string(), G_HEAD.to_string()))
    );
    assert!(!app.apply_commit_card("lib".into(), "f".repeat(40), Err("not shown".into())));
    assert!(app.apply_commit_card("lib".into(), G_HEAD.into(), Err("boom".into())));
    assert_eq!(
        app.commit_card_want(),
        Some(("app".to_string(), missing.clone()))
    );
    assert!(app.apply_commit_card("app".into(), missing, Ok(Commit::default())));
    assert_eq!(app.commit_card_want(), None, "every shown commit has data");
    app.close_popover();
    assert_eq!(app.commit_card_want(), None);
}

#[test]
fn a_pinned_commit_card_copies_the_full_sha() {
    let mut app = graph_app();
    let hit = IconHit {
        y: 0,
        x: 0,
        width: 7,
        kind: IconKind::CommitHash,
        target: commit_target("app", G_HEAD),
    };
    app.pin_icon(&hit);
    assert!(app.popover_pinned());
    assert_eq!(app.commit_card_want(), None, "a graph commit needs no git");
    assert_eq!(app.dispatch(Action::PopoverCopyLine), copy(G_HEAD));
    app.dispatch(Action::PopoverMove(1));
    assert_eq!(app.dispatch(Action::PopoverCopyLine), copy("add graph"));
}

/// An Explorer tab's clean file: a click on the blame note's short hash
/// pins the commit card through the Explorer click path; `y` copies the
/// full id, `j` then `y` the subject, and Esc closes it. A click inside
/// a peek over the age pins the time popover. The cursor line never
/// moves.
#[test]
fn explorer_click_on_a_blame_hash_pins_its_card_and_esc_closes() {
    const SHA: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let mut app = app();
    app.open_loaded_inner_file("app", "README.md", &["one", "two", "three"]);
    assert_eq!(app.focus, FocusPane::Right, "the open focuses the file");
    app.dispatch(Action::Move(1));
    let blame_key = app.line_blame_want().expect("cursor line asks");
    app.line_blame.insert(
        blame_key,
        Some(crate::git::LineBlame {
            sha: SHA.into(),
            author: "Ada Lovelace".into(),
            author_time: super::super::unix_now() - 3 * 86_400,
            summary: "fix the parser".into(),
            orig_line: 2,
            filename: "README.md".into(),
            previous: None,
            boundary: false,
            uncommitted: false,
        }),
    );
    app.commit_cards.insert(
        "app".into(),
        SHA.into(),
        Ok(Commit {
            id: SHA.into(),
            subject: "fix the parser".into(),
            parents: vec!["c".repeat(40)],
            ..Commit::default()
        }),
    );
    let cursor = |app: &AppState| app.tabs.active_file_view().expect("file view").cursor;
    assert_eq!(cursor(&app), 1);
    paint(&mut app);
    let hit_of = |app: &AppState, kind: IconKind| {
        app.layout
            .icon_hits
            .iter()
            .find(|hit| hit.kind == kind)
            .cloned()
            .unwrap_or_else(|| panic!("no {kind:?} hit: {:?}", app.layout.icon_hits))
    };
    let hash = hit_of(&app, IconKind::CommitHash);
    assert_eq!(hash.target, commit_target("app", SHA));
    app.dispatch(Action::Click {
        col: hash.x,
        row: hash.y,
    });
    assert!(app.popover_pinned(), "the hash pins on an Explorer tab");
    let sections = app.open_popover_sections();
    assert_eq!(
        (sections.len(), sections[0].icon, &sections[0].target),
        (1, IconKind::CommitHash, &commit_target("app", SHA))
    );
    assert_eq!(app.input_mode(), InputMode::Popover);
    paint(&mut app);
    let press = |app: &mut AppState, code: KeyCode| {
        let action = event_to_action(&key(code), app.input_mode(), false, false);
        app.dispatch(action)
    };
    assert_eq!(press(&mut app, KeyCode::Char('y')), copy(SHA));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(press(&mut app, KeyCode::Char('y')), copy("fix the parser"));
    assert_eq!(cursor(&app), 1, "j moves the popover focus, not the line");
    press(&mut app, KeyCode::Esc);
    assert!(app.popover.is_none(), "Esc closes it");
    assert_eq!(cursor(&app), 1);
    assert_eq!(app.focus, FocusPane::Right);

    // A peek over the age; a click inside it pins it.
    paint(&mut app);
    let date = hit_of(&app, IconKind::CommitDate);
    peek_at(&mut app, &date, Instant::now());
    paint(&mut app);
    let rect = app.layout.popover.as_ref().expect("peek").rect;
    app.dispatch(Action::Click {
        col: rect.x + 2,
        row: rect.y + 1,
    });
    assert!(app.popover_pinned(), "a click inside the peek pins it");
    let sections = app.open_popover_sections();
    assert_eq!(
        (sections[0].icon, &sections[0].target),
        (IconKind::CommitDate, &commit_target("app", SHA))
    );
    assert_eq!(cursor(&app), 1);
}
