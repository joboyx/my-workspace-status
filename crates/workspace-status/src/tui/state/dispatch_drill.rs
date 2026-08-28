//! Enter / Esc depth-stack actions.

use super::super::action::{Action, Effect};
use super::AppState;

impl AppState {
    /// Apply Enter / Esc on the ViewStack.
    pub(crate) fn dispatch_drill(&mut self, action: Action) -> Effect {
        match action {
            Action::NavEnter => self.nav_enter(),
            Action::NavEsc => {
                if self.search_active {
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_hit = None;
                    self.status = "search cleared".into();
                    Effect::None
                } else {
                    self.nav_esc()
                }
            }
            _ => Effect::None,
        }
    }
}
