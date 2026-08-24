//! Double Ctrl-C quit chord.
//!
//! First press arms a short window and prompts. Second press within the window
//! quits. An expired arm is treated as a fresh first press.

use std::time::{Duration, Instant};

/// Window to confirm exit after the first Ctrl-C (ms).
pub const CTRL_C_EXIT_MS: u64 = 2000;

/// Status / overlay copy shown after the first Ctrl-C.
pub const CTRL_C_EXIT_PROMPT: &str = "Press Ctrl+C again to exit";

/// Result of one Ctrl-C press against the armed window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CtrlCExitResult {
    /// Instant until which a second Ctrl-C quits. `None` when not armed.
    pub armed_until: Option<Instant>,
    /// True when this press should quit the TUI.
    pub quit: bool,
    /// True when chrome should show [`CTRL_C_EXIT_PROMPT`].
    pub prompt: bool,
}

/// Pure double-Ctrl-C state machine. Pass explicit `now` in tests.
pub fn handle_ctrl_c(armed_until: Option<Instant>, now: Instant) -> CtrlCExitResult {
    if armed_until.is_some_and(|until| now < until) {
        return CtrlCExitResult {
            armed_until: None,
            quit: true,
            prompt: false,
        };
    }
    CtrlCExitResult {
        armed_until: Some(now + Duration::from_millis(CTRL_C_EXIT_MS)),
        quit: false,
        prompt: true,
    }
}

/// True when `status` is the ephemeral quit prompt (never a breadcrumb toast).
pub fn is_ctrl_c_exit_prompt(status: &str) -> bool {
    status == CTRL_C_EXIT_PROMPT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_press_arms_and_second_within_window_quits() {
        let t0 = Instant::now();
        let first = handle_ctrl_c(None, t0);
        assert!(!first.quit);
        assert!(first.prompt);
        assert_eq!(
            first.armed_until,
            Some(t0 + Duration::from_millis(CTRL_C_EXIT_MS))
        );

        let second = handle_ctrl_c(first.armed_until, t0 + Duration::from_millis(50));
        assert!(second.quit);
        assert!(!second.prompt);
        assert_eq!(second.armed_until, None);
    }

    #[test]
    fn expired_arm_is_a_fresh_first_press() {
        let t0 = Instant::now();
        let armed = handle_ctrl_c(None, t0);
        let late = handle_ctrl_c(
            armed.armed_until,
            t0 + Duration::from_millis(CTRL_C_EXIT_MS),
        );
        assert!(!late.quit);
        assert!(late.prompt);
        assert_eq!(
            late.armed_until,
            Some(t0 + Duration::from_millis(CTRL_C_EXIT_MS * 2))
        );
    }

    #[test]
    fn prompt_string_matches_harness_copy() {
        assert!(
            CTRL_C_EXIT_PROMPT
                .to_ascii_lowercase()
                .contains("ctrl+c again"),
            "{CTRL_C_EXIT_PROMPT}"
        );
        assert!(is_ctrl_c_exit_prompt(CTRL_C_EXIT_PROMPT));
        assert!(!is_ctrl_c_exit_prompt("refreshed workspace"));
    }
}
