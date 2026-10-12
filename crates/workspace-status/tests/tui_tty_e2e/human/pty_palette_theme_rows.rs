use std::fs;
use std::path::Path;

use crate::harness::{user_config_file, PtySession, BASELINE_USER_CONFIG};
use crate::seed::daily_workspace;
use crate::support::{
    apply_palette_theme, open_command_palette, type_palette_filter, GIT_WAIT, WAIT,
};

/// Painted chrome for one built-in theme. Surfaces / pills / headings are
/// unique across the built-ins so a no-op or a wrong row cannot pass.
struct ThemeChrome {
    id: &'static str,
    label: &'static str,
    surface: (u8, u8, u8),
    pill: (u8, u8, u8),
    heading: (u8, u8, u8),
    muted: (u8, u8, u8),
}

/// Every built-in theme in `THEME_IDS` order (the palette `Theme: …` row
/// order). The apply test walks it from Monokai and wraps to Tokyo Night.
const THEMES: [ThemeChrome; 13] = [
    ThemeChrome {
        id: "tokyo-night",
        label: "Tokyo Night",
        surface: (0x1a, 0x1b, 0x26),
        pill: (0x3d, 0x59, 0xa1),
        heading: (0x7d, 0xcf, 0xff),
        muted: (0x9a, 0xa5, 0xce),
    },
    ThemeChrome {
        id: "monokai",
        label: "Monokai",
        surface: (0x27, 0x28, 0x22),
        pill: (0x49, 0x48, 0x3e),
        heading: (0x66, 0xd9, 0xef),
        muted: (0xb8, 0xb3, 0x9c),
    },
    ThemeChrome {
        id: "dracula",
        label: "Dracula",
        surface: (0x28, 0x2a, 0x36),
        pill: (0x44, 0x47, 0x5a),
        heading: (0x8b, 0xe9, 0xfd),
        muted: (0xb4, 0xbc, 0xe4),
    },
    ThemeChrome {
        id: "gruvbox-dark",
        label: "Gruvbox Dark",
        surface: (0x28, 0x28, 0x28),
        pill: (0x50, 0x49, 0x45),
        heading: (0x83, 0xa5, 0x98),
        muted: (0xbd, 0xae, 0x93),
    },
    ThemeChrome {
        id: "catppuccin-mocha",
        label: "Catppuccin Mocha",
        surface: (0x1e, 0x1e, 0x2e),
        pill: (0x45, 0x47, 0x5a),
        heading: (0x89, 0xdc, 0xeb),
        muted: (0xa6, 0xad, 0xc8),
    },
    ThemeChrome {
        id: "slate",
        label: "Slate",
        surface: (0x11, 0x15, 0x1b),
        pill: (0x2d, 0x45, 0x66),
        heading: (0x7f, 0xc4, 0xd6),
        muted: (0x8a, 0x94, 0xa3),
    },
    ThemeChrome {
        id: "solarized-dark",
        label: "Solarized Dark",
        surface: (0x00, 0x2b, 0x36),
        pill: (0x0e, 0x5a, 0x74),
        heading: (0x39, 0xa8, 0x9f),
        muted: (0x92, 0xa1, 0xa3),
    },
    ThemeChrome {
        id: "nord",
        label: "Nord",
        surface: (0x2e, 0x34, 0x40),
        pill: (0x5e, 0x81, 0xac),
        heading: (0x88, 0xc0, 0xd0),
        muted: (0xaf, 0xb8, 0xc7),
    },
    ThemeChrome {
        id: "rose-pine",
        label: "Rosé Pine",
        surface: (0x19, 0x17, 0x24),
        pill: (0x31, 0x74, 0x8f),
        heading: (0xeb, 0xbc, 0xba),
        muted: (0x9c, 0x99, 0xb3),
    },
    ThemeChrome {
        id: "kanagawa",
        label: "Kanagawa",
        surface: (0x1f, 0x1f, 0x28),
        pill: (0x2d, 0x4f, 0x67),
        heading: (0x7f, 0xb4, 0xca),
        muted: (0xa6, 0xa0, 0x8a),
    },
    ThemeChrome {
        id: "everforest",
        label: "Everforest",
        surface: (0x2d, 0x35, 0x3b),
        pill: (0x3a, 0x51, 0x5d),
        heading: (0x83, 0xc0, 0x92),
        muted: (0xae, 0xb8, 0xb0),
    },
    ThemeChrome {
        id: "one-dark",
        label: "One Dark",
        surface: (0x28, 0x2c, 0x34),
        pill: (0x3b, 0x50, 0x70),
        heading: (0x56, 0xb6, 0xc2),
        muted: (0xa6, 0xab, 0xb4),
    },
    ThemeChrome {
        id: "github-dark-dimmed",
        label: "GitHub Dark Dimmed",
        surface: (0x22, 0x27, 0x2e),
        pill: (0x25, 0x5a, 0xb2),
        heading: (0x96, 0xd0, 0xff),
        muted: (0x9e, 0xa8, 0xb3),
    },
];

impl ThemeChrome {
    /// Status text that Enter on this theme's palette row shows.
    fn toast(&self) -> String {
        format!("theme: {}", self.label)
    }
}

/// Tokyo Night graph lane 0 (`DEFAULT_LANE_COLORS[0]` / dir). Absent from
/// every other built-in palette, so a stuck Tokyo gutter fails.
const TOKYO_GRAPH_LANE0: (u8, u8, u8) = (0x7a, 0xa2, 0xf7);

fn theme_by_id(id: &str) -> &'static ThemeChrome {
    THEMES
        .iter()
        .find(|theme| theme.id == id)
        .unwrap_or_else(|| panic!("no built-in theme {id}"))
}

/// Search `merger` so its graph is focused and lane colours are on screen.
/// The launch README diff view under Tokyo Night paints Nord's heading
/// colour, so the one-theme-only colour checks run on the graph instead.
fn focus_merger_graph(tui: &mut PtySession) {
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
}

fn theme_is_tree_not_flat(screen: &str) -> bool {
    screen.contains(" tree") && !screen.contains("Flat paths")
}

/// With the palette open, wait until `shown` paints its heading and muted
/// colours and `gone` paints none of its pill, heading, or muted colours.
///
/// The open palette covers the terminal-background surface, and a palette
/// with no rows paints no mode pill colour. Heading and muted show either
/// way.
fn wait_preview(tui: &PtySession, shown: &ThemeChrome, gone: &ThemeChrome, what: &str) {
    tui.wait_pred(
        |_| {
            [shown.heading, shown.muted]
                .iter()
                .all(|&(r, g, b)| tui.has_rgb(r, g, b))
                && [gone.pill, gone.heading, gone.muted]
                    .iter()
                    .all(|&(r, g, b)| !tui.has_rgb(r, g, b))
        },
        &format!(
            "{what}: {} heading / muted painted, no {} colour",
            shown.id, gone.id
        ),
        WAIT,
    );
}

fn wait_theme_chrome(tui: &PtySession, theme: &ThemeChrome, expect_toast: bool) {
    let toast = theme.toast();
    tui.wait_pred(
        |screen| {
            theme_is_tree_not_flat(screen)
                && (!expect_toast || screen.contains(&toast))
                && THEMES
                    .iter()
                    .filter(|other| other.id != theme.id)
                    .all(|other| !screen.contains(&other.toast()))
        },
        &format!(
            "{} chrome: tree pill, not Flat paths{}",
            theme.id,
            if expect_toast {
                format!(", toast `{toast}`")
            } else {
                String::new()
            }
        ),
        WAIT,
    );
    tui.wait_has_rgb(theme.surface.0, theme.surface.1, theme.surface.2, WAIT);
    tui.wait_has_rgb(theme.pill.0, theme.pill.1, theme.pill.2, WAIT);
    tui.wait_has_rgb(theme.heading.0, theme.heading.1, theme.heading.2, WAIT);
    tui.wait_has_rgb(theme.muted.0, theme.muted.1, theme.muted.2, WAIT);
    for other in THEMES.iter().filter(|other| other.id != theme.id) {
        for (role, (r, g, b)) in [
            ("surface", other.surface),
            ("mode pill", other.pill),
            ("heading", other.heading),
            ("muted", other.muted),
        ] {
            assert!(
                !tui.has_rgb(r, g, b),
                "{} {role} must not remain after {}:\n{}",
                other.id,
                theme.id,
                tui.screen()
            );
        }
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

/// Palette theme rows save to the last-used theme store; they never
/// rewrite the user config file.
fn assert_config_unchanged(config_home: &Path) {
    let config = user_config_file(config_home);
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        BASELINE_USER_CONFIG,
        "a palette theme row must not rewrite the user config file {}",
        config.display()
    );
}

fn assert_store_names(theme_store: &Path, theme: &ThemeChrome) {
    let stored = fs::read_to_string(theme_store).unwrap_or_else(|err| {
        panic!(
            "Enter on `Theme: {}` must save the theme store {}: {err}",
            theme.label,
            theme_store.display()
        )
    });
    assert!(
        stored.contains(&format!("\"{}\"", theme.id)),
        "theme store must name {} after Enter on its row:\n{stored}",
        theme.id
    );
}

/// Each palette `Theme: …` row applies its theme across the whole TUI.
///
/// Docs: open the command palette, pick a `Theme: <name>` row, Enter
/// applies it and saves it as the last-used theme. Launch seed
/// `WS_STATUS_THEME=tokyo-night`, with the merger graph focused so lane
/// colours are on screen. Each step types the theme slug after `:` and
/// presses Enter, walking Monokai → … → GitHub Dark Dimmed → Tokyo Night.
/// Toast, surface, mode pill, heading, muted (line numbers / graph meta),
/// and graph lane 0 must all match that theme, and the cell colours must
/// change. A no-op, a wrong row, or a toast-only step cannot pass. Each
/// step saves its id to the theme store; the user config file stays
/// byte-identical.
#[test]
fn pty_palette_theme_rows_apply_every_theme() {
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
    tui.wait_contains(" tree", WAIT);
    tui.wait_contains("README.md", WAIT);

    focus_merger_graph(&mut tui);

    wait_theme_chrome(&tui, &THEMES[0], false);
    wait_graph_lanes_match_theme(&tui, &THEMES[0]);

    for step in 1..=THEMES.len() {
        let theme = &THEMES[step % THEMES.len()];
        let before = tui.color_fingerprint();
        apply_palette_theme(&mut tui, theme.id, theme.label);
        wait_theme_chrome(&tui, theme, true);
        wait_graph_lanes_match_theme(&tui, theme);
        assert_ne!(
            tui.color_fingerprint(),
            before,
            "palette step {step} ({}) must repaint cells; a toast-only no-op fails:\n{}",
            theme.id,
            tui.screen()
        );
        assert_store_names(&theme_store, theme);
    }
    assert_config_unchanged(&config_home);
}

/// Focusing a palette theme row previews it; every other way out puts the
/// open-time theme back and saves nothing.
///
/// Docs: moving over `Theme: …` rows previews them, typing that moves the
/// focus follows the same rule, Esc restores the theme active when the
/// palette opened, and only Enter saves. Launch on Tokyo Night with an
/// empty theme store and focus the merger graph:
///
/// 1. `:` `dracula` paints Dracula with the palette open, no toast.
/// 2. `x` leaves no row focused: Tokyo Night again. Backspace: Dracula.
/// 3. Esc closes the palette on Tokyo Night, with no theme toast.
/// 4. `:` `theme` lands on `Theme: Tokyo Night`; Ctrl-n moves to
///    `Theme: Monokai` and paints it; Esc puts Tokyo Night back.
/// 5. The store file still does not exist. Then `:` `nord` Enter applies
///    Nord, toasts `theme: Nord`, and the store names `nord`.
///
/// A preview that saves, a close that keeps the preview, or a preview
/// that only changes the palette text cannot pass.
#[test]
fn pty_palette_theme_row_previews_and_esc_restores() {
    let (_root, workspace) = daily_workspace();
    let theme_store = workspace.join(".e2e-state").join("theme-preview.json");
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_THEME", "tokyo-night"),
            ("WS_STATUS_THEME_STORE", theme_store.to_str().unwrap()),
        ],
    );
    let tokyo = theme_by_id("tokyo-night");
    let dracula = theme_by_id("dracula");
    let monokai = theme_by_id("monokai");
    let nord = theme_by_id("nord");
    tui.wait_contains("README.md", WAIT);
    focus_merger_graph(&mut tui);
    wait_theme_chrome(&tui, tokyo, false);

    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "dracula");
    tui.wait_pred(
        |screen| screen.contains("Theme: Dracula") && !screen.contains(&dracula.toast()),
        "palette filters to Theme: Dracula; no apply toast yet",
        WAIT,
    );
    wait_preview(&tui, dracula, tokyo, "focused Theme: Dracula previews");

    tui.key('x');
    tui.wait_pred(
        |screen| !screen.contains("Theme: Dracula"),
        "`draculax` leaves no Theme row",
        WAIT,
    );
    wait_preview(
        &tui,
        tokyo,
        dracula,
        "no focused theme row restores the open-time theme",
    );

    tui.send_bytes(b"\x7f");
    tui.wait_contains("Theme: Dracula", WAIT);
    wait_preview(
        &tui,
        dracula,
        tokyo,
        "Backspace focuses Theme: Dracula again",
    );

    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("Theme: Dracula"),
        "Esc closes the palette",
        WAIT,
    );
    wait_theme_chrome(&tui, tokyo, false);
    assert!(
        !theme_store.exists(),
        "preview and Esc must not save the theme store {}",
        theme_store.display()
    );

    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "theme");
    tui.wait_pred(
        |screen| screen.contains("Theme: Tokyo Night") && screen.contains("Theme: Monokai"),
        "`theme` lists the Theme rows",
        WAIT,
    );
    tui.ctrl_letter('n');
    wait_preview(&tui, monokai, tokyo, "Ctrl-n onto Theme: Monokai previews");
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("Theme: Monokai"),
        "Esc closes the palette",
        WAIT,
    );
    wait_theme_chrome(&tui, tokyo, false);
    assert!(
        !theme_store.exists(),
        "a moved-over row must not save the theme store {}",
        theme_store.display()
    );

    apply_palette_theme(&mut tui, nord.id, nord.label);
    wait_theme_chrome(&tui, nord, true);
    assert_store_names(&theme_store, nord);
}
