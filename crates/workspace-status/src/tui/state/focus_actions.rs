//! Tree focus actions: the `*` key, the `Focus: …` palette rows, and the
//! focus prompt.
//!
//! `*` ([`Action::FocusThisFeature`]) focuses the tree on the feature token
//! of the focused row's branch ([`feature_token`]); when the applied query is
//! already that token, it clears the focus. The palette adds `Focus: this
//! repo` ([`repo_preset`]), `Focus: type a query…` (the prompt) and `Focus:
//! clear`. The prompt previews each draft live; Enter applies it (an empty
//! draft clears), Esc restores the focus that was applied before it opened.
//! The query state and the tree rebuild live in [`super::focus_query`].

use super::super::action::{Action, Effect};
use super::super::status::StatusMessage;
use super::super::tree::{NodeKind, VisibleRow};
use super::super::tree_focus::{feature_token, repo_preset};
use super::AppState;
use crate::helpers::{is_counted_local_branch, is_default_branch, is_detached_head_branch};
use crate::snapshot::WorkspaceRepoSnapshot;

/// Reason when the focused row has no checkout (workspace and group rows).
pub(crate) const FOCUS_A_REPO_OR_CHECKOUT: &str = "focus a repo or checkout";
/// Reason when the row's checkout is on its default branch.
pub(crate) const NO_FEATURE_ON_DEFAULT_BRANCH: &str = "no feature on the default branch";
/// Reason when the row's checkout has a detached HEAD.
pub(crate) const NO_FEATURE_ON_DETACHED_HEAD: &str = "no feature on a detached HEAD";
/// Reason when the row's checkout HEAD could not be read.
pub(crate) const NO_FEATURE_ON_UNKNOWN_HEAD: &str = "no feature on an unknown HEAD";
/// Reason when a non-default branch name leaves no feature token (only a
/// prefix, such as `feature/`).
pub(crate) const NO_FEATURE_IN_BRANCH_NAME: &str = "no feature in this branch name";
/// Reason for `Focus: clear` with no focus applied.
pub(crate) const NO_FOCUS_SET: &str = "no focus set";
/// Status after Esc in the focus prompt.
pub(crate) const FOCUS_UNCHANGED: &str = "focus unchanged";

/// The open focus prompt: one line of free text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FocusPromptState {
    /// The draft query. It opens with the applied query; each edit
    /// previews it.
    pub(crate) query: String,
}

impl AppState {
    /// Run one tree focus action (`*`, a `Focus: …` palette row, or a focus
    /// prompt key). A preset that cannot run puts its reason on the status
    /// line as a warning.
    pub(super) fn dispatch_focus(&mut self, action: Action) -> Effect {
        match action {
            Action::FocusThisFeature => self.focus_this_feature(),
            Action::FocusThisRepo => match self.repo_focus_preset() {
                Ok(path) => self.set_focus_query(&path),
                Err(reason) => self.warn_focus(reason),
            },
            Action::FocusClear => {
                if self.focus_query().is_none() {
                    return self.warn_focus(NO_FOCUS_SET);
                }
                self.clear_focus_query()
            }
            Action::FocusPromptOpen => {
                self.cancel_mouse_drag();
                let query = self
                    .focus_query()
                    .map(|focus| focus.query().to_string())
                    .unwrap_or_default();
                self.focus_prompt = Some(FocusPromptState { query });
                Effect::None
            }
            Action::FocusPromptChar(c) => self.edit_focus_prompt(|query| query.push(c)),
            Action::FocusPromptBackspace => self.edit_focus_prompt(|query| {
                query.pop();
            }),
            Action::FocusPromptSubmit => match self.focus_prompt.take() {
                Some(prompt) => self.set_focus_query(&prompt.query),
                None => Effect::None,
            },
            Action::FocusPromptCancel => {
                if self.focus_prompt.take().is_none() {
                    return Effect::None;
                }
                let effect = self.cancel_focus_preview();
                self.status = StatusMessage::info(FOCUS_UNCHANGED);
                effect
            }
            _ => Effect::None,
        }
    }

    /// Why the `Focus: …` palette row of `action` cannot run, or `None`.
    ///
    /// The same check as the key: [`Self::dispatch_focus`] warns with the
    /// same copy.
    pub(crate) fn focus_palette_reason(&self, action: &Action) -> Option<String> {
        let reason = match action {
            Action::FocusThisFeature => self.feature_focus_preset().err(),
            Action::FocusThisRepo => self.repo_focus_preset().err(),
            Action::FocusClear => self.focus_query().is_none().then_some(NO_FOCUS_SET),
            _ => None,
        };
        reason.map(str::to_string)
    }

    /// `*`: focus on the feature of the focused row, or clear the focus
    /// when the applied query already is that feature (case-insensitive,
    /// trimmed).
    fn focus_this_feature(&mut self) -> Effect {
        let token = match self.feature_focus_preset() {
            Ok(token) => token,
            Err(reason) => return self.warn_focus(reason),
        };
        let applied = self
            .focus_query()
            .is_some_and(|focus| same_query(focus.query(), &token));
        if applied {
            self.clear_focus_query()
        } else {
            self.set_focus_query(&token)
        }
    }

    /// The "this feature" token of the focused row's checkout: on a file,
    /// dir, or section row that row's checkout, on a family container row
    /// the primary checkout. `Err` names why there is none.
    fn feature_focus_preset(&self) -> Result<String, &'static str> {
        let checkout = self
            .focused_row()
            .and_then(|row| self.row_checkout(row))
            .ok_or(FOCUS_A_REPO_OR_CHECKOUT)?;
        let branch = checkout.branch.as_str();
        let override_name = checkout.default_branch_override.as_deref();
        if is_detached_head_branch(branch) {
            return Err(NO_FEATURE_ON_DETACHED_HEAD);
        }
        if !is_counted_local_branch(branch) {
            return Err(NO_FEATURE_ON_UNKNOWN_HEAD);
        }
        if is_default_branch(branch, override_name) {
            return Err(NO_FEATURE_ON_DEFAULT_BRANCH);
        }
        feature_token(branch, override_name).ok_or(NO_FEATURE_IN_BRANCH_NAME)
    }

    /// The "this repo" query of the focused row: its family primary repo
    /// path. `Err` on workspace and group rows.
    fn repo_focus_preset(&self) -> Result<String, &'static str> {
        self.focused_row()
            .filter(|row| has_checkout(row))
            .and_then(|row| repo_preset(row.repo.as_deref(), row.primary_repo.as_deref()))
            .ok_or(FOCUS_A_REPO_OR_CHECKOUT)
    }

    /// The snapshot checkout of `row`: the checkout of a checkout, file,
    /// dir, or section row, and the primary checkout of a family container
    /// row (its `repo` is the primary path). `None` on workspace and group
    /// rows.
    fn row_checkout(&self, row: &VisibleRow) -> Option<&WorkspaceRepoSnapshot> {
        if !has_checkout(row) {
            return None;
        }
        let repo = row.repo.as_deref()?;
        self.snapshot.repos.iter().find(|snap| snap.repo == repo)
    }

    /// Apply `edit` to the open prompt's draft and preview the result.
    fn edit_focus_prompt(&mut self, edit: impl FnOnce(&mut String)) -> Effect {
        let Some(prompt) = self.focus_prompt.as_mut() else {
            return Effect::None;
        };
        edit(&mut prompt.query);
        let draft = prompt.query.clone();
        self.preview_focus_query(&draft)
    }

    /// Put `reason` on the status line as a warning.
    fn warn_focus(&mut self, reason: &str) -> Effect {
        self.status = StatusMessage::warn(reason.to_string());
        Effect::None
    }
}

/// True for a row that belongs to one checkout (not the workspace root or
/// a group).
fn has_checkout(row: &VisibleRow) -> bool {
    !matches!(row.kind, NodeKind::Workspace | NodeKind::Group)
}

/// True when two focus queries are the same text, ignoring case and outer
/// whitespace.
fn same_query(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

#[cfg(test)]
mod tests;
