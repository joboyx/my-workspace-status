//! Elm-style Action / Effect for graph view state.
//!
//! Dispatch is pure. This crate does not run an event loop. A later TUI
//! can map keys to [`Action`] and apply [`Effect`].

/// User or system input that changes graph view state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Flip [`crate::GraphModel::show_ignored`].
    ToggleShowIgnored,
    /// Set [`crate::GraphModel::show_ignored`] to a fixed value.
    SetShowIgnored(bool),
}

/// Side effect requested after dispatch.
///
/// Graph view updates are in-memory. [`Effect::None`] is the only variant
/// today. Keep the type so a later TUI can add I/O without a new pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// No side effect.
    None,
}
