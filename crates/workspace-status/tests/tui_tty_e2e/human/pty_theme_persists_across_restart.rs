use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// Painted chrome for the themes these tests launch into. Values match
/// `pty_shift_t_csi_u_cycles_theme`; each is unique across the built-ins.
struct ThemeChrome {
    id: &'static str,
    surface: (u8, u8, u8),
    pill: (u8, u8, u8),
    heading: (u8, u8, u8),
}

const TOKYO_NIGHT: ThemeChrome = ThemeChrome {
    id: "tokyo-night",
    surface: (0x1a, 0x1b, 0x26),
    pill: (0x3d, 0x59, 0xa1),
    heading: (0x7d, 0xcf, 0xff),
};

const MONOKAI: ThemeChrome = ThemeChrome {
    id: "monokai",
    surface: (0x27, 0x28, 0x22),
    pill: (0x49, 0x48, 0x3e),
    heading: (0x66, 0xd9, 0xef),
};

const DRACULA: ThemeChrome = ThemeChrome {
    id: "dracula",
    surface: (0x28, 0x2a, 0x36),
    pill: (0x44, 0x47, 0x5a),
    heading: (0x8b, 0xe9, 0xfd),
};

const SLATE: ThemeChrome = ThemeChrome {
    id: "slate",
    surface: (0x11, 0x15, 0x1b),
    pill: (0x2d, 0x45, 0x66),
    heading: (0x7f, 0xc4, 0xd6),
};

const THEMES: [&ThemeChrome; 4] = [&TOKYO_NIGHT, &MONOKAI, &DRACULA, &SLATE];

/// Wait until `theme` paints its surface, mode pill, and heading, then fail
/// if any other theme in [`THEMES`] still paints one of its colours.
fn wait_active_theme(tui: &PtySession, theme: &ThemeChrome) {
    tui.wait_contains(" tree", WAIT);
    for (r, g, b) in [theme.surface, theme.pill, theme.heading] {
        tui.wait_has_rgb(r, g, b, WAIT);
    }
    for other in THEMES.iter().filter(|other| other.id != theme.id) {
        for (r, g, b) in [other.surface, other.pill, other.heading] {
            assert!(
                !tui.has_rgb(r, g, b),
                "{} colour rgb({r},{g},{b}) must not paint when {} is active:\n{}",
                other.id,
                theme.id,
                tui.screen()
            );
        }
    }
}

/// Write a last-used theme store at `path` the way the binary saves it.
fn seed_theme_store(path: &Path, theme: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("{{\n  \"version\": 1,\n  \"theme\": \"{theme}\"\n}}\n"),
    )
    .unwrap();
}

/// Operator repro: `T` picks a theme, quit, relaunch, the theme is kept.
///
/// The baseline user config theme is Tokyo Night and no `WS_STATUS_THEME`
/// is set. One CSI-u Shift+T cycles to Monokai, which the store at the
/// shared `WS_STATUS_THEME_STORE` path must hold. `q` quits. A second
/// launch with the same store must paint Monokai, not the config Tokyo
/// Night and not the Slate default. A session-only `T` cannot pass.
#[test]
fn pty_theme_cycle_survives_restart() {
    let (_root, workspace) = daily_workspace();
    let theme_store = workspace.join(".e2e-state").join("theme-restart.json");
    let store = theme_store.to_str().expect("utf-8 store path");
    let env = [("WS_STATUS_THEME_STORE", store)];

    let mut first = PtySession::open_with_env(&workspace, &env);
    wait_active_theme(&first, &TOKYO_NIGHT);
    first.shift_letter('T');
    first.wait_contains("theme: Monokai", WAIT);
    wait_active_theme(&first, &MONOKAI);
    first.key('q');
    first.wait_exit(WAIT);

    let stored = fs::read_to_string(&theme_store).unwrap_or_else(|err| {
        panic!(
            "`T` must save the theme store {}: {err}",
            theme_store.display()
        )
    });
    assert!(
        stored.contains("\"monokai\""),
        "store must hold the cycled theme monokai:\n{stored}"
    );

    // `first` stays bound: its Drop removes the fixture root.
    let restarted = PtySession::open_with_env(&workspace, &env);
    wait_active_theme(&restarted, &MONOKAI);
}

/// A known `WS_STATUS_THEME` beats the last-used theme store.
///
/// The store holds Monokai. Launch with `WS_STATUS_THEME=dracula` must
/// paint Dracula, and the launch itself must not rewrite the store.
#[test]
fn pty_theme_env_beats_stored_theme() {
    let (_root, workspace) = daily_workspace();
    let theme_store = workspace.join(".e2e-state").join("theme-seeded.json");
    seed_theme_store(&theme_store, "monokai");
    let seeded = fs::read(&theme_store).unwrap();
    let store = theme_store.to_str().expect("utf-8 store path");

    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_THEME_STORE", store),
            ("WS_STATUS_THEME", "dracula"),
        ],
    );
    wait_active_theme(&tui, &DRACULA);
    tui.key('q');
    tui.wait_exit(WAIT);
    assert_eq!(
        fs::read(&theme_store).unwrap(),
        seeded,
        "a launch without `T` must leave the theme store unchanged"
    );
}
