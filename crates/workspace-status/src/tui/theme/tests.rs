use super::*;

#[test]
fn theme_ids_order_slugs_and_labels() {
    let order = [
        ThemeId::TokyoNight,
        ThemeId::Monokai,
        ThemeId::Dracula,
        ThemeId::GruvboxDark,
        ThemeId::CatppuccinMocha,
        ThemeId::Slate,
        ThemeId::SolarizedDark,
        ThemeId::Nord,
        ThemeId::RosePine,
        ThemeId::Kanagawa,
        ThemeId::Everforest,
        ThemeId::OneDark,
        ThemeId::GithubDarkDimmed,
    ];
    assert_eq!(DARK_THEME_IDS, order);
    assert_eq!(THEME_IDS[..13], DARK_THEME_IDS, "dark themes first");
    assert_eq!(THEME_IDS[13..], LIGHT_THEME_IDS, "then the light twins");
    assert_eq!(DEFAULT_THEME_ID, ThemeId::Slate);
    assert_eq!(ThemeId::default(), DEFAULT_THEME_ID);
    let slugs: Vec<_> = DARK_THEME_IDS.iter().map(|id| id.as_str()).collect();
    assert_eq!(
        slugs,
        [
            "tokyo-night",
            "monokai",
            "dracula",
            "gruvbox-dark",
            "catppuccin-mocha",
            "slate",
            "solarized-dark",
            "nord",
            "rose-pine",
            "kanagawa",
            "everforest",
            "one-dark",
            "github-dark-dimmed",
        ]
    );
    let labels: Vec<_> = DARK_THEME_IDS.iter().map(|id| id.label()).collect();
    assert_eq!(
        labels[5..],
        [
            "Slate",
            "Solarized Dark",
            "Nord",
            "Rosé Pine",
            "Kanagawa",
            "Everforest",
            "One Dark",
            "GitHub Dark Dimmed",
        ]
    );
    let labels: Vec<_> = LIGHT_THEME_IDS.iter().map(|id| id.label()).collect();
    assert_eq!(
        labels,
        [
            "Tokyo Night Day",
            "Monokai Light",
            "Dracula Light",
            "Gruvbox Light",
            "Catppuccin Latte",
            "Slate Light",
            "Solarized Light",
            "Nord Light",
            "Rosé Pine Dawn",
            "Kanagawa Lotus",
            "Everforest Light",
            "One Light",
            "GitHub Light",
        ]
    );
    for id in THEME_IDS {
        assert_eq!(id.theme().id, id, "{id:?} preset id");
    }
}

#[test]
fn every_dark_theme_has_a_light_twin_with_its_own_slug() {
    for (dark, light) in DARK_THEME_IDS.into_iter().zip(LIGHT_THEME_IDS) {
        assert!(!dark.is_light(), "{dark:?}");
        assert!(light.is_light(), "{light:?}");
        assert_eq!(dark.twin(), light, "{dark:?} twin");
        assert_eq!(light.twin(), dark, "{light:?} twin");
        assert_eq!((dark.dark_variant(), dark.light_variant()), (dark, light));
        assert_eq!((light.dark_variant(), light.light_variant()), (dark, light));
        assert_ne!(dark.as_str(), light.as_str());
    }
    let slugs: std::collections::HashSet<_> = THEME_IDS.iter().map(|id| id.as_str()).collect();
    assert_eq!(slugs.len(), 26, "every theme has its own slug");
    let labels: std::collections::HashSet<_> = THEME_IDS.iter().map(|id| id.label()).collect();
    assert_eq!(labels.len(), 26, "every theme has its own label");
    for id in THEME_IDS {
        assert_eq!(id.is_light(), LIGHT_THEME_IDS.contains(&id), "{id:?}");
    }
}

#[test]
fn twin_is_an_involution() {
    for id in THEME_IDS {
        assert_ne!(id.twin(), id, "{id:?}");
        assert_ne!(id.twin().is_light(), id.is_light(), "{id:?}");
        assert_eq!(id.twin().twin(), id, "{id:?}");
    }
}

#[test]
fn existing_dark_slugs_still_parse() {
    use ThemeId::*;
    for (slug, id) in [
        ("tokyo-night", TokyoNight),
        ("monokai", Monokai),
        ("dracula", Dracula),
        ("gruvbox-dark", GruvboxDark),
        ("catppuccin-mocha", CatppuccinMocha),
        ("slate", Slate),
        ("solarized-dark", SolarizedDark),
        ("nord", Nord),
        ("rose-pine", RosePine),
        ("kanagawa", Kanagawa),
        ("everforest", Everforest),
        ("one-dark", OneDark),
        ("github-dark-dimmed", GithubDarkDimmed),
    ] {
        assert_eq!(parse_theme_id(slug), Some(id), "{slug}");
        assert!(!id.is_light(), "{slug}");
    }
}

#[test]
fn light_slugs_parse() {
    use ThemeId::*;
    for (slug, id) in [
        ("tokyo-night-day", TokyoNightDay),
        ("monokai-light", MonokaiLight),
        ("dracula-light", DraculaLight),
        ("gruvbox-light", GruvboxLight),
        ("catppuccin-latte", CatppuccinLatte),
        ("slate-light", SlateLight),
        ("solarized-light", SolarizedLight),
        ("nord-light", NordLight),
        ("rose-pine-dawn", RosePineDawn),
        ("kanagawa-lotus", KanagawaLotus),
        ("everforest-light", EverforestLight),
        ("one-light", OneLight),
        ("github-light", GithubLight),
    ] {
        assert_eq!(parse_theme_id(slug), Some(id), "{slug}");
        assert_eq!(theme_id_from(Some(slug), None, None), id, "{slug}");
        assert!(id.is_light(), "{slug}");
    }
}

/// The default role rule darkens and lightens a dark surface, so every
/// light preset sets `sidebar`, `chrome`, and `panel` itself, each a
/// distinct background from the surface and from each other.
#[test]
fn light_themes_set_their_own_background_roles() {
    for id in LIGHT_THEME_IDS {
        let theme = id.theme();
        let (Some(sidebar), Some(chrome), Some(panel)) = (theme.sidebar, theme.chrome, theme.panel)
        else {
            panic!("{id:?} must set sidebar, chrome, and panel");
        };
        let roles: std::collections::HashSet<_> = [theme.surface, sidebar, chrome, panel]
            .into_iter()
            .collect();
        assert_eq!(roles.len(), 4, "{id:?} background roles must differ");
        let surface_l = relative_luminance(hex_color(theme.surface));
        assert!(surface_l > 0.5, "{id:?} surface must be light");
    }
}

#[test]
fn theme_precedence_env_then_last_used_then_config_then_slate() {
    use ThemeId::{Dracula, Monokai, Nord, Slate};
    /// (env, last used, config, want)
    type Case = (
        Option<&'static str>,
        Option<ThemeId>,
        Option<ThemeId>,
        ThemeId,
    );
    let table: [Case; 14] = [
        (Some("monokai"), Some(Nord), Some(Dracula), Monokai),
        (Some("monokai"), None, Some(Dracula), Monokai),
        (Some("monokai"), Some(Nord), None, Monokai),
        (Some("monokai"), None, None, Monokai),
        (Some("slate"), Some(Nord), Some(Dracula), Slate),
        (None, Some(Nord), Some(Dracula), Nord),
        (None, Some(Slate), Some(Dracula), Slate),
        (None, Some(Nord), None, Nord),
        (None, None, Some(Dracula), Dracula),
        (None, None, None, Slate),
        (Some("nope"), Some(Nord), Some(Dracula), Nord),
        (Some("nope"), None, Some(Dracula), Dracula),
        (Some(""), None, None, Slate),
        (Some(" monokai"), Some(Nord), Some(Dracula), Nord),
    ];
    for (env, last_used, config, want) in table {
        assert_eq!(
            theme_id_from(env, last_used, config),
            want,
            "env {env:?}, last used {last_used:?}, config {config:?}"
        );
    }
    for bad in ["", "nope", " monokai", "Monokai"] {
        assert_eq!(
            theme_id_from(Some(bad), None, Some(Dracula)),
            Dracula,
            "env {bad:?}"
        );
    }
}

#[test]
fn resolve_known_and_fallback() {
    assert_eq!(theme_id_from(None, None, None), ThemeId::Slate);
    assert_eq!(theme_id_from(Some(""), None, None), ThemeId::Slate);
    assert_eq!(theme_id_from(Some("nope"), None, None), ThemeId::Slate);
    assert_eq!(parse_theme_id("solarized"), None, "exact slug only");
    assert_eq!(parse_theme_id("Dracula"), None, "case-sensitive");
    for id in THEME_IDS {
        assert_eq!(theme_id_from(Some(id.as_str()), None, None), id);
        assert_eq!(parse_theme_id(id.as_str()), Some(id));
        assert!(!id.label().is_empty());
        assert!(id.theme().surface.starts_with('#'));
        let lanes = id.theme().lane_colors;
        assert_eq!(lanes.len(), 8);
        let unique: std::collections::HashSet<_> = lanes.iter().copied().collect();
        assert_eq!(unique.len(), 8, "{id:?} lane colours must be distinct");
    }
    assert_eq!(
        ThemeId::TokyoNight.theme().lane_colors,
        workspace_status_graph::DEFAULT_LANE_COLORS
    );
    assert_ne!(
        ThemeId::Monokai.theme().lane_colors[0],
        ThemeId::TokyoNight.theme().lane_colors[0]
    );
}

/// A theme that leaves `sidebar` / `chrome` / `panel` unset gets the
/// surface mixed 22% / 38% toward black and 5% toward white. A set role
/// is used verbatim.
#[test]
fn background_roles_use_the_default_rule_or_the_theme_hex() {
    let mut defaulted = 0;
    let mut explicit = 0;
    for id in THEME_IDS {
        let theme = id.theme();
        let pal = id.palette();
        let surface = hex_color(theme.surface);
        assert_eq!(pal.surface, surface, "{id:?} surface");
        for (name, hex, got, target, percent) in [
            (
                "sidebar",
                theme.sidebar,
                pal.sidebar,
                Color::Rgb(0, 0, 0),
                22,
            ),
            ("chrome", theme.chrome, pal.chrome, Color::Rgb(0, 0, 0), 38),
            (
                "panel",
                theme.panel,
                pal.panel,
                Color::Rgb(255, 255, 255),
                5,
            ),
        ] {
            match hex {
                Some(hex) => {
                    explicit += 1;
                    assert_eq!(got, hex_color(hex), "{id:?} {name} is the theme hex");
                }
                None => {
                    defaulted += 1;
                    assert_eq!(
                        got,
                        mix_toward(surface, target, percent),
                        "{id:?} {name} follows the default rule"
                    );
                }
            }
        }
    }
    assert!(defaulted > 0 && explicit > 0);
    // Hand-computed rule values (rounded to nearest per channel).
    let tokyo = ThemeId::TokyoNight.palette();
    assert_eq!(tokyo.sidebar, Color::Rgb(0x14, 0x15, 0x1e));
    assert_eq!(tokyo.chrome, Color::Rgb(0x10, 0x11, 0x18));
    assert_eq!(tokyo.panel, Color::Rgb(0x25, 0x26, 0x31));
    let slate = ThemeId::Slate.palette();
    assert_eq!(slate.sidebar, Color::Rgb(0x15, 0x1a, 0x21));
    assert_eq!(slate.chrome, Color::Rgb(0x0d, 0x11, 0x16));
    assert_eq!(slate.panel, Color::Rgb(0x19, 0x20, 0x2a));
    // The current five keep the rule except where a panel had to be
    // lifted less to keep popup text over its floor.
    for id in [ThemeId::TokyoNight, ThemeId::CatppuccinMocha] {
        let theme = id.theme();
        assert_eq!(
            (theme.sidebar, theme.chrome, theme.panel),
            (None, None, None)
        );
    }
    for (id, panel) in [
        (ThemeId::Monokai, "#30312b"),
        (ThemeId::Dracula, "#2e303c"),
        (ThemeId::GruvboxDark, "#2c2c2c"),
    ] {
        let theme = id.theme();
        assert_eq!((theme.sidebar, theme.chrome), (None, None), "{id:?}");
        assert_eq!(theme.panel, Some(panel), "{id:?}");
    }
}

/// The dialog backdrop is meant to recede, so it has no contrast floor;
/// it only has to show. Muted text and every pane role move visibly
/// toward `surface` on every theme.
#[test]
fn backdrop_dim_is_visible_on_every_theme() {
    use crate::tui::render::BACKDROP_PERCENT;
    for id in THEME_IDS {
        let palette = id.palette();
        assert!(
            matches!(palette.surface, Color::Rgb(..)),
            "{id:?} surface is RGB"
        );
        for (name, color) in [
            ("muted", palette.muted),
            ("file", palette.file),
            ("sidebar", palette.sidebar),
            ("chrome", palette.chrome),
        ] {
            assert_ne!(
                mix_toward(color, palette.surface, BACKDROP_PERCENT),
                color,
                "{id:?} {name} must dim"
            );
        }
    }
}

#[test]
fn mix_toward_rounds_per_channel_and_skips_non_rgb() {
    let base = Color::Rgb(10, 100, 200);
    assert_eq!(mix_toward(base, Color::Rgb(0, 0, 0), 0), base);
    assert_eq!(
        mix_toward(base, Color::Rgb(255, 255, 255), 100),
        Color::Rgb(255, 255, 255)
    );
    // 10 - 10*0.22 = 7.8 -> 8; 100 - 22 = 78; 200 - 44 = 156.
    assert_eq!(
        mix_toward(base, Color::Rgb(0, 0, 0), 22),
        Color::Rgb(8, 78, 156)
    );
    // 10 + 245*0.05 = 22.25 -> 22; 100 + 7.75 -> 108; 200 + 2.75 -> 203.
    assert_eq!(
        mix_toward(base, Color::Rgb(255, 255, 255), 5),
        Color::Rgb(22, 108, 203)
    );
    assert_eq!(
        mix_toward(Color::Reset, Color::Rgb(0, 0, 0), 22),
        Color::Reset
    );
}

#[test]
fn hex_parses_tokyo_heading() {
    assert_eq!(hex_color("#7dcfff"), Color::Rgb(0x7d, 0xcf, 0xff));
    assert_eq!(hex_color("bad"), Color::White);
}

#[test]
fn flash_ramp_starts_at_flash_and_fades() {
    for id in THEME_IDS {
        let pal = id.palette();
        assert_eq!(pal.flash_ramp[0], pal.flash);
        assert_eq!(pal.flash_bg(1.0), Some(pal.flash));
        assert_eq!(pal.flash_bg(0.0), None);
        assert_ne!(pal.flash_bg(0.3), pal.flash_bg(1.0));
    }
}

#[test]
fn flash_bg_for_dispatches_kind_ramps() {
    use super::super::watch::FlashKind;
    for id in THEME_IDS {
        let pal = id.palette();
        assert_eq!(pal.flash_ramp[0], pal.flash);
        assert_eq!(pal.flash_update_ramp[0], pal.flash_update);
        assert_eq!(pal.flash_remove_ramp[0], pal.flash_remove);
        assert_eq!(pal.flash_bg_for(FlashKind::Add, 1.0), Some(pal.flash));
        assert_eq!(
            pal.flash_bg_for(FlashKind::Update, 1.0),
            Some(pal.flash_update)
        );
        assert_eq!(
            pal.flash_bg_for(FlashKind::Remove, 1.0),
            Some(pal.flash_remove)
        );
        assert_eq!(pal.flash_bg_for(FlashKind::Add, 0.0), None);
        assert_eq!(pal.flash_bg_for(FlashKind::Update, 0.0), None);
        assert_eq!(pal.flash_bg_for(FlashKind::Remove, 0.0), None);
        assert_eq!(pal.flash_bg(1.0), pal.flash_bg_for(FlashKind::Add, 1.0));
        assert_ne!(pal.flash, pal.flash_update);
        assert_ne!(pal.flash, pal.flash_remove);
        assert_ne!(pal.flash_update, pal.flash_remove);
        assert_ne!(
            pal.flash_bg_for(FlashKind::Add, 0.3),
            pal.flash_bg_for(FlashKind::Add, 1.0)
        );
        assert_ne!(
            pal.flash_bg_for(FlashKind::Update, 0.3),
            pal.flash_bg_for(FlashKind::Update, 1.0)
        );
        assert_ne!(
            pal.flash_bg_for(FlashKind::Remove, 0.3),
            pal.flash_bg_for(FlashKind::Remove, 1.0)
        );
    }
    let tokyo = ThemeId::TokyoNight.palette();
    // Leftover TTY e2e hardcodes Tokyo Night ramp[0] RGB. Keep in sync.
    assert_eq!(tokyo.flash, Color::Rgb(0x51, 0x66, 0x43));
    assert_eq!(tokyo.flash_update, Color::Rgb(0x6d, 0x59, 0x42));
    assert_eq!(tokyo.flash_remove, Color::Rgb(0x77, 0x41, 0x52));
    assert_eq!(tokyo.diff_add_bg, Color::Rgb(0x3f, 0x4d, 0x39));
    assert_eq!(tokyo.diff_del_bg, Color::Rgb(0x58, 0x34, 0x43));
    assert_eq!(tokyo.diff_add_word_bg, Color::Rgb(0x42, 0x68, 0x32));
    assert_eq!(tokyo.diff_del_word_bg, Color::Rgb(0x81, 0x3d, 0x59));
}

fn srgb_lin(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn relative_luminance(color: Color) -> f64 {
    let Color::Rgb(r, g, b) = color else {
        panic!("{color:?} must be rgb");
    };
    0.2126 * srgb_lin(r) + 0.7152 * srgb_lin(g) + 0.0722 * srgb_lin(b)
}

fn contrast_ratio(fg: Color, bg: Color) -> f64 {
    let l1 = relative_luminance(fg);
    let l2 = relative_luminance(bg);
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// HSL hue (degrees) and saturation (0..=1) of an RGB colour.
fn hue_sat(color: Color) -> (f64, f64) {
    let Color::Rgb(r, g, b) = color else {
        panic!("{color:?} must be rgb");
    };
    let [r, g, b] = [r, g, b].map(|c| f64::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta == 0.0 {
        return (0.0, 0.0);
    }
    let light = (max + min) / 2.0;
    let sat = delta / (1.0 - (2.0 * light - 1.0).abs());
    let hue = if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (hue, sat)
}

/// Graph lanes paint on the left pane (`sidebar`) and on the right pane
/// (`surface`). Every lane colour must reach the 3:1 non-text floor on
/// both, so a light terminal still shows each rail and node.
#[test]
fn lane_colors_meet_floor_on_sidebar_and_surface() {
    const LANE_FLOOR: f64 = 3.0;
    let mut failures = Vec::new();
    for id in THEME_IDS {
        let pal = id.palette();
        for (lane, fg) in id.lane_colors().into_iter().enumerate() {
            for (role, bg) in [("sidebar", pal.sidebar), ("surface", pal.surface)] {
                let ratio = contrast_ratio(fg, bg);
                if ratio < LANE_FLOOR {
                    failures.push(format!("{id:?} lane[{lane}] {fg:?} on {role} {ratio:.2}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "lanes below {LANE_FLOOR}:1:\n{}",
        failures.join("\n")
    );
}

/// Toasts paint `repo` text on the add-flash ramp as they fade. Every
/// step must keep the text readable, on dark and light themes.
#[test]
fn toast_text_meets_floor_on_every_flash_step() {
    const TOAST_FLOOR: f64 = 3.0;
    assert_eq!(THEME_IDS.len(), 26);
    for id in THEME_IDS {
        let pal = id.palette();
        for (step, bg) in pal.flash_ramp.into_iter().enumerate() {
            let ratio = contrast_ratio(pal.repo, bg);
            assert!(
                ratio >= TOAST_FLOOR,
                "{id:?} toast repo on flash_ramp[{step}] contrast {ratio:.2} < {TOAST_FLOOR}"
            );
        }
    }
}

/// A PR change toast holds on `panel`: its state glyph, `repo` text,
/// and `muted` detail stay readable there on dark and light themes.
/// (On the add-flash peak several of them fall under 2:1.)
#[test]
fn pr_toast_colors_meet_floor_on_the_panel() {
    const TOAST_FLOOR: f64 = 3.0;
    for id in THEME_IDS {
        let pal = id.palette();
        for (role, fg) in [
            ("repo", pal.repo),
            ("muted", pal.muted),
            ("deleted", pal.deleted),
            ("added", pal.added),
            ("branch_feature", pal.branch_feature),
        ] {
            let ratio = contrast_ratio(fg, pal.panel);
            assert!(
                ratio >= TOAST_FLOOR,
                "{id:?} PR toast {role} on panel contrast {ratio:.2} < {TOAST_FLOOR}"
            );
        }
    }
}

/// Minimum contrast between a changed-word bg and its row bg.
const WORD_VS_ROW_FLOOR: f64 = 1.25;

/// Text roles meet AA on the surface of every theme, dark and light (the
/// name predates the light twins). `muted` stays unique across all of
/// them for the TTY e2e chrome checks.
#[test]
fn dark_surface_foregrounds_meet_aa_contrast() {
    const AA: f64 = 4.5;
    const DELETED_FLOOR: f64 = 4.0;
    for id in THEME_IDS {
        let theme = id.theme();
        let surface = hex_color(theme.surface);
        let pal = id.palette();
        let tokens = [
            ("muted", pal.muted),
            ("viewed", pal.viewed),
            ("heading", pal.heading),
            ("repo", pal.repo),
            ("dir", pal.dir),
            ("file", pal.file),
            ("renamed", pal.renamed),
            ("branch_default", pal.branch_default),
            ("branch_feature", pal.branch_feature),
            ("added", pal.added),
            ("modified", pal.modified),
            ("head_mark", pal.head_mark),
            ("cursor", pal.cursor),
            ("diff_hunk", pal.diff_hunk),
        ];
        for (name, fg) in tokens {
            let ratio = contrast_ratio(fg, surface);
            assert!(
                ratio >= AA,
                "{id:?} {name} contrast {ratio:.2} < {AA} vs {}",
                theme.surface
            );
        }
        let deleted = contrast_ratio(pal.deleted, surface);
        assert!(
            deleted >= DELETED_FLOOR,
            "{id:?} deleted contrast {deleted:.2} < {DELETED_FLOOR} vs {}",
            theme.surface
        );
        let cursor_bg = pal.cursor_bg;
        for (name, fg) in [("muted", pal.muted), ("viewed", pal.viewed)] {
            let ratio = contrast_ratio(fg, cursor_bg);
            assert!(
                ratio >= AA,
                "{id:?} {name} contrast {ratio:.2} < {AA} vs cursor_bg {}",
                theme.palette.cursor_bg
            );
        }
        assert_ne!(
            pal.viewed, pal.muted,
            "{id:?} viewed eye must not use muted"
        );
        assert_ne!(
            pal.viewed, pal.renamed,
            "{id:?} viewed must not reuse renamed"
        );
        assert_ne!(
            pal.viewed, pal.heading,
            "{id:?} viewed must not reuse heading"
        );
        assert_ne!(pal.viewed, pal.dir, "{id:?} viewed must not reuse dir");
        const ROW_BG_FLOOR: f64 = 3.0;
        for (name, fg, bg) in [
            ("added on diff_add_bg", pal.added, pal.diff_add_bg),
            ("deleted on diff_del_bg", pal.deleted, pal.diff_del_bg),
            ("repo on diff_add_bg", pal.repo, pal.diff_add_bg),
            ("repo on diff_del_bg", pal.repo, pal.diff_del_bg),
            ("muted on diff_add_bg", pal.muted, pal.diff_add_bg),
            ("muted on diff_del_bg", pal.muted, pal.diff_del_bg),
            ("repo on diff_add_word_bg", pal.repo, pal.diff_add_word_bg),
            ("repo on diff_del_word_bg", pal.repo, pal.diff_del_word_bg),
        ] {
            let ratio = contrast_ratio(fg, bg);
            assert!(
                ratio >= ROW_BG_FLOOR,
                "{id:?} {name} contrast {ratio:.2} < {ROW_BG_FLOOR}"
            );
        }
        assert_ne!(
            pal.diff_add_bg, pal.diff_del_bg,
            "{id:?} add/del row backgrounds must differ"
        );
        assert_ne!(
            pal.diff_add_word_bg, pal.diff_del_word_bg,
            "{id:?} add/del word backgrounds must differ"
        );
        // Word bg reads as a highlight on its row: visibly different,
        // same hue family, at least as saturated.
        const WORD_HUE_MAX_DEG: f64 = 20.0;
        for (name, word, row) in [
            ("add", pal.diff_add_word_bg, pal.diff_add_bg),
            ("del", pal.diff_del_word_bg, pal.diff_del_bg),
        ] {
            let ratio = contrast_ratio(word, row);
            assert!(
                ratio >= WORD_VS_ROW_FLOOR,
                "{id:?} {name} word bg vs row bg contrast {ratio:.2} < {WORD_VS_ROW_FLOOR}"
            );
            let (word_hue, word_sat) = hue_sat(word);
            let (row_hue, row_sat) = hue_sat(row);
            let diff = (word_hue - row_hue).abs() % 360.0;
            let hue_dist = diff.min(360.0 - diff);
            assert!(
                    hue_dist <= WORD_HUE_MAX_DEG,
                    "{id:?} {name} word bg hue {word_hue:.1} drifts {hue_dist:.1}° from row hue {row_hue:.1}"
                );
            assert!(
                word_sat >= row_sat,
                "{id:?} {name} word bg saturation {word_sat:.2} < row {row_sat:.2}"
            );
        }
    }
    let muteds: Vec<_> = THEME_IDS
        .iter()
        .map(|id| id.theme().palette.muted)
        .collect();
    let unique: std::collections::HashSet<_> = muteds.iter().copied().collect();
    assert_eq!(
        unique.len(),
        muteds.len(),
        "muted hexes must stay unique for TTY e2e theme chrome: {muteds:?}"
    );
}

/// The curly underline under commented code is a non-text mark: it
/// meets the WCAG 3:1 non-text floor on every row background a diff or
/// file-view code line can have (the surface, the add / remove rows,
/// the cursor bar), on every theme, light twins included. Changed-word
/// highlights are left out: they are short runs inside a row, and
/// several dark themes put every accent under 3:1 on them.
#[test]
fn comment_underline_meets_non_text_floor_on_row_backgrounds() {
    const NON_TEXT: f64 = 3.0;
    assert_eq!(THEME_IDS.len(), 26);
    for id in THEME_IDS {
        let pal = id.palette();
        let curl = pal.comment_underline();
        assert_ne!(curl, pal.surface, "{id:?} curl must differ from surface");
        assert_ne!(curl, pal.repo, "{id:?} curl must differ from code text");
        for (name, bg) in [
            ("surface", pal.surface),
            ("diff_add_bg", pal.diff_add_bg),
            ("diff_del_bg", pal.diff_del_bg),
            ("cursor_bg", pal.cursor_bg),
        ] {
            let ratio = contrast_ratio(curl, bg);
            assert!(
                ratio >= NON_TEXT,
                "{id:?} comment underline on {name} contrast {ratio:.2} < {NON_TEXT}"
            );
        }
    }
}

/// Icon popover and `?` legend text. Popups paint on `panel`; the
/// same roles paint the right pane on the surface, the
/// left pane on `sidebar` and the bars on `chrome`:
/// popover headings and field values in their pane colour roles,
/// muted labels and notes, legend glyphs in the colour their pane
/// paints them (`head_mark` for the checkout mark and `[HEAD]`,
/// `cursor` for the cursor bar), and the `❯` focus marker and cues in
/// `cursor`. Every role meets AA on the surface, `panel`,
/// `sidebar`, and `chrome` (`deleted` its 4.0 floor). A pinned popover's focused line puts the same text on
/// `cursor_bg`: 3.5, and `deleted` 2.5 like the hovered tab close.
/// Key chips paint the surface (bars) or `panel` (popups) on `cursor`
/// (enabled) or `muted` (disabled): AA.
#[test]
fn popover_and_legend_text_meets_floors_on_surface_and_focus() {
    const AA: f64 = 4.5;
    const DELETED_ON_SURFACE: f64 = 4.0;
    const ON_FOCUS: f64 = 3.5;
    const DELETED_ON_FOCUS: f64 = 2.5;
    for id in THEME_IDS {
        let pal = id.palette();
        let surface = hex_color(id.theme().surface);
        let base_floor = |deleted: bool| if deleted { DELETED_ON_SURFACE } else { AA };
        let roles = [
            ("heading", pal.heading),
            ("repo", pal.repo),
            ("dir", pal.dir),
            ("file", pal.file),
            ("muted", pal.muted),
            ("added", pal.added),
            ("modified", pal.modified),
            ("deleted", pal.deleted),
            ("renamed", pal.renamed),
            ("viewed", pal.viewed),
            ("branch_default", pal.branch_default),
            ("branch_feature", pal.branch_feature),
            ("head_mark", pal.head_mark),
            ("cursor", pal.cursor),
        ];
        for (name, fg) in roles {
            let deleted = name == "deleted";
            for (bg_name, bg, floor) in [
                ("surface", surface, base_floor(deleted)),
                ("panel", pal.panel, base_floor(deleted)),
                ("sidebar", pal.sidebar, base_floor(deleted)),
                ("chrome", pal.chrome, base_floor(deleted)),
                (
                    "cursor_bg",
                    pal.cursor_bg,
                    if deleted { DELETED_ON_FOCUS } else { ON_FOCUS },
                ),
            ] {
                let ratio = contrast_ratio(fg, bg);
                assert!(
                    ratio >= floor,
                    "{id:?} popover {name} on {bg_name} {ratio:.2} < {floor}"
                );
            }
        }
        for (name, chip_bg) in [("enabled", pal.cursor), ("disabled", pal.muted)] {
            for (fg_name, fg) in [("surface", surface), ("panel", pal.panel)] {
                let ratio = contrast_ratio(fg, chip_bg);
                assert!(
                    ratio >= AA,
                    "{id:?} {name} key chip ({fg_name}) {ratio:.2} < {AA}"
                );
            }
        }
    }
}

#[test]
fn cursor_tint_keeps_word_highlight_and_text_readable() {
    const TEXT_FLOOR: f64 = 3.0;
    for id in THEME_IDS {
        let pal = id.palette();
        for (name, row, word) in [
            ("add", pal.diff_add_bg, pal.diff_add_word_bg),
            ("del", pal.diff_del_bg, pal.diff_del_word_bg),
        ] {
            for (overlay_name, overlay) in [
                ("cursor_bg", pal.cursor_bg),
                ("cursor_bg_inactive", pal.cursor_bg_inactive),
            ] {
                let tinted_row = pal.cursor_tint(row, overlay);
                let tinted_word = pal.cursor_tint(word, overlay);
                let ctx = format!("{id:?} {name} on {overlay_name}");
                let ratio = contrast_ratio(tinted_word, tinted_row);
                assert!(
                    ratio >= WORD_VS_ROW_FLOOR,
                    "{ctx}: tinted word vs row {ratio:.2} < {WORD_VS_ROW_FLOOR}"
                );
                for (bg_name, bg) in [("row", tinted_row), ("word", tinted_word)] {
                    let ratio = contrast_ratio(pal.repo, bg);
                    assert!(
                        ratio >= TEXT_FLOOR,
                        "{ctx}: repo on tinted {bg_name} {ratio:.2} < {TEXT_FLOOR}"
                    );
                }
                assert_ne!(tinted_row, row, "{ctx}: tint must move the row bg");
                assert_ne!(tinted_word, word, "{ctx}: tint must move the word bg");
            }
            assert_ne!(
                pal.cursor_tint(row, pal.cursor_bg),
                pal.cursor_tint(row, pal.cursor_bg_inactive),
                "{id:?} {name}: focused and inactive tints must differ"
            );
            assert_ne!(
                pal.cursor_tint(word, pal.cursor_bg),
                pal.cursor_tint(word, pal.cursor_bg_inactive),
                "{id:?} {name}: focused and inactive word tints must differ"
            );
        }
    }
}

#[test]
fn cursor_tint_follows_the_theme_shift_and_clamps() {
    let mut pal = ThemeId::TokyoNight.palette();
    pal.surface = Color::Rgb(100, 100, 100);
    // Lighter cursor bar: +8 per channel at full strength, +6 at 3/4.
    assert_eq!(
        pal.cursor_tint(Color::Rgb(10, 20, 250), Color::Rgb(108, 108, 108)),
        Color::Rgb(16, 26, 255)
    );
    // Darker cursor bar darkens the bg and clamps at 0.
    assert_eq!(
        pal.cursor_tint(Color::Rgb(4, 50, 50), Color::Rgb(92, 92, 92)),
        Color::Rgb(0, 44, 44)
    );
    assert_eq!(
        pal.cursor_tint(Color::Reset, pal.cursor_bg),
        pal.cursor_bg,
        "non-RGB bg falls back to the flat cursor bg"
    );
    pal.surface = Color::Reset;
    assert_eq!(
        pal.cursor_tint(pal.diff_add_bg, pal.cursor_bg),
        pal.cursor_bg,
        "non-RGB surface falls back to the flat cursor bg"
    );
}

/// `border_dim` is a near-surface rule: less contrast on the surface than
/// `muted` on dark and light themes alike. Hexes are locked and unique.
#[test]
fn border_dim_is_unique_and_darker_than_muted() {
    const LOCKED: [(ThemeId, &str); 26] = [
        (ThemeId::TokyoNight, "#3b4261"),
        (ThemeId::Monokai, "#49483e"),
        (ThemeId::Dracula, "#44475a"),
        (ThemeId::GruvboxDark, "#504945"),
        (ThemeId::CatppuccinMocha, "#45475a"),
        (ThemeId::Slate, "#2b3442"),
        (ThemeId::SolarizedDark, "#1a4957"),
        (ThemeId::Nord, "#434c5e"),
        (ThemeId::RosePine, "#403d52"),
        (ThemeId::Kanagawa, "#363646"),
        (ThemeId::Everforest, "#475258"),
        (ThemeId::OneDark, "#3e4451"),
        (ThemeId::GithubDarkDimmed, "#444c56"),
        (ThemeId::TokyoNightDay, "#bec1ca"),
        (ThemeId::MonokaiLight, "#d0d0cb"),
        (ThemeId::DraculaLight, "#d2cfc2"),
        (ThemeId::GruvboxLight, "#d5ccaa"),
        (ThemeId::CatppuccinLatte, "#ced1d9"),
        (ThemeId::SlateLight, "#cacdd2"),
        (ThemeId::SolarizedLight, "#ccd0c3"),
        (ThemeId::NordLight, "#c6cad0"),
        (ThemeId::RosePineDawn, "#d9d4d6"),
        (ThemeId::KanagawaLotus, "#cfcaac"),
        (ThemeId::EverforestLight, "#dcdbd3"),
        (ThemeId::OneLight, "#d3d4d5"),
        (ThemeId::GithubLight, "#d2d3d4"),
    ];
    assert_eq!(LOCKED.map(|(id, _)| id), THEME_IDS);
    let mut hexes = Vec::new();
    for (id, hex) in LOCKED {
        let theme = id.theme();
        assert_eq!(theme.palette.border_dim, hex, "{id:?} borderDim hex");
        let pal = id.palette();
        assert_eq!(pal.border_dim, hex_color(hex), "{id:?} palette.border_dim");
        let dim = contrast_ratio(pal.border_dim, pal.surface);
        let muted = contrast_ratio(pal.muted, pal.surface);
        assert!(
                dim < muted,
                "{id:?} contrast(border_dim, surface)={dim:.2} must be < contrast(muted, surface)={muted:.2}"
            );
        hexes.push(hex);
    }
    let unique: std::collections::HashSet<_> = hexes.iter().copied().collect();
    assert_eq!(
        unique.len(),
        hexes.len(),
        "borderDim hexes must be unique across themes: {hexes:?}"
    );
}

/// Idle `[✗]` is a neutral gray (R=G=B) that reads dimmer than the
/// `muted` tab label (less contrast on the surface, on dark and light
/// themes alike), yet stays legible on the active tab. Floors:
/// `muted`/`tab_close` contrast >= 1.5 (visibly dimmer), `tab_close` >= 2.5
/// on `cursor_bg` and >= 3.0 on the surface. Hover is the theme's `deleted`
/// red on every theme: >= 2.5 on `cursor_bg`, and more contrast on the
/// surface than the idle gray.
#[test]
fn tab_close_is_dimmer_than_muted_and_hover_is_red() {
    const DIMMER_THAN_MUTED: f64 = 1.5;
    const ON_CURSOR_BG: f64 = 2.5;
    const ON_SURFACE: f64 = 3.0;
    const HOVER_ON_CURSOR_BG: f64 = 2.5;
    for id in THEME_IDS {
        let pal = id.palette();
        let surface = hex_color(id.theme().surface);
        let close_on_surface = contrast_ratio(pal.tab_close, surface);
        assert!(
            close_on_surface < contrast_ratio(pal.muted, surface),
            "{id:?} tab_close must have less contrast on the surface than muted"
        );
        assert_eq!(
            pal.tab_close_hover, pal.deleted,
            "{id:?} tab_close_hover must be the deleted red"
        );
        let Color::Rgb(r, g, b) = pal.tab_close else {
            panic!("{id:?} tab_close must be rgb");
        };
        assert!(r == g && g == b, "{id:?} tab_close must be neutral gray");
        assert_ne!(pal.tab_close, pal.border_dim, "{id:?} tab_close");
        assert_ne!(pal.tab_close, pal.cursor_bg, "{id:?} tab_close");
        for (name, ratio, floor) in [
            (
                "muted vs tab_close",
                contrast_ratio(pal.muted, pal.tab_close),
                DIMMER_THAN_MUTED,
            ),
            (
                "tab_close on cursor_bg",
                contrast_ratio(pal.tab_close, pal.cursor_bg),
                ON_CURSOR_BG,
            ),
            ("tab_close on surface", close_on_surface, ON_SURFACE),
            (
                "tab_close_hover on cursor_bg",
                contrast_ratio(pal.tab_close_hover, pal.cursor_bg),
                HOVER_ON_CURSOR_BG,
            ),
        ] {
            assert!(ratio >= floor, "{id:?} {name} {ratio:.2} < {floor}");
        }
        assert!(
            contrast_ratio(pal.tab_close_hover, surface) > close_on_surface,
            "{id:?} tab_close_hover must stand out more on the surface than tab_close"
        );
    }
}

/// The unfocused selection bar is a weaker shift than the focused one:
/// its luminance sits strictly between the surface and `cursor_bg`, on
/// dark and light themes alike. Hexes are locked and unique.
#[test]
fn cursor_bg_inactive_is_darker_than_cursor_bg() {
    const LOCKED: [(ThemeId, &str); 26] = [
        (ThemeId::TokyoNight, "#21273e"),
        (ThemeId::Monokai, "#32322a"),
        (ThemeId::Dracula, "#363848"),
        (ThemeId::GruvboxDark, "#32302f"),
        (ThemeId::CatppuccinMocha, "#272839"),
        (ThemeId::Slate, "#19212a"),
        (ThemeId::SolarizedDark, "#033342"),
        (ThemeId::Nord, "#353d4a"),
        (ThemeId::RosePine, "#232632"),
        (ThemeId::Kanagawa, "#272936"),
        (ThemeId::Everforest, "#344045"),
        (ThemeId::OneDark, "#2d3643"),
        (ThemeId::GithubDarkDimmed, "#28323f"),
        (ThemeId::TokyoNightDay, "#d2dae7"),
        (ThemeId::MonokaiLight, "#ecece6"),
        (ThemeId::DraculaLight, "#f2ede8"),
        (ThemeId::GruvboxLight, "#f5e2b7"),
        (ThemeId::CatppuccinLatte, "#dee6f5"),
        (ThemeId::SlateLight, "#e5ebf3"),
        (ThemeId::SolarizedLight, "#ecede1"),
        (ThemeId::NordLight, "#e0e6ee"),
        (ThemeId::RosePineDawn, "#e9e9e4"),
        (ThemeId::KanagawaLotus, "#e5e1b9"),
        (ThemeId::EverforestLight, "#eff3eb"),
        (ThemeId::OneLight, "#ebeff9"),
        (ThemeId::GithubLight, "#ebf3fc"),
    ];
    assert_eq!(LOCKED.map(|(id, _)| id), THEME_IDS);
    let mut hexes = Vec::new();
    for (id, hex) in LOCKED {
        let theme = id.theme();
        assert_eq!(
            theme.palette.cursor_bg_inactive, hex,
            "{id:?} cursorBgInactive hex"
        );
        let pal = id.palette();
        assert_eq!(
            pal.cursor_bg_inactive,
            hex_color(hex),
            "{id:?} palette.cursor_bg_inactive"
        );
        assert_ne!(
            pal.cursor_bg_inactive, pal.cursor_bg,
            "{id:?} inactive cursor bg must not match focused cursor_bg"
        );
        let inactive_l = relative_luminance(pal.cursor_bg_inactive);
        let cursor_l = relative_luminance(pal.cursor_bg);
        let surface_l = relative_luminance(hex_color(theme.surface));
        let (lo, hi) = (cursor_l.min(surface_l), cursor_l.max(surface_l));
        assert!(
                lo < inactive_l && inactive_l < hi,
                "{id:?} luminance(cursor_bg_inactive)={inactive_l:.4} must sit strictly between surface={surface_l:.4} and cursor_bg={cursor_l:.4}"
            );
        hexes.push(hex);
    }
    let unique: std::collections::HashSet<_> = hexes.iter().copied().collect();
    assert_eq!(
        unique.len(),
        hexes.len(),
        "cursorBgInactive hexes must be unique across themes: {hexes:?}"
    );
}
