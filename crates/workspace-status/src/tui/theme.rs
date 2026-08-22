//! Built-in TUI colour themes. Same ids and cycle order as Ink `theme.ts`.
//!
//! Launch seed is `WS_STATUS_THEME`. `T` cycles in the current session only.
//! Ink does not write a theme file; this TUI matches that.

use ratatui::style::Color;

/// Built-in dark theme identifiers (Ink cycle order).
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
    pub cursor: &'static str,
    pub cursor_bg: &'static str,
    pub diff_hunk: &'static str,
    pub flash: &'static str,
}

/// One built-in theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub id: ThemeId,
    pub label: &'static str,
    pub surface: &'static str,
    pub palette: ThemePalette,
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
        cursor: "#7aa2f7",
        cursor_bg: "#283457",
        diff_hunk: "#7dcfff",
        flash: "#3d5236",
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
        cursor: "#f8f8f2",
        cursor_bg: "#3e3d32",
        diff_hunk: "#66d9ef",
        flash: "#3e4a28",
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
        cursor: "#bd93f9",
        cursor_bg: "#44475a",
        diff_hunk: "#8be9fd",
        flash: "#2d4a3e",
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
        cursor: "#fe8019",
        cursor_bg: "#3c3836",
        diff_hunk: "#83a598",
        flash: "#32361a",
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
        cursor: "#89b4fa",
        cursor_bg: "#313244",
        diff_hunk: "#89dceb",
        flash: "#1e2b1e",
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
