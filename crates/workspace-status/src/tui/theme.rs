//! Built-in TUI colour themes.
//!
//! Launch seed is `WS_STATUS_THEME`. `T` cycles in the current session only.
//! There is no theme file; the cycle stays in the current session.

use ratatui::style::Color;

/// Built-in dark theme identifiers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeId {
    #[default]
    TokyoNight,
    Monokai,
    Dracula,
    GruvboxDark,
    CatppuccinMocha,
}

/// Cycle order for `T`.
pub const THEME_IDS: [ThemeId; 5] = [
    ThemeId::TokyoNight,
    ThemeId::Monokai,
    ThemeId::Dracula,
    ThemeId::GruvboxDark,
    ThemeId::CatppuccinMocha,
];

/// Default when `WS_STATUS_THEME` is unset or unknown.
pub const DEFAULT_THEME_ID: ThemeId = ThemeId::TokyoNight;

/// Ratatui colours for the active theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub heading: Color,
    pub repo: Color,
    pub dir: Color,
    pub file: Color,
    pub muted: Color,
    pub added: Color,
    pub modified: Color,
    pub deleted: Color,
    pub renamed: Color,
    pub branch_default: Color,
    pub branch_feature: Color,
    pub head_mark: Color,
    pub cursor: Color,
    pub cursor_bg: Color,
    pub diff_hunk: Color,
    pub flash: Color,
}

/// Semantic colours used by the ratatui paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub heading: &'static str,
    pub repo: &'static str,
    pub dir: &'static str,
    pub file: &'static str,
    pub muted: &'static str,
    pub added: &'static str,
    pub modified: &'static str,
    pub deleted: &'static str,
    pub renamed: &'static str,
    pub branch_default: &'static str,
    pub branch_feature: &'static str,
    pub head_mark: &'static str,
    pub cursor: &'static str,
    pub cursor_bg: &'static str,
    pub diff_hunk: &'static str,
    pub flash: &'static str,
}

/// Status-bar pill hex pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePill {
    pub mode_bg: &'static str,
    pub mode_fg: &'static str,
    pub diff_bg: &'static str,
    pub diff_fg: &'static str,
    pub filter_bg: &'static str,
    pub filter_fg: &'static str,
}

/// One built-in theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub id: ThemeId,
    pub label: &'static str,
    pub surface: &'static str,
    pub palette: ThemePalette,
    pub pill: ThemePill,
}

/// One status-bar pill (background + foreground).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pill {
    pub bg: Color,
    pub fg: Color,
}

/// Mode / diff / filter pills for the active theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pills {
    pub mode: Pill,
    pub diff: Pill,
    pub filter: Pill,
}

impl ThemeId {
    /// Env / config slug (`tokyo-night`).
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeId::TokyoNight => "tokyo-night",
            ThemeId::Monokai => "monokai",
            ThemeId::Dracula => "dracula",
            ThemeId::GruvboxDark => "gruvbox-dark",
            ThemeId::CatppuccinMocha => "catppuccin-mocha",
        }
    }

    /// Status-bar label (`Tokyo Night`).
    pub fn label(self) -> &'static str {
        self.theme().label
    }

    /// Status-bar pill colours.
    pub fn pills(self) -> Pills {
        let pill = self.theme().pill;
        Pills {
            mode: Pill {
                bg: hex_color(pill.mode_bg),
                fg: hex_color(pill.mode_fg),
            },
            diff: Pill {
                bg: hex_color(pill.diff_bg),
                fg: hex_color(pill.diff_fg),
            },
            filter: Pill {
                bg: hex_color(pill.filter_bg),
                fg: hex_color(pill.filter_fg),
            },
        }
    }

    /// Ratatui colours for paint.
    pub fn palette(self) -> Palette {
        let p = self.theme().palette;
        Palette {
            heading: hex_color(p.heading),
            repo: hex_color(p.repo),
            dir: hex_color(p.dir),
            file: hex_color(p.file),
            muted: hex_color(p.muted),
            added: hex_color(p.added),
            modified: hex_color(p.modified),
            deleted: hex_color(p.deleted),
            renamed: hex_color(p.renamed),
            branch_default: hex_color(p.branch_default),
            branch_feature: hex_color(p.branch_feature),
            head_mark: hex_color(p.head_mark),
            cursor: hex_color(p.cursor),
            cursor_bg: hex_color(p.cursor_bg),
            diff_hunk: hex_color(p.diff_hunk),
            flash: hex_color(p.flash),
        }
    }

    /// Full preset.
    pub fn theme(self) -> Theme {
        match self {
            ThemeId::TokyoNight => TOKYO_NIGHT,
            ThemeId::Monokai => MONOKAI,
            ThemeId::Dracula => DRACULA,
            ThemeId::GruvboxDark => GRUVBOX_DARK,
            ThemeId::CatppuccinMocha => CATPPUCCIN_MOCHA,
        }
    }
}

const TOKYO_NIGHT: Theme = Theme {
    id: ThemeId::TokyoNight,
    label: "Tokyo Night",
    surface: "#1a1b26",
    palette: ThemePalette {
        heading: "#7dcfff",
        repo: "#c0caf5",
        dir: "#7aa2f7",
        file: "#a9b1d6",
        muted: "#565f89",
        added: "#9ece6a",
        modified: "#e0af68",
        deleted: "#f7768e",
        renamed: "#7dcfff",
        branch_default: "#7aa2f7",
        branch_feature: "#bb9af7",
        head_mark: "#e0af68",
        cursor: "#7aa2f7",
        cursor_bg: "#283457",
        diff_hunk: "#7dcfff",
        flash: "#3d5236",
    },
    pill: ThemePill {
        mode_bg: "#3d59a1",
        mode_fg: "#c0caf5",
        diff_bg: "#33467c",
        diff_fg: "#c0caf5",
        filter_bg: "#bb9af7",
        filter_fg: "#1a1b26",
    },
};

const MONOKAI: Theme = Theme {
    id: ThemeId::Monokai,
    label: "Monokai",
    surface: "#272822",
    palette: ThemePalette {
        heading: "#66d9ef",
        repo: "#f8f8f2",
        dir: "#66d9ef",
        file: "#f8f8f2",
        muted: "#75715e",
        added: "#a6e22e",
        modified: "#e6db74",
        deleted: "#f92672",
        renamed: "#66d9ef",
        branch_default: "#66d9ef",
        branch_feature: "#ae81ff",
        head_mark: "#a6e22e",
        cursor: "#f8f8f2",
        cursor_bg: "#3e3d32",
        diff_hunk: "#66d9ef",
        flash: "#3e4a28",
    },
    pill: ThemePill {
        mode_bg: "#49483e",
        mode_fg: "#f8f8f2",
        diff_bg: "#3e3d32",
        diff_fg: "#f8f8f2",
        filter_bg: "#ae81ff",
        filter_fg: "#272822",
    },
};

const DRACULA: Theme = Theme {
    id: ThemeId::Dracula,
    label: "Dracula",
    surface: "#282a36",
    palette: ThemePalette {
        heading: "#8be9fd",
        repo: "#f8f8f2",
        dir: "#bd93f9",
        file: "#f8f8f2",
        muted: "#6272a4",
        added: "#50fa7b",
        modified: "#f1fa8c",
        deleted: "#ff5555",
        renamed: "#8be9fd",
        branch_default: "#bd93f9",
        branch_feature: "#bd93f9",
        head_mark: "#50fa7b",
        cursor: "#bd93f9",
        cursor_bg: "#44475a",
        diff_hunk: "#8be9fd",
        flash: "#2d4a3e",
    },
    pill: ThemePill {
        mode_bg: "#44475a",
        mode_fg: "#f8f8f2",
        diff_bg: "#6272a4",
        diff_fg: "#f8f8f2",
        filter_bg: "#bd93f9",
        filter_fg: "#282a36",
    },
};

const GRUVBOX_DARK: Theme = Theme {
    id: ThemeId::GruvboxDark,
    label: "Gruvbox Dark",
    surface: "#282828",
    palette: ThemePalette {
        heading: "#83a598",
        repo: "#ebdbb2",
        dir: "#458588",
        file: "#ebdbb2",
        muted: "#928374",
        added: "#b8bb26",
        modified: "#fabd2f",
        deleted: "#fb4934",
        renamed: "#83a598",
        branch_default: "#458588",
        branch_feature: "#d3869b",
        head_mark: "#fe8019",
        cursor: "#fe8019",
        cursor_bg: "#3c3836",
        diff_hunk: "#83a598",
        flash: "#32361a",
    },
    pill: ThemePill {
        mode_bg: "#504945",
        mode_fg: "#ebdbb2",
        diff_bg: "#3c3836",
        diff_fg: "#ebdbb2",
        filter_bg: "#d3869b",
        filter_fg: "#282828",
    },
};

const CATPPUCCIN_MOCHA: Theme = Theme {
    id: ThemeId::CatppuccinMocha,
    label: "Catppuccin Mocha",
    surface: "#1e1e2e",
    palette: ThemePalette {
        heading: "#89dceb",
        repo: "#cdd6f4",
        dir: "#89b4fa",
        file: "#cdd6f4",
        muted: "#6c7086",
        added: "#a6e3a1",
        modified: "#f9e2af",
        deleted: "#f38ba8",
        renamed: "#89dceb",
        branch_default: "#89b4fa",
        branch_feature: "#cba6f7",
        head_mark: "#f9e2af",
        cursor: "#89b4fa",
        cursor_bg: "#313244",
        diff_hunk: "#89dceb",
        flash: "#1e2b1e",
    },
    pill: ThemePill {
        mode_bg: "#45475a",
        mode_fg: "#cdd6f4",
        diff_bg: "#313244",
        diff_fg: "#cdd6f4",
        filter_bg: "#cba6f7",
        filter_fg: "#1e1e2e",
    },
};

/// Map an env / session string to a built-in theme id.
pub fn resolve_theme_id(raw: Option<&str>) -> ThemeId {
    match raw {
        Some("tokyo-night") => ThemeId::TokyoNight,
        Some("monokai") => ThemeId::Monokai,
        Some("dracula") => ThemeId::Dracula,
        Some("gruvbox-dark") => ThemeId::GruvboxDark,
        Some("catppuccin-mocha") => ThemeId::CatppuccinMocha,
        _ => DEFAULT_THEME_ID,
    }
}

/// Next theme id in `THEME_IDS` order (wraps).
pub fn cycle_theme_id(current: ThemeId) -> ThemeId {
    let index = THEME_IDS.iter().position(|id| *id == current).unwrap_or(0);
    THEME_IDS[(index + 1) % THEME_IDS.len()]
}

/// Launch seed from `WS_STATUS_THEME`. Unknown values fall back to Tokyo Night.
pub fn theme_from_env() -> ThemeId {
    resolve_theme_id(std::env::var("WS_STATUS_THEME").ok().as_deref())
}

/// Parse `#rrggbb` into a ratatui colour. Invalid hex falls back to white.
pub fn hex_color(hex: &str) -> Color {
    let bytes = hex.strip_prefix('#').unwrap_or(hex);
    if bytes.len() != 6 {
        return Color::White;
    }
    let Ok(r) = u8::from_str_radix(&bytes[0..2], 16) else {
        return Color::White;
    };
    let Ok(g) = u8::from_str_radix(&bytes[2..4], 16) else {
        return Color::White;
    };
    let Ok(b) = u8::from_str_radix(&bytes[4..6], 16) else {
        return Color::White;
    };
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_order_and_wrap() {
        assert_eq!(
            THEME_IDS,
            [
                ThemeId::TokyoNight,
                ThemeId::Monokai,
                ThemeId::Dracula,
                ThemeId::GruvboxDark,
                ThemeId::CatppuccinMocha,
            ]
        );
        assert_eq!(DEFAULT_THEME_ID, ThemeId::TokyoNight);
        let mut id = ThemeId::TokyoNight;
        let mut seen = Vec::new();
        for _ in 0..THEME_IDS.len() {
            id = cycle_theme_id(id);
            seen.push(id);
        }
        assert_eq!(
            seen,
            vec![
                ThemeId::Monokai,
                ThemeId::Dracula,
                ThemeId::GruvboxDark,
                ThemeId::CatppuccinMocha,
                ThemeId::TokyoNight,
            ]
        );
    }

    #[test]
    fn resolve_known_and_fallback() {
        assert_eq!(resolve_theme_id(None), ThemeId::TokyoNight);
        assert_eq!(resolve_theme_id(Some("")), ThemeId::TokyoNight);
        assert_eq!(resolve_theme_id(Some("nope")), ThemeId::TokyoNight);
        for id in THEME_IDS {
            assert_eq!(resolve_theme_id(Some(id.as_str())), id);
            assert!(!id.label().is_empty());
            assert!(id.theme().surface.starts_with('#'));
        }
    }

    #[test]
    fn hex_parses_tokyo_heading() {
        assert_eq!(hex_color("#7dcfff"), Color::Rgb(0x7d, 0xcf, 0xff));
        assert_eq!(hex_color("bad"), Color::White);
    }
}
