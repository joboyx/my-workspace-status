use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{GIT_WAIT, WAIT};

/// Painted chrome for one built-in theme. Surfaces / pills / headings are
/// unique across the cycle so a no-op or a skipped id cannot pass.
struct ThemeChrome {
    id: &'static str,
    toast: &'static str,
    surface: (u8, u8, u8),
    pill: (u8, u8, u8),
    heading: (u8, u8, u8),
}

/// Docs + help `T` cycle. Wraps after Catppuccin Mocha.
const THEME_CYCLE: [ThemeChrome; 5] = [
    ThemeChrome {
        id: "tokyo-night",
        toast: "theme: Tokyo Night",
        surface: (0x1a, 0x1b, 0x26),
        pill: (0x3d, 0x59, 0xa1),
        heading: (0x7d, 0xcf, 0xff),
    },
    ThemeChrome {
        id: "monokai",
        toast: "theme: Monokai",
        surface: (0x27, 0x28, 0x22),
        pill: (0x49, 0x48, 0x3e),
        heading: (0x66, 0xd9, 0xef),
    },
    ThemeChrome {
        id: "dracula",
        toast: "theme: Dracula",
        surface: (0x28, 0x2a, 0x36),
        pill: (0x44, 0x47, 0x5a),
        heading: (0x8b, 0xe9, 0xfd),
    },
    ThemeChrome {
        id: "gruvbox-dark",
        toast: "theme: Gruvbox Dark",
        surface: (0x28, 0x28, 0x28),
        pill: (0x50, 0x49, 0x45),
        heading: (0x83, 0xa5, 0x98),
    },
    ThemeChrome {
        id: "catppuccin-mocha",
        toast: "theme: Catppuccin Mocha",
        surface: (0x1e, 0x1e, 0x2e),
        pill: (0x45, 0x47, 0x5a),
        heading: (0x89, 0xdc, 0xeb),
    },
];

/// Tokyo Night graph lane 0 (`DEFAULT_LANE_COLORS[0]` / dir). Absent from
/// every other built-in palette, so a stuck Tokyo gutter after `T` fails.
const TOKYO_GRAPH_LANE0: (u8, u8, u8) = (0x7a, 0xa2, 0xf7);

fn theme_is_tree_not_flat(screen: &str) -> bool {
    screen.contains(" tree") && !screen.contains("Flat paths")
}

fn wait_theme_chrome(tui: &PtySession, theme: &ThemeChrome, expect_toast: bool) {
    tui.wait_pred(
        |screen| {
            theme_is_tree_not_flat(screen)
                && (!expect_toast || screen.contains(theme.toast))
                && THEME_CYCLE
                    .iter()
                    .filter(|other| other.toast != theme.toast)
                    .all(|other| !screen.contains(other.toast))
        },
        &format!(
            "{} chrome: tree pill, not Flat paths{}",
            theme.id,
            if expect_toast {
                format!(", toast `{}`", theme.toast)
            } else {
                String::new()
            }
        ),
        WAIT,
    );
    tui.wait_has_rgb(theme.surface.0, theme.surface.1, theme.surface.2, WAIT);
    tui.wait_has_rgb(theme.pill.0, theme.pill.1, theme.pill.2, WAIT);
    tui.wait_has_rgb(theme.heading.0, theme.heading.1, theme.heading.2, WAIT);
    for other in THEME_CYCLE.iter().filter(|other| other.id != theme.id) {
        assert!(
            !tui.has_rgb(other.surface.0, other.surface.1, other.surface.2),
            "{} surface must not remain after {}:\n{}",
            other.id,
            theme.id,
            tui.screen()
        );
        assert!(
            !tui.has_rgb(other.pill.0, other.pill.1, other.pill.2),
            "{} mode pill must not remain after {}:\n{}",
            other.id,
            theme.id,
            tui.screen()
        );
        assert!(
            !tui.has_rgb(other.heading.0, other.heading.1, other.heading.2),
            "{} heading must not remain after {}:\n{}",
            other.id,
            theme.id,
            tui.screen()
        );
    }
}

fn wait_graph_lanes_match_theme(tui: &PtySession, theme: &ThemeChrome) {
    if theme.id == "tokyo-night" {
        tui.wait_has_rgb(
            TOKYO_GRAPH_LANE0.0,
            TOKYO_GRAPH_LANE0.1,
            TOKYO_GRAPH_LANE0.2,
            WAIT,
        );
        return;
    }
    tui.wait_pred(
        |_| {
            !tui.has_rgb(
                TOKYO_GRAPH_LANE0.0,
                TOKYO_GRAPH_LANE0.1,
                TOKYO_GRAPH_LANE0.2,
            )
        },
        &format!(
            "graph lanes follow {} (Tokyo lane 0 {} gone)",
            theme.id, "#7aa2f7"
        ),
        WAIT,
    );
}

fn assert_no_theme_store(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let name = entry.unwrap().file_name();
        let s = name.to_string_lossy();
        assert!(
            !s.to_ascii_lowercase().contains("theme"),
            "`T` is session-only; must not write {}:\n{}",
            dir.join(&*s).display(),
            s
        );
    }
}

/// CSI-u Shift+T cycles the documented colour theme. Raw `'T'` / `'t'` are
/// different paths (`t` is tree/flat).
///
/// Help lists `T` cycle theme next to `t` flat/tree. Launch seed
/// `WS_STATUS_THEME` paints that id. Each Shift+T advances
/// Tokyo Night → Monokai → Dracula → Gruvbox Dark → Catppuccin Mocha →
/// Tokyo Night. Toast, surface, mode pill, heading, and graph lane 0 must
/// all match that id. A no-op, a skipped id, or lowercase `t` cannot pass.
/// There is no theme file.
#[test]
fn pty_shift_t_csi_u_cycles_theme() {
    let (_root, workspace) = daily_workspace();
    let config_home = workspace.join(".e2e-config");
    fs::create_dir_all(&config_home).unwrap();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_THEME", "tokyo-night"),
            ("XDG_CONFIG_HOME", config_home.to_str().unwrap()),
        ],
    );
    tui.wait_contains(" tree", WAIT);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("cycle theme")
                && screen.contains("flat / tree")
                && screen.contains("VIEW")
        },
        "help VIEW lists T cycle theme and t flat/tree as distinct rows",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("cycle theme") && theme_is_tree_not_flat(screen),
        "Esc closes help so Shift+T is not swallowed",
        WAIT,
    );

    tui.search("merger");
    tui.wait_contains("/merger", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("workspace › merger")
                && (screen.contains("Working tree") || screen.contains("Uncommitted"))
        },
        "merger graph is focused so lane colours are on screen",
        GIT_WAIT,
    );

    wait_theme_chrome(&tui, &THEME_CYCLE[0], false);
    wait_graph_lanes_match_theme(&tui, &THEME_CYCLE[0]);

    for step in 1..=THEME_CYCLE.len() {
        let theme = &THEME_CYCLE[step % THEME_CYCLE.len()];
        let before = tui.color_fingerprint();
        tui.shift_letter('T');
        wait_theme_chrome(&tui, theme, true);
        wait_graph_lanes_match_theme(&tui, theme);
        assert_ne!(
            tui.color_fingerprint(),
            before,
            "Shift+T step {step} ({}) must repaint cells; a toast-only no-op fails:\n{}",
            theme.id,
            tui.screen()
        );
    }

    assert_no_theme_store(&workspace.join(".e2e-state"));
    assert_no_theme_store(&config_home);

    let (_root2, seeded) = daily_workspace();
    let mut seeded_tui = PtySession::open_with_env(&seeded, &[("WS_STATUS_THEME", "dracula")]);
    seeded_tui.wait_contains(" tree", WAIT);
    wait_theme_chrome(&seeded_tui, &THEME_CYCLE[2], false);
    seeded_tui.shift_letter('T');
    wait_theme_chrome(&seeded_tui, &THEME_CYCLE[3], true);
}
