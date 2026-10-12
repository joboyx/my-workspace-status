use std::fs;

use crate::harness::{user_config_file, PtySession, BASELINE_USER_CONFIG};
use crate::seed::daily_workspace;
use crate::support::{SETTLE_MS, WAIT};

/// Tokyo Night (the launch theme) surface, mode pill, and heading.
const TOKYO_NIGHT: [(u8, u8, u8); 3] = [(0x1a, 0x1b, 0x26), (0x3d, 0x59, 0xa1), (0x7d, 0xcf, 0xff)];

/// Monokai, the theme after Tokyo Night in the old `T` cycle: surface,
/// mode pill, and heading.
const MONOKAI: [(u8, u8, u8); 3] = [(0x27, 0x28, 0x22), (0x49, 0x48, 0x3e), (0x66, 0xd9, 0xef)];

/// Tree pill, no `Flat paths`, no `theme:` toast, Tokyo Night painted, and
/// no Monokai colour.
fn still_tokyo_tree(tui: &PtySession, screen: &str) -> bool {
    screen.contains(" tree")
        && !screen.contains("Flat paths")
        && !screen.contains("theme: ")
        && TOKYO_NIGHT.iter().all(|&(r, g, b)| tui.has_rgb(r, g, b))
        && MONOKAI.iter().all(|&(r, g, b)| !tui.has_rgb(r, g, b))
}

/// `T` is not a theme key: CSI-u Shift+T and a raw `T` byte set no theme.
///
/// Docs: themes are palette `Theme: …` rows; bare `T` does nothing and
/// only `gT` uses `T`. Help has no `cycle theme` row. Launch seed
/// `WS_STATUS_THEME=tokyo-night` with an empty theme store. Shift+T and
/// raw `T` must keep Tokyo Night painted, paint no Monokai (the old next
/// theme), show no `theme:` toast, write no theme store, and leave the
/// user config file byte-identical. Lowercase `t` after them still
/// flattens the tree, so the keys were read and the TUI is live.
#[test]
fn pty_shift_t_does_not_cycle_theme() {
    let (_root, workspace) = daily_workspace();
    let config_home = workspace.join(".e2e-config");
    fs::create_dir_all(&config_home).unwrap();
    let theme_store = workspace.join(".e2e-state").join("theme.json");
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_THEME", "tokyo-night"),
            ("XDG_CONFIG_HOME", config_home.to_str().unwrap()),
            ("WS_STATUS_THEME_STORE", theme_store.to_str().unwrap()),
        ],
    );
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| still_tokyo_tree(&tui, screen),
        "launch: Tokyo Night tree",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("VIEW")
                && screen.contains("flat / tree")
                && !screen.contains("cycle theme")
        },
        "help VIEW lists t flat/tree and no T cycle theme row",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("flat / tree") && still_tokyo_tree(&tui, screen),
        "Esc closes help so Shift+T reaches normal mode",
        WAIT,
    );

    tui.shift_letter('T');
    tui.key('T');
    tui.wait_ms(SETTLE_MS);
    let screen = tui.screen();
    assert!(
        still_tokyo_tree(&tui, &screen),
        "Shift+T and raw T must keep Tokyo Night and show no theme toast:\n{screen}"
    );

    tui.key('t');
    tui.wait_pred(
        |screen| {
            screen.contains("Flat paths")
                && !screen.contains("theme: ")
                && TOKYO_NIGHT.iter().all(|&(r, g, b)| tui.has_rgb(r, g, b))
                && MONOKAI.iter().all(|&(r, g, b)| !tui.has_rgb(r, g, b))
        },
        "lowercase t after T still flattens; Tokyo Night stays painted",
        WAIT,
    );
    assert!(
        !theme_store.exists(),
        "`T` must not write the theme store {}",
        theme_store.display()
    );
    let config = user_config_file(&config_home);
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        BASELINE_USER_CONFIG,
        "`T` must not rewrite the user config file {}",
        config.display()
    );
}
