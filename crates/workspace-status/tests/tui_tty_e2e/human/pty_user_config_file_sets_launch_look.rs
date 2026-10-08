use std::fs;

use crate::harness::{user_config_file, PtySession, UserConfig, BASELINE_USER_CONFIG, COLS, ROWS};
use crate::seed::daily_workspace;
use crate::support::WAIT;

const TOKYO_NIGHT_SURFACE: (u8, u8, u8) = (0x1a, 0x1b, 0x26);
const DRACULA_SURFACE: (u8, u8, u8) = (0x28, 0x2a, 0x36);
const DRACULA_CONFIG: &str = "{\"theme\":\"dracula\"}\n";

/// The user config file sets the launch look, and the harness pin is
/// explicit.
///
/// A plain spawn writes the baseline file (Tokyo Night, terminal
/// background). The binary accepts it and paints Tokyo Night. A file the
/// test wrote first is kept, so its Dracula theme paints. The
/// `UserConfig::Own` opt-out writes no file.
#[test]
fn pty_user_config_file_sets_launch_look() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    let (r, g, b) = TOKYO_NIGHT_SURFACE;
    tui.wait_has_rgb(r, g, b, WAIT);
    let file = user_config_file(&workspace.join(".e2e-config"));
    assert_eq!(fs::read_to_string(&file).unwrap(), BASELINE_USER_CONFIG);
    drop(tui);

    let (_root, owned) = daily_workspace();
    let file = user_config_file(&owned.join(".e2e-config"));
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, DRACULA_CONFIG).unwrap();
    let tui = PtySession::open(&owned);
    tui.wait_contains("README.md", WAIT);
    let (r, g, b) = DRACULA_SURFACE;
    tui.wait_has_rgb(r, g, b, WAIT);
    let (r, g, b) = TOKYO_NIGHT_SURFACE;
    assert!(
        !tui.has_rgb(r, g, b),
        "test-owned config picks Dracula, not the baseline theme:\n{}",
        tui.screen()
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        DRACULA_CONFIG,
        "the harness must not clobber a config file the test wrote"
    );
    drop(tui);

    let (_root, bare) = daily_workspace();
    let tui = PtySession::open_size_with_config(&bare, COLS, ROWS, &[], UserConfig::Own);
    tui.wait_contains("README.md", WAIT);
    assert!(
        !user_config_file(&bare.join(".e2e-config")).exists(),
        "UserConfig::Own writes no user config file"
    );
}
