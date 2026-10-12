//! Go to line: the `:<n>` row of Quick Open commands mode.
//!
//! [`super::super::action::Action::GoToLine`] moves the cursor of the
//! focused line view with the same cursor and scroll paths `gg` / `G` use:
//! a file body (file tab, or a clean file in the Explorer preview) moves
//! its line cursor; a file diff
//! (Workspace diff pane, compare tab, commit or stash diff, or a changed
//! file in the Explorer preview) moves its diff cursor to the row of that
//! line ([`row_for_line`]). A `V` highlight keeps its anchor, so the jump
//! moves the range end like `G` does. [`AppState::go_to_line_refusal`] is
//! the one gate: the palette row and the dispatch both read it.

use super::super::action::Effect;
use super::super::diff::row_for_line;
use super::super::gates::{ListFocusTarget, FOCUS_A_FILE_DIFF};
use super::super::status::StatusMessage;
use super::super::tabs::ExplorerPreview;
use super::{AppState, FocusPane};

/// Go-to-line refusal on a view that is a line view but shows no line yet
/// (loading, empty, binary, or failed body or diff; an Explorer folder row).
pub(crate) const NO_LINES_TO_GO_TO: &str = "no lines to go to";

/// Which cursor a go to line moves.
enum LineView {
    /// The line cursor of the active file body.
    FileBody,
    /// The diff cursor of the displayed file diff.
    Diff,
}

impl AppState {
    /// Why the go-to-line row cannot run here, or `None` when it can.
    ///
    /// A file tab or an Explorer tab needs a body or diff with lines (the
    /// Explorer tree jumps in its preview). Elsewhere the file diff must
    /// have focus: the tree, graph, commit-file, and compare file lists,
    /// and a folder summary refuse with [`FOCUS_A_FILE_DIFF`]; a diff with
    /// no numbered line refuses with [`NO_LINES_TO_GO_TO`].
    pub(crate) fn go_to_line_refusal(&self) -> Option<String> {
        self.line_view().err().map(str::to_string)
    }

    /// Run [`super::super::action::Action::GoToLine`]: put the focused line
    /// view's cursor on 1-based `line`, clamped to the view (`0` is line
    /// 1). A refusal goes on the status line and moves nothing.
    pub(super) fn go_to_line(&mut self, line: usize) -> Effect {
        let view = match self.line_view() {
            Ok(view) => view,
            Err(reason) => {
                self.status = StatusMessage::warn(reason);
                return Effect::None;
            }
        };
        if self.is_explorer_tab() {
            // From the Explorer tree the jump lands in the preview.
            self.focus = FocusPane::Right;
        }
        let line = line.max(1);
        match view {
            LineView::FileBody => self.set_file_cursor(line - 1),
            LineView::Diff => {
                let line = u32::try_from(line).unwrap_or(u32::MAX);
                if let Some(row) =
                    row_for_line(self.current_diff_content(), self.diff_layout(), line)
                {
                    self.diff_cursor = row;
                    self.sync_diff_scroll();
                }
            }
        }
        Effect::None
    }

    /// The line view a go to line moves, or why there is none.
    fn line_view(&self) -> Result<LineView, &'static str> {
        if self.is_file_tab() {
            return self.file_body_has_lines().ok_or(NO_LINES_TO_GO_TO);
        }
        if let Some(tab) = self.tabs.active_explorer() {
            return match &tab.preview {
                ExplorerPreview::File(_) => self.file_body_has_lines(),
                ExplorerPreview::Diff { .. } => self.diff_has_lines(),
                ExplorerPreview::None => None,
            }
            .ok_or(NO_LINES_TO_GO_TO);
        }
        if self.list_focus_target() != ListFocusTarget::None || self.folder_summary().is_some() {
            return Err(FOCUS_A_FILE_DIFF);
        }
        self.diff_has_lines().ok_or(NO_LINES_TO_GO_TO)
    }

    /// [`LineView::FileBody`] when the active file body has a line.
    fn file_body_has_lines(&self) -> Option<LineView> {
        self.tabs
            .active_file_view()
            .is_some_and(|tab| !tab.lines().is_empty())
            .then_some(LineView::FileBody)
    }

    /// [`LineView::Diff`] when the displayed diff has a numbered line.
    fn diff_has_lines(&self) -> Option<LineView> {
        row_for_line(self.current_diff_content(), self.diff_layout(), 1).map(|_| LineView::Diff)
    }
}
