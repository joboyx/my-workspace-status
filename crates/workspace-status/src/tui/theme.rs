//! Built-in TUI colour themes.
//!
//! Palettes are dark. Secondary tokens (`muted`, line numbers, graph meta)
//! stay readable on the theme surface and on the `sidebar`, `chrome`, and
//! `panel` backgrounds. Launch seed is `WS_STATUS_THEME`, then the config
//! `theme` key, then [`DEFAULT_THEME_ID`] (Slate).
//! `T` cycles in the current session only. There is no theme file.

use ratatui::style::Color;

use super::watch::FlashKind;

/// Built-in dark theme identifiers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeId {
    TokyoNight,
    Monokai,
    Dracula,
    GruvboxDark,
    CatppuccinMocha,
    /// The launch default ([`DEFAULT_THEME_ID`]).
    #[default]
    Slate,
    SolarizedDark,
    Nord,
    RosePine,
    Kanagawa,
    Everforest,
    OneDark,
    GithubDarkDimmed,
}

/// Cycle order for `T`.
pub const THEME_IDS: [ThemeId; 13] = [
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

/// Default when neither `WS_STATUS_THEME` nor the config `theme` key sets a theme.
pub const DEFAULT_THEME_ID: ThemeId = ThemeId::Slate;

/// Default `sidebar` rule: the surface mixed this many percent toward black.
const SIDEBAR_TOWARD_BLACK_PERCENT: i32 = 22;

/// Default `chrome` rule: the surface mixed this many percent toward black.
const CHROME_TOWARD_BLACK_PERCENT: i32 = 38;

/// Default `panel` rule: the surface mixed this many percent toward white.
const PANEL_TOWARD_WHITE_PERCENT: i32 = 5;

/// Who paints the pane and chrome backgrounds (`viewDefaults.background`).
///
/// Popups sit on the theme panel colour in both modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackgroundMode {
    /// `"paint"`: the TUI fills panes and chrome rows with theme colours
    /// and draws flat panes with no border glyphs. The launch default.
    #[default]
    Paint,
    /// `"terminal"`: boxed panes with no fills, so the terminal background
    /// shows through.
    Terminal,
}

/// Ratatui colours for the active theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub heading: Color,
    pub repo: Color,
    pub dir: Color,
    pub file: Color,
    pub muted: Color,
    /// Unfocused pane border. Near-surface dark gray. Darker than [`Self::muted`].
    pub border_dim: Color,
    /// Idle compare-tab close `[✗]`. Neutral dark gray, darker than
    /// [`Self::muted`], still readable on the surface and on
    /// [`Self::cursor_bg`] (active tab).
    pub tab_close: Color,
    /// Hovered compare-tab close `[✗]`. Red, the theme's [`Self::deleted`].
    pub tab_close_hover: Color,
    pub added: Color,
    pub modified: Color,
    pub deleted: Color,
    pub renamed: Color,
    /// Reviewed eye (`ICON_VIEWED`). Teal/cyan on dark surfaces.
    pub viewed: Color,
    pub branch_default: Color,
    pub branch_feature: Color,
    pub head_mark: Color,
    pub cursor: Color,
    pub cursor_bg: Color,
    /// Selected-row background on an unfocused list. Darker than [`Self::cursor_bg`].
    pub cursor_bg_inactive: Color,
    /// Pane surface (background): the right pane and the file tab. The base
    /// that [`Self::cursor_tint`] measures the cursor shift from.
    pub surface: Color,
    /// Left pane background (tree, graph, files list). [`Theme::sidebar`],
    /// else the surface mixed 22% toward black.
    pub sidebar: Color,
    /// Tab strip, breadcrumb, ctrl-c prompt, and key-chip footer background.
    /// [`Theme::chrome`], else the surface mixed 38% toward black.
    pub chrome: Color,
    /// Popup background. [`Theme::panel`], else the surface mixed 5% toward
    /// white.
    pub panel: Color,
    pub diff_hunk: Color,
    /// Add-line row background. Syntax fg paints on top. The cursor,
    /// visual-line, and unfocused selected overlays tint it with
    /// [`Self::cursor_tint`]; the search overlay replaces it.
    pub diff_add_bg: Color,
    /// Del-line row background. Syntax fg paints on top. The cursor,
    /// visual-line, and unfocused selected overlays tint it with
    /// [`Self::cursor_tint`]; the search overlay replaces it.
    pub diff_del_bg: Color,
    /// Changed-word background on a paired add line. Stronger shade of
    /// [`Self::diff_add_bg`]. Syntax fg paints on top. The cursor,
    /// visual-line, and unfocused selected overlays tint it with
    /// [`Self::cursor_tint`]; the search overlay replaces it.
    pub diff_add_word_bg: Color,
    /// Changed-word background on a paired del line. Stronger shade of
    /// [`Self::diff_del_bg`]. Syntax fg paints on top. The cursor,
    /// visual-line, and unfocused selected overlays tint it with
    /// [`Self::cursor_tint`]; the search overlay replaces it.
    pub diff_del_word_bg: Color,
    /// Add-flash peak. Equals index 0 of [`Self::flash_ramp`].
    pub flash: Color,
    /// Four-step add fade. Index 0 is [`Self::flash`].
    pub flash_ramp: [Color; 4],
    /// Update-flash peak. Index 0 of [`Self::flash_update_ramp`].
    pub flash_update: Color,
    /// Four-step update fade.
    pub flash_update_ramp: [Color; 4],
    /// Remove-flash peak. Index 0 of [`Self::flash_remove_ramp`].
    pub flash_remove: Color,
    /// Four-step remove fade.
    pub flash_remove_ramp: [Color; 4],
}

/// Semantic colours used by the ratatui paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub heading: &'static str,
    pub repo: &'static str,
    pub dir: &'static str,
    pub file: &'static str,
    pub muted: &'static str,
    /// Unfocused pane border hex (`palette.borderDim` in docs). Near-surface
    /// dark gray. Darker than [`Self::muted`]. Not a readable-text token.
    pub border_dim: &'static str,
    /// Idle compare-tab close `[✗]` hex. Neutral dark gray, darker than
    /// [`Self::muted`].
    pub tab_close: &'static str,
    /// Hovered compare-tab close `[✗]` hex. Red, the theme's `deleted` hex.
    pub tab_close_hover: &'static str,
    pub added: &'static str,
    pub modified: &'static str,
    pub deleted: &'static str,
    pub renamed: &'static str,
    /// Reviewed eye hex. Teal/cyan on dark surfaces.
    pub viewed: &'static str,
    pub branch_default: &'static str,
    pub branch_feature: &'static str,
    pub head_mark: &'static str,
    pub cursor: &'static str,
    pub cursor_bg: &'static str,
    /// Unfocused selected-row background hex. Darker than [`Self::cursor_bg`].
    pub cursor_bg_inactive: &'static str,
    pub diff_hunk: &'static str,
    /// Add-line row background hex (`palette.diffAddBg` in docs).
    pub diff_add_bg: &'static str,
    /// Del-line row background hex (`palette.diffDelBg` in docs).
    pub diff_del_bg: &'static str,
    /// Changed-word background hex on a paired add line
    /// (`palette.diffAddWordBg` in docs). Stronger shade of [`Self::diff_add_bg`].
    pub diff_add_word_bg: &'static str,
    /// Changed-word background hex on a paired del line
    /// (`palette.diffDelWordBg` in docs). Stronger shade of [`Self::diff_del_bg`].
    pub diff_del_word_bg: &'static str,
    /// Add-flash peak hex. Index 0 of [`Self::flash_ramp`].
    pub flash: &'static str,
    /// Four-step add fade hex. Index 0 matches [`Self::flash`].
    pub flash_ramp: [&'static str; 4],
    /// Update-flash peak hex.
    pub flash_update: &'static str,
    /// Four-step update fade hex.
    pub flash_update_ramp: [&'static str; 4],
    /// Remove-flash peak hex.
    pub flash_remove: &'static str,
    /// Four-step remove fade hex.
    pub flash_remove_ramp: [&'static str; 4],
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
    /// Left pane background hex. `None` uses the default rule: the surface
    /// mixed 22% toward black.
    pub sidebar: Option<&'static str>,
    /// Tab strip, breadcrumb, prompt, and footer row background hex. `None`
    /// uses the default rule: the surface mixed 38% toward black.
    pub chrome: Option<&'static str>,
    /// Popup background hex. `None` uses the default rule: the surface mixed
    /// 5% toward white.
    pub panel: Option<&'static str>,
    pub palette: ThemePalette,
    pub pill: ThemePill,
    /// Graph gutter cycle. Tokyo Night matches [`workspace_status_graph::DEFAULT_LANE_COLORS`].
    pub lane_colors: [&'static str; 8],
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
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeId::TokyoNight => "tokyo-night",
            ThemeId::Monokai => "monokai",
            ThemeId::Dracula => "dracula",
            ThemeId::GruvboxDark => "gruvbox-dark",
            ThemeId::CatppuccinMocha => "catppuccin-mocha",
            ThemeId::Slate => "slate",
            ThemeId::SolarizedDark => "solarized-dark",
            ThemeId::Nord => "nord",
            ThemeId::RosePine => "rose-pine",
            ThemeId::Kanagawa => "kanagawa",
            ThemeId::Everforest => "everforest",
            ThemeId::OneDark => "one-dark",
            ThemeId::GithubDarkDimmed => "github-dark-dimmed",
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
        let theme = self.theme();
        let p = theme.palette;
        let surface = hex_color(theme.surface);
        let role = |hex: Option<&str>, target: Color, percent: i32| {
            hex.map_or_else(|| mix_toward(surface, target, percent), hex_color)
        };
        Palette {
            heading: hex_color(p.heading),
            repo: hex_color(p.repo),
            dir: hex_color(p.dir),
            file: hex_color(p.file),
            muted: hex_color(p.muted),
            border_dim: hex_color(p.border_dim),
            tab_close: hex_color(p.tab_close),
            tab_close_hover: hex_color(p.tab_close_hover),
            added: hex_color(p.added),
            modified: hex_color(p.modified),
            deleted: hex_color(p.deleted),
            renamed: hex_color(p.renamed),
            viewed: hex_color(p.viewed),
            branch_default: hex_color(p.branch_default),
            branch_feature: hex_color(p.branch_feature),
            head_mark: hex_color(p.head_mark),
            cursor: hex_color(p.cursor),
            cursor_bg: hex_color(p.cursor_bg),
            cursor_bg_inactive: hex_color(p.cursor_bg_inactive),
            surface,
            sidebar: role(
                theme.sidebar,
                Color::Rgb(0, 0, 0),
                SIDEBAR_TOWARD_BLACK_PERCENT,
            ),
            chrome: role(
                theme.chrome,
                Color::Rgb(0, 0, 0),
                CHROME_TOWARD_BLACK_PERCENT,
            ),
            panel: role(
                theme.panel,
                Color::Rgb(255, 255, 255),
                PANEL_TOWARD_WHITE_PERCENT,
            ),
            diff_hunk: hex_color(p.diff_hunk),
            diff_add_bg: hex_color(p.diff_add_bg),
            diff_del_bg: hex_color(p.diff_del_bg),
            diff_add_word_bg: hex_color(p.diff_add_word_bg),
            diff_del_word_bg: hex_color(p.diff_del_word_bg),
            flash: hex_color(p.flash),
            flash_ramp: p.flash_ramp.map(hex_color),
            flash_update: hex_color(p.flash_update),
            flash_update_ramp: p.flash_update_ramp.map(hex_color),
            flash_remove: hex_color(p.flash_remove),
            flash_remove_ramp: p.flash_remove_ramp.map(hex_color),
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
            ThemeId::Slate => SLATE,
            ThemeId::SolarizedDark => SOLARIZED_DARK,
            ThemeId::Nord => NORD,
            ThemeId::RosePine => ROSE_PINE,
            ThemeId::Kanagawa => KANAGAWA,
            ThemeId::Everforest => EVERFOREST,
            ThemeId::OneDark => ONE_DARK,
            ThemeId::GithubDarkDimmed => GITHUB_DARK_DIMMED,
        }
    }

    /// Graph gutter colours for `T`.
    pub fn lane_colors(self) -> [Color; 8] {
        self.theme().lane_colors.map(hex_color)
    }
}

/// Tokyo Night TTY e2e hardcodes these peak RGB values:
/// add `#516643` = `Rgb(81, 102, 67)`,
/// update `#6d5942` = `Rgb(109, 89, 66)`,
/// remove `#774152` = `Rgb(119, 65, 82)`.
/// Diff add/del row backgrounds: add `#3f4d39` = `Rgb(63, 77, 57)`,
/// del `#583443` = `Rgb(88, 52, 67)`.
/// Changed-word backgrounds: add `#426832` = `Rgb(66, 104, 50)`,
/// del `#813d59` = `Rgb(129, 61, 89)`.
const TOKYO_NIGHT: Theme = Theme {
    id: ThemeId::TokyoNight,
    label: "Tokyo Night",
    surface: "#1a1b26",
    sidebar: None,
    chrome: None,
    panel: None,
    palette: ThemePalette {
        heading: "#7dcfff",
        repo: "#c0caf5",
        dir: "#7aa2f7",
        file: "#a9b1d6",
        muted: "#9aa5ce",
        border_dim: "#3b4261",
        tab_close: "#787878",
        tab_close_hover: "#f7768e",
        added: "#9ece6a",
        modified: "#e0af68",
        deleted: "#f7768e",
        renamed: "#7dcfff",
        viewed: "#73daca",
        branch_default: "#7aa2f7",
        branch_feature: "#bb9af7",
        head_mark: "#e0af68",
        cursor: "#7aa2f7",
        cursor_bg: "#283457",
        cursor_bg_inactive: "#21273e",
        diff_hunk: "#7dcfff",
        diff_add_bg: "#3f4d39",
        diff_del_bg: "#583443",
        diff_add_word_bg: "#426832",
        diff_del_word_bg: "#813d59",
        flash: "#516643",
        flash_ramp: ["#516643", "#3f4d39", "#2f3831", "#25292b"],
        flash_update: "#6d5942",
        flash_update_ramp: ["#6d5942", "#514438", "#3a3331", "#2a272b"],
        flash_remove: "#774152",
        flash_remove_ramp: ["#774152", "#583443", "#3d2a37", "#2c222e"],
    },
    pill: ThemePill {
        mode_bg: "#3d59a1",
        mode_fg: "#c0caf5",
        diff_bg: "#33467c",
        diff_fg: "#c0caf5",
        filter_bg: "#bb9af7",
        filter_fg: "#1a1b26",
    },
    lane_colors: [
        "#7aa2f7", "#bb9af7", "#7dcfff", "#9ece6a", "#e0af68", "#f7768e", "#ff9e64", "#73daca",
    ],
};

const MONOKAI: Theme = Theme {
    id: ThemeId::Monokai,
    label: "Monokai",
    surface: "#272822",
    sidebar: None,
    chrome: None,
    // The 5% default panel drops popup text below its contrast floor
    // (`popover_and_legend_text_meets_floors_on_surface_and_focus`), so
    // this panel is the largest lift that passes: 4% toward white.
    panel: Some("#30312b"),
    palette: ThemePalette {
        heading: "#66d9ef",
        repo: "#f8f8f2",
        dir: "#66d9ef",
        file: "#f8f8f2",
        muted: "#b8b39c",
        border_dim: "#49483e",
        tab_close: "#808080",
        tab_close_hover: "#ff6188",
        added: "#a6e22e",
        modified: "#e6db74",
        deleted: "#ff6188",
        renamed: "#66d9ef",
        viewed: "#a1efe4",
        branch_default: "#66d9ef",
        branch_feature: "#ae81ff",
        head_mark: "#a6e22e",
        cursor: "#f8f8f2",
        cursor_bg: "#3e3d32",
        cursor_bg_inactive: "#32322a",
        diff_hunk: "#66d9ef",
        diff_add_bg: "#4b5c25",
        diff_del_bg: "#63383f",
        diff_add_word_bg: "#59751b",
        diff_del_word_bg: "#8f404d",
        flash: "#5c7627",
        flash_ramp: ["#5c7627", "#4b5c25", "#3b4624", "#313723"],
        flash_update: "#777344",
        flash_update_ramp: ["#777344", "#5c5a39", "#46452f", "#363629"],
        flash_remove: "#82404d",
        flash_remove_ramp: ["#82404d", "#63383f", "#4a3132", "#382d2a"],
    },
    pill: ThemePill {
        mode_bg: "#49483e",
        mode_fg: "#f8f8f2",
        diff_bg: "#3e3d32",
        diff_fg: "#f8f8f2",
        filter_bg: "#ae81ff",
        filter_fg: "#272822",
    },
    lane_colors: [
        "#66d9ef", "#ae81ff", "#a6e22e", "#e6db74", "#f92672", "#fd971f", "#f8f8f2", "#a1efe4",
    ],
};

const DRACULA: Theme = Theme {
    id: ThemeId::Dracula,
    label: "Dracula",
    surface: "#282a36",
    sidebar: None,
    chrome: None,
    // The 5% default panel drops popup text below its contrast floor
    // (`popover_and_legend_text_meets_floors_on_surface_and_focus`), so
    // this panel is the largest lift that passes: 3% toward white.
    panel: Some("#2e303c"),
    palette: ThemePalette {
        heading: "#8be9fd",
        repo: "#f8f8f2",
        dir: "#bd93f9",
        file: "#f8f8f2",
        muted: "#b4bce4",
        border_dim: "#44475a",
        tab_close: "#888888",
        tab_close_hover: "#ff5555",
        added: "#50fa7b",
        modified: "#f1fa8c",
        deleted: "#ff5555",
        renamed: "#8be9fd",
        viewed: "#aef2ff",
        branch_default: "#bd93f9",
        branch_feature: "#bd93f9",
        head_mark: "#50fa7b",
        cursor: "#bd93f9",
        cursor_bg: "#44475a",
        cursor_bg_inactive: "#363848",
        diff_hunk: "#8be9fd",
        diff_add_bg: "#336449",
        diff_del_bg: "#64363f",
        diff_add_word_bg: "#287f4f",
        diff_del_word_bg: "#8f3e4e",
        flash: "#398153",
        flash_ramp: ["#398153", "#336449", "#2e4b41", "#2b3b3c"],
        flash_update: "#7c815a",
        flash_update_ramp: ["#7c815a", "#60644e", "#484b44", "#383b3d"],
        flash_remove: "#823c43",
        flash_remove_ramp: ["#823c43", "#64363f", "#4a313b", "#392d38"],
    },
    pill: ThemePill {
        mode_bg: "#44475a",
        mode_fg: "#f8f8f2",
        diff_bg: "#6272a4",
        diff_fg: "#f8f8f2",
        filter_bg: "#bd93f9",
        filter_fg: "#282a36",
    },
    lane_colors: [
        "#8be9fd", "#bd93f9", "#50fa7b", "#f1fa8c", "#ff5555", "#ffb86c", "#ff79c6", "#6272a4",
    ],
};

const GRUVBOX_DARK: Theme = Theme {
    id: ThemeId::GruvboxDark,
    label: "Gruvbox Dark",
    surface: "#282828",
    sidebar: None,
    chrome: None,
    // The 5% default panel drops popup text below its contrast floor
    // (`popover_and_legend_text_meets_floors_on_surface_and_focus`), so
    // this panel is the largest lift that passes: 2% toward white.
    panel: Some("#2c2c2c"),
    palette: ThemePalette {
        heading: "#83a598",
        repo: "#ebdbb2",
        dir: "#8ec07c",
        file: "#ebdbb2",
        muted: "#bdae93",
        border_dim: "#504945",
        tab_close: "#787878",
        tab_close_hover: "#fb4934",
        added: "#b8bb26",
        modified: "#fabd2f",
        deleted: "#fb4934",
        renamed: "#83a598",
        viewed: "#aedc9a",
        branch_default: "#83a598",
        branch_feature: "#d3869b",
        head_mark: "#fe8019",
        cursor: "#fe8019",
        cursor_bg: "#3c3836",
        cursor_bg_inactive: "#32302f",
        diff_hunk: "#83a598",
        diff_add_bg: "#505127",
        diff_del_bg: "#63312b",
        diff_add_word_bg: "#66671e",
        diff_del_word_bg: "#963226",
        flash: "#646627",
        flash_ramp: ["#646627", "#505127", "#3f4028", "#343428"],
        flash_update: "#80672b",
        flash_update_ramp: ["#80672b", "#63522a", "#4a4029", "#393429"],
        flash_remove: "#81362d",
        flash_remove_ramp: ["#81362d", "#63312b", "#4a2d2a", "#392b29"],
    },
    pill: ThemePill {
        mode_bg: "#504945",
        mode_fg: "#ebdbb2",
        diff_bg: "#3c3836",
        diff_fg: "#ebdbb2",
        filter_bg: "#d3869b",
        filter_fg: "#282828",
    },
    lane_colors: [
        "#83a598", "#d3869b", "#b8bb26", "#fabd2f", "#fb4934", "#fe8019", "#8ec07c", "#d79921",
    ],
};

const CATPPUCCIN_MOCHA: Theme = Theme {
    id: ThemeId::CatppuccinMocha,
    label: "Catppuccin Mocha",
    surface: "#1e1e2e",
    sidebar: None,
    chrome: None,
    panel: None,
    palette: ThemePalette {
        heading: "#89dceb",
        repo: "#cdd6f4",
        dir: "#89b4fa",
        file: "#cdd6f4",
        muted: "#a6adc8",
        border_dim: "#45475a",
        tab_close: "#707070",
        tab_close_hover: "#f38ba8",
        added: "#a6e3a1",
        modified: "#f9e2af",
        deleted: "#f38ba8",
        renamed: "#89dceb",
        viewed: "#94e2d5",
        branch_default: "#89b4fa",
        branch_feature: "#cba6f7",
        head_mark: "#f9e2af",
        cursor: "#89b4fa",
        cursor_bg: "#313244",
        cursor_bg_inactive: "#272839",
        diff_hunk: "#89dceb",
        diff_add_bg: "#44554e",
        diff_del_bg: "#5a3d50",
        diff_add_word_bg: "#3b715b",
        diff_del_word_bg: "#82476d",
        flash: "#57715e",
        flash_ramp: ["#57715e", "#44554e", "#343e40", "#292e37"],
        flash_update: "#7a7064",
        flash_update_ramp: ["#7a7064", "#5b5552", "#413d43", "#302e38"],
        flash_remove: "#774c61",
        flash_remove_ramp: ["#774c61", "#5a3d50", "#402f42", "#2f2738"],
    },
    pill: ThemePill {
        mode_bg: "#45475a",
        mode_fg: "#cdd6f4",
        diff_bg: "#313244",
        diff_fg: "#cdd6f4",
        filter_bg: "#cba6f7",
        filter_fg: "#1e1e2e",
    },
    lane_colors: [
        "#89b4fa", "#cba6f7", "#a6e3a1", "#f9e2af", "#f38ba8", "#fab387", "#89dceb", "#f5c2e7",
    ],
};

// The eight themes below take their base colours from one table (surface,
// fg, accents, pill). Roles derive from it: `cursor_bg` / `cursor_bg_inactive`
// mix the surface 16% / 8% toward blue, diff rows 20% and changed words 38%
// toward green / red, the diff pill 35% toward blue, and the flash ramps
// 42 / 28 / 16 / 8% toward the added / modified / deleted colour (the rule
// the five themes above follow). Where a value failed a contrast or
// distinctness test below, it was lifted toward white, re-mixed, or hue-shifted
// by the smallest step that passes.
const SLATE: Theme = Theme {
    id: ThemeId::Slate,
    label: "Slate",
    surface: "#11151b",
    sidebar: Some("#151a21"),
    chrome: Some("#0d1116"),
    panel: Some("#19202a"),
    palette: ThemePalette {
        heading: "#7fc4d6",
        repo: "#e1e6ee",
        dir: "#79a6dc",
        file: "#c3cad5",
        muted: "#8a94a3",
        border_dim: "#2b3442",
        tab_close: "#686868",
        tab_close_hover: "#e27a7a",
        added: "#78c48d",
        modified: "#e0b35c",
        deleted: "#e27a7a",
        renamed: "#7fc4d6",
        viewed: "#69c3c0",
        branch_default: "#79a6dc",
        branch_feature: "#b39ddb",
        head_mark: "#e0b35c",
        cursor: "#79a6dc",
        cursor_bg: "#222c3a",
        cursor_bg_inactive: "#19212a",
        diff_hunk: "#7fc4d6",
        diff_add_bg: "#263832",
        diff_del_bg: "#3b292e",
        diff_add_word_bg: "#385846",
        diff_del_word_bg: "#603b3f",
        flash: "#3c5f4b",
        flash_ramp: ["#3c5f4b", "#2e463b", "#21312d", "#192324"],
        flash_update: "#685736",
        flash_update_ramp: ["#685736", "#4b412d", "#322e25", "#222220"],
        flash_remove: "#693f43",
        flash_remove_ramp: ["#693f43", "#4c3136", "#32252a", "#221d23"],
    },
    pill: ThemePill {
        mode_bg: "#2d4566",
        mode_fg: "#e1e6ee",
        diff_bg: "#35485f",
        diff_fg: "#e1e6ee",
        filter_bg: "#b39ddb",
        filter_fg: "#11151b",
    },
    lane_colors: [
        "#79a6dc", "#b39ddb", "#7fc4d6", "#78c48d", "#e0b35c", "#e27a7a", "#e8a06a", "#69c3c0",
    ],
};

const SOLARIZED_DARK: Theme = Theme {
    id: ThemeId::SolarizedDark,
    label: "Solarized Dark",
    surface: "#002b36",
    sidebar: None,
    chrome: None,
    panel: Some("#073642"),
    palette: ThemePalette {
        heading: "#39a89f",
        repo: "#eee8d5",
        dir: "#4b9fda",
        file: "#93a1a1",
        muted: "#92a1a3",
        border_dim: "#1a4957",
        tab_close: "#727272",
        tab_close_hover: "#e56967",
        added: "#8fa114",
        modified: "#bc9417",
        deleted: "#e56967",
        renamed: "#39a89f",
        viewed: "#4cb087",
        branch_default: "#4b9fda",
        branch_feature: "#8f93d2",
        head_mark: "#bc9417",
        cursor: "#4b9fda",
        cursor_bg: "#063a4f",
        cursor_bg_inactive: "#033342",
        diff_hunk: "#39a89f",
        diff_add_bg: "#2d5024",
        diff_del_bg: "#422d34",
        diff_add_word_bg: "#40601c",
        diff_del_word_bg: "#6e2f32",
        flash: "#3c5d28",
        flash_ramp: ["#3c5d28", "#284c2c", "#173e31", "#0b3433"],
        flash_update: "#4f5729",
        flash_update_ramp: ["#4f5729", "#35482d", "#1e3c31", "#0f3334"],
        flash_remove: "#60454b",
        flash_remove_ramp: ["#60454b", "#403c44", "#25353e", "#12303a"],
    },
    pill: ThemePill {
        mode_bg: "#0e5a74",
        mode_fg: "#eee8d5",
        diff_bg: "#0d4d6d",
        diff_fg: "#eee8d5",
        filter_bg: "#8f93d2",
        filter_fg: "#002b36",
    },
    lane_colors: [
        "#268bd2", "#6c71c4", "#2aa198", "#859900", "#b58900", "#dc322f", "#cb4b16", "#4cb087",
    ],
};

const NORD: Theme = Theme {
    id: ThemeId::Nord,
    label: "Nord",
    surface: "#2e3440",
    sidebar: None,
    chrome: None,
    panel: Some("#3b4252"),
    palette: ThemePalette {
        heading: "#88c0d0",
        repo: "#eceff4",
        dir: "#96b1cc",
        file: "#d8dee9",
        muted: "#afb8c7",
        border_dim: "#434c5e",
        tab_close: "#828282",
        tab_close_hover: "#d3949a",
        added: "#a4bf8d",
        modified: "#ebcb8b",
        deleted: "#d3949a",
        renamed: "#88c0d0",
        viewed: "#8fbcbb",
        branch_default: "#96b1cc",
        branch_feature: "#c3a5bd",
        head_mark: "#ebcb8b",
        cursor: "#96b1cc",
        cursor_bg: "#3b4555",
        cursor_bg_inactive: "#353d4a",
        diff_hunk: "#88c0d0",
        diff_add_bg: "#58665b",
        diff_del_bg: "#54404b",
        diff_add_word_bg: "#677865",
        diff_del_word_bg: "#744a54",
        flash: "#606e60",
        flash_ramp: ["#606e60", "#4f5b56", "#414a4c", "#373f46"],
        flash_update: "#7d7360",
        flash_update_ramp: ["#7d7360", "#635e55", "#4c4c4c", "#3d4046"],
        flash_remove: "#735c66",
        flash_remove_ramp: ["#735c66", "#5c4f59", "#48434e", "#3b3c47"],
    },
    pill: ThemePill {
        mode_bg: "#5e81ac",
        mode_fg: "#eceff4",
        diff_bg: "#4b5a6d",
        diff_fg: "#eceff4",
        filter_bg: "#c3a5bd",
        filter_fg: "#2e3440",
    },
    lane_colors: [
        "#81a1c1", "#b48ead", "#88c0d0", "#a3be8c", "#ebcb8b", "#bf616a", "#d08770", "#8fbcbb",
    ],
};

const ROSE_PINE: Theme = Theme {
    id: ThemeId::RosePine,
    label: "Rosé Pine",
    surface: "#191724",
    sidebar: Some("#1f1d2e"),
    chrome: Some("#131220"),
    panel: Some("#26233a"),
    palette: ThemePalette {
        heading: "#ebbcba",
        repo: "#f2f0ff",
        dir: "#9ccfd8",
        file: "#e0def4",
        muted: "#9c99b3",
        border_dim: "#403d52",
        tab_close: "#707070",
        tab_close_hover: "#eb6f92",
        added: "#9ccfd8",
        modified: "#f6c177",
        deleted: "#eb6f92",
        renamed: "#ebbcba",
        viewed: "#9cd8cd",
        branch_default: "#9ccfd8",
        branch_feature: "#c4a7e7",
        head_mark: "#f6c177",
        cursor: "#9ccfd8",
        cursor_bg: "#2e3441",
        cursor_bg_inactive: "#232632",
        diff_hunk: "#ebbcba",
        diff_add_bg: "#333c48",
        diff_del_bg: "#43293a",
        diff_add_word_bg: "#495d68",
        diff_del_word_bg: "#69384e",
        flash: "#506470",
        flash_ramp: ["#506470", "#3e4b56", "#2e3441", "#232632"],
        flash_update: "#765e47",
        flash_update_ramp: ["#765e47", "#57473b", "#3c3231", "#2b252b"],
        flash_remove: "#713c52",
        flash_remove_ramp: ["#713c52", "#543043", "#3b2536", "#2a1e2d"],
    },
    pill: ThemePill {
        mode_bg: "#31748f",
        mode_fg: "#f2f0ff",
        diff_bg: "#475763",
        diff_fg: "#f2f0ff",
        filter_bg: "#c4a7e7",
        filter_fg: "#191724",
    },
    lane_colors: [
        "#9ccfd8", "#c4a7e7", "#ebbcba", "#9cd8b9", "#f6c177", "#eb6f92", "#ebccba", "#9cd8cd",
    ],
};

const KANAGAWA: Theme = Theme {
    id: ThemeId::Kanagawa,
    label: "Kanagawa",
    surface: "#1f1f28",
    sidebar: Some("#16161d"),
    chrome: Some("#121218"),
    panel: Some("#2a2a37"),
    palette: ThemePalette {
        heading: "#7fb4ca",
        repo: "#ece6c8",
        dir: "#7e9cd8",
        file: "#dcd7ba",
        muted: "#a6a08a",
        border_dim: "#363646",
        tab_close: "#717171",
        tab_close_hover: "#e46876",
        added: "#98bb6c",
        modified: "#e6c384",
        deleted: "#e46876",
        renamed: "#7fb4ca",
        viewed: "#7aa89f",
        branch_default: "#7e9cd8",
        branch_feature: "#9d89be",
        head_mark: "#e6c384",
        cursor: "#7e9cd8",
        cursor_bg: "#2e3344",
        cursor_bg_inactive: "#272936",
        diff_hunk: "#7fb4ca",
        diff_add_bg: "#373e36",
        diff_del_bg: "#462e38",
        diff_add_word_bg: "#4d5a42",
        diff_del_word_bg: "#6a3b46",
        flash: "#526145",
        flash_ramp: ["#526145", "#414b3b", "#323833", "#292b2d"],
        flash_update: "#73644f",
        flash_update_ramp: ["#73644f", "#574d42", "#3f3937", "#2f2c2f"],
        flash_remove: "#723e49",
        flash_remove_ramp: ["#723e49", "#56333e", "#3f2b34", "#2f252e"],
    },
    pill: ThemePill {
        mode_bg: "#2d4f67",
        mode_fg: "#ece6c8",
        diff_bg: "#404b66",
        diff_fg: "#ece6c8",
        filter_bg: "#9d89be",
        filter_fg: "#1f1f28",
    },
    lane_colors: [
        "#7e9cd8", "#957fb8", "#7fb4ca", "#98bb6c", "#e6c384", "#e46876", "#ffa066", "#7aa89f",
    ],
};

const EVERFOREST: Theme = Theme {
    id: ThemeId::Everforest,
    label: "Everforest",
    surface: "#2d353b",
    sidebar: Some("#232a2e"),
    chrome: None,
    panel: Some("#343f44"),
    palette: ThemePalette {
        heading: "#83c092",
        repo: "#e4dac0",
        dir: "#7fbbb3",
        file: "#d3c6aa",
        muted: "#aeb8b0",
        border_dim: "#475258",
        tab_close: "#858585",
        tab_close_hover: "#e78183",
        added: "#a7c080",
        modified: "#dbbc7f",
        deleted: "#e78183",
        renamed: "#83c092",
        viewed: "#8dc388",
        branch_default: "#7fbbb3",
        branch_feature: "#d699b6",
        head_mark: "#dbbc7f",
        cursor: "#7fbbb3",
        cursor_bg: "#3a4a4e",
        cursor_bg_inactive: "#344045",
        diff_hunk: "#83c092",
        diff_add_bg: "#4f5c4e",
        diff_del_bg: "#524449",
        diff_add_word_bg: "#606f58",
        diff_del_word_bg: "#735155",
        flash: "#606f58",
        flash_ramp: ["#606f58", "#4f5c4e", "#414b46", "#374041"],
        flash_update: "#766e58",
        flash_update_ramp: ["#766e58", "#5e5b4e", "#494b46", "#3b4040"],
        flash_remove: "#7b5559",
        flash_remove_ramp: ["#7b5559", "#614a4f", "#4b4147", "#3c3b41"],
    },
    pill: ThemePill {
        mode_bg: "#3a515d",
        mode_fg: "#e4dac0",
        diff_bg: "#4a6465",
        diff_fg: "#e4dac0",
        filter_bg: "#d699b6",
        filter_fg: "#2d353b",
    },
    lane_colors: [
        "#7fbbb3", "#d699b6", "#83c092", "#a7c080", "#dbbc7f", "#e67e80", "#e69875", "#8dc388",
    ],
};

const ONE_DARK: Theme = Theme {
    id: ThemeId::OneDark,
    label: "One Dark",
    surface: "#282c34",
    sidebar: Some("#21252b"),
    chrome: Some("#1b1e23"),
    panel: Some("#2c313c"),
    palette: ThemePalette {
        heading: "#56b6c2",
        repo: "#d7dae0",
        dir: "#61afef",
        file: "#abb2bf",
        muted: "#a6abb4",
        border_dim: "#3e4451",
        tab_close: "#7c7c7c",
        tab_close_hover: "#e06c75",
        added: "#98c379",
        modified: "#e5c07b",
        deleted: "#e06c75",
        renamed: "#56b6c2",
        viewed: "#56c2aa",
        branch_default: "#61afef",
        branch_feature: "#c77bde",
        head_mark: "#e5c07b",
        cursor: "#61afef",
        cursor_bg: "#314152",
        cursor_bg_inactive: "#2d3643",
        diff_hunk: "#56b6c2",
        diff_add_bg: "#455346",
        diff_del_bg: "#4d3941",
        diff_add_word_bg: "#576b51",
        diff_del_word_bg: "#6e444d",
        flash: "#576b51",
        flash_ramp: ["#576b51", "#475647", "#3a443f", "#31383a"],
        flash_update: "#776a52",
        flash_update_ramp: ["#776a52", "#5d5548", "#46443f", "#37383a"],
        flash_remove: "#75474f",
        flash_remove_ramp: ["#75474f", "#5c3e46", "#45363e", "#373139"],
    },
    pill: ThemePill {
        mode_bg: "#3b5070",
        mode_fg: "#d7dae0",
        diff_bg: "#3c5a75",
        diff_fg: "#d7dae0",
        filter_bg: "#c77bde",
        filter_fg: "#282c34",
    },
    lane_colors: [
        "#61afef", "#c678dd", "#56b6c2", "#98c379", "#e5c07b", "#e06c75", "#d19a66", "#56c2aa",
    ],
};

const GITHUB_DARK_DIMMED: Theme = Theme {
    id: ThemeId::GithubDarkDimmed,
    label: "GitHub Dark Dimmed",
    surface: "#22272e",
    sidebar: Some("#1c2128"),
    chrome: Some("#171b21"),
    panel: Some("#2d333b"),
    palette: ThemePalette {
        heading: "#96d0ff",
        repo: "#cdd9e5",
        dir: "#6cb6ff",
        file: "#adbac7",
        muted: "#9ea8b3",
        border_dim: "#444c56",
        tab_close: "#797979",
        tab_close_hover: "#f47067",
        added: "#6bc46d",
        modified: "#daaa3f",
        deleted: "#f47067",
        renamed: "#96d0ff",
        viewed: "#56d4dd",
        branch_default: "#6cb6ff",
        branch_feature: "#dcbdfb",
        head_mark: "#daaa3f",
        cursor: "#6cb6ff",
        cursor_bg: "#2e3e4f",
        cursor_bg_inactive: "#28323f",
        diff_hunk: "#96d0ff",
        diff_add_bg: "#31463b",
        diff_del_bg: "#4c3639",
        diff_add_word_bg: "#3e6346",
        diff_del_word_bg: "#724344",
        flash: "#416948",
        flash_ramp: ["#416948", "#365340", "#2e4038", "#283433"],
        flash_update: "#6f5e35",
        flash_update_ramp: ["#6f5e35", "#564c33", "#3f3c31", "#31312f"],
        flash_remove: "#7a4646",
        flash_remove_ramp: ["#7a4646", "#5d3b3e", "#443337", "#332d33"],
    },
    pill: ThemePill {
        mode_bg: "#255ab2",
        mode_fg: "#cdd9e5",
        diff_bg: "#3c5977",
        diff_fg: "#cdd9e5",
        filter_bg: "#dcbdfb",
        filter_fg: "#22272e",
    },
    lane_colors: [
        "#6cb6ff", "#dcbdfb", "#96d0ff", "#6bc46d", "#daaa3f", "#f47067", "#f69d50", "#56d4dd",
    ],
};

/// Built-in theme id for an exact slug (`tokyo-night`). `None` when unknown.
pub fn parse_theme_id(raw: &str) -> Option<ThemeId> {
    THEME_IDS.into_iter().find(|id| id.as_str() == raw)
}

/// Launch theme: a known `WS_STATUS_THEME` slug wins, then the config
/// `theme` key, then [`DEFAULT_THEME_ID`]. An unset or unknown env value
/// falls through to the config value.
pub fn theme_id_from(env_value: Option<&str>, config: Option<ThemeId>) -> ThemeId {
    env_value
        .and_then(parse_theme_id)
        .or(config)
        .unwrap_or(DEFAULT_THEME_ID)
}

/// Next theme id in `THEME_IDS` order (wraps).
pub fn cycle_theme_id(current: ThemeId) -> ThemeId {
    let index = THEME_IDS.iter().position(|id| *id == current).unwrap_or(0);
    THEME_IDS[(index + 1) % THEME_IDS.len()]
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

/// Share of the theme's cursor shift (`cursor - surface`) that
/// [`Palette::cursor_tint`] adds, in percent. Full strength drops default
/// text ([`Palette::repo`]) below 3:1 on the tinted Tokyo Night add word
/// background; 3/4 keeps it at 3:1 or more on every shipped theme while the
/// shift stays visible.
const CURSOR_TINT_PERCENT: i32 = 75;

/// `num / den` rounded to the nearest integer (halves away from zero).
fn div_round(num: i32, den: i32) -> i32 {
    if num >= 0 {
        (num + den / 2) / den
    } else {
        (num - den / 2) / den
    }
}

/// Mix `base` toward `target` by `percent` per RGB channel:
/// `base + (target - base) * percent / 100`, rounded to nearest. Non-RGB
/// input returns `base` unchanged.
fn mix_toward(base: Color, target: Color, percent: i32) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) = (base, target) else {
        return base;
    };
    let channel = |from: u8, to: u8| -> u8 {
        let shift = div_round((i32::from(to) - i32::from(from)) * percent, 100);
        (i32::from(from) + shift).clamp(0, 255) as u8
    };
    Color::Rgb(channel(r, tr), channel(g, tg), channel(b, tb))
}

impl Palette {
    /// Tint `bg` with a cursor overlay instead of replacing it.
    ///
    /// Adds 3/4 of the theme's own cursor shift (`cursor - surface`) to
    /// each RGB channel of `bg`, clamped to `0..=255`. `cursor` is [`Self::cursor_bg`] or
    /// [`Self::cursor_bg_inactive`]. The shift follows the theme: a cursor bar
    /// darker than the surface darkens `bg`. The same shift lands on a row bg
    /// and its word bg, so the word highlight keeps its contrast against the
    /// row. A non-RGB `bg`, `cursor`, or surface returns `cursor` unchanged.
    pub fn cursor_tint(self, bg: Color, cursor: Color) -> Color {
        let (Color::Rgb(r, g, b), Color::Rgb(cr, cg, cb), Color::Rgb(sr, sg, sb)) =
            (bg, cursor, self.surface)
        else {
            return cursor;
        };
        let channel = |base: u8, cur: u8, surf: u8| -> u8 {
            let shift = div_round(
                (i32::from(cur) - i32::from(surf)) * CURSOR_TINT_PERCENT,
                100,
            );
            (i32::from(base) + shift).clamp(0, 255) as u8
        };
        Color::Rgb(channel(r, cr, sr), channel(g, cg, sg), channel(b, cb, sb))
    }

    fn ramp_for(self, kind: FlashKind) -> [Color; 4] {
        match kind {
            FlashKind::Add => self.flash_ramp,
            FlashKind::Update => self.flash_update_ramp,
            FlashKind::Remove => self.flash_remove_ramp,
        }
    }

    fn ramp_bg(ramp: [Color; 4], strength: f32) -> Option<Color> {
        if strength <= 0.0 {
            return None;
        }
        let idx = if strength > 0.75 {
            0
        } else if strength > 0.50 {
            1
        } else if strength > 0.25 {
            2
        } else {
            3
        };
        Some(ramp[idx])
    }

    #[cfg(test)]
    /// Background for a decaying add flash. `None` when the flash has expired.
    pub fn flash_bg(self, strength: f32) -> Option<Color> {
        self.flash_bg_for(FlashKind::Add, strength)
    }

    /// Background for a decaying flash of `kind`. `None` when expired.
    pub fn flash_bg_for(self, kind: FlashKind, strength: f32) -> Option<Color> {
        Self::ramp_bg(self.ramp_for(kind), strength)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_order_and_wrap() {
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
        assert_eq!(THEME_IDS, order);
        assert_eq!(DEFAULT_THEME_ID, ThemeId::Slate);
        assert_eq!(ThemeId::default(), DEFAULT_THEME_ID);
        let slugs: Vec<_> = THEME_IDS.iter().map(|id| id.as_str()).collect();
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
        let labels: Vec<_> = THEME_IDS.iter().map(|id| id.label()).collect();
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
        for id in THEME_IDS {
            assert_eq!(id.theme().id, id, "{id:?} preset id");
        }
        let mut id = ThemeId::TokyoNight;
        let mut seen = Vec::new();
        for _ in 0..THEME_IDS.len() {
            id = cycle_theme_id(id);
            seen.push(id);
        }
        let mut expected = order[1..].to_vec();
        expected.push(ThemeId::TokyoNight);
        assert_eq!(seen, expected, "T walks every theme once and wraps");
        assert_eq!(
            cycle_theme_id(ThemeId::GithubDarkDimmed),
            ThemeId::TokyoNight
        );
    }

    #[test]
    fn theme_env_beats_config_and_unknown_env_falls_through() {
        assert_eq!(
            theme_id_from(None, Some(ThemeId::Dracula)),
            ThemeId::Dracula
        );
        assert_eq!(
            theme_id_from(Some("monokai"), Some(ThemeId::Dracula)),
            ThemeId::Monokai
        );
        for bad in ["", "nope", " monokai"] {
            assert_eq!(
                theme_id_from(Some(bad), Some(ThemeId::Dracula)),
                ThemeId::Dracula,
                "env {bad:?}"
            );
        }
    }

    #[test]
    fn resolve_known_and_fallback() {
        assert_eq!(theme_id_from(None, None), ThemeId::Slate);
        assert_eq!(theme_id_from(Some(""), None), ThemeId::Slate);
        assert_eq!(theme_id_from(Some("nope"), None), ThemeId::Slate);
        assert_eq!(parse_theme_id("solarized"), None, "exact slug only");
        assert_eq!(parse_theme_id("Dracula"), None, "case-sensitive");
        for id in THEME_IDS {
            assert_eq!(theme_id_from(Some(id.as_str()), None), id);
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

    /// Minimum contrast between a changed-word bg and its row bg.
    const WORD_VS_ROW_FLOOR: f64 = 1.25;

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

    /// Icon popover and `?` legend text. Both paint on the theme surface
    /// (and on `panel` once popups move there); the same roles paint the
    /// left pane on `sidebar` and the bars on `chrome`:
    /// popover headings and field values in their pane colour roles,
    /// muted labels and notes, legend glyphs in the colour their pane
    /// paints them (`head_mark` for the checkout mark and `[HEAD]`,
    /// `cursor` for the cursor bar), and the `❯` focus marker and cues in
    /// `cursor`. Every role meets AA on the surface, `panel`,
    /// `sidebar`, and `chrome` (`deleted` its 4.0 floor). A pinned popover's focused line puts the same text on
    /// `cursor_bg`: 3.5, and `deleted` 2.5 like the hovered tab close.
    /// Key chips paint the surface on `cursor` (enabled) or `muted`
    /// (disabled): AA.
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
                let ratio = contrast_ratio(surface, chip_bg);
                assert!(ratio >= AA, "{id:?} {name} key chip {ratio:.2} < {AA}");
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

    #[test]
    fn border_dim_is_unique_and_darker_than_muted() {
        const LOCKED: [(ThemeId, &str); 13] = [
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
        ];
        assert_eq!(LOCKED.map(|(id, _)| id), THEME_IDS);
        let mut hexes = Vec::new();
        for (id, hex) in LOCKED {
            let theme = id.theme();
            assert_eq!(theme.palette.border_dim, hex, "{id:?} borderDim hex");
            let pal = id.palette();
            assert_eq!(pal.border_dim, hex_color(hex), "{id:?} palette.border_dim");
            let dim_l = relative_luminance(pal.border_dim);
            let muted_l = relative_luminance(pal.muted);
            assert!(
                dim_l < muted_l,
                "{id:?} luminance(border_dim)={dim_l:.4} must be < luminance(muted)={muted_l:.4}"
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

    /// Idle `[✗]` is a neutral dark gray (R=G=B) that reads dimmer than the
    /// `muted` tab label, yet stays legible on the active tab. Floors:
    /// `muted`/`tab_close` contrast >= 1.5 (visibly darker), `tab_close` >= 2.5
    /// on `cursor_bg` and >= 3.0 on the surface. Hover is the theme's `deleted`
    /// red on every (dark) theme: >= 2.5 on `cursor_bg`.
    #[test]
    fn tab_close_is_dimmer_than_muted_and_hover_is_red() {
        const DIMMER_THAN_MUTED: f64 = 1.5;
        const ON_CURSOR_BG: f64 = 2.5;
        const ON_SURFACE: f64 = 3.0;
        const HOVER_ON_CURSOR_BG: f64 = 2.5;
        for id in THEME_IDS {
            let pal = id.palette();
            let surface = hex_color(id.theme().surface);
            let close_l = relative_luminance(pal.tab_close);
            assert!(
                close_l < relative_luminance(pal.muted),
                "{id:?} tab_close must be darker than muted"
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
                (
                    "tab_close on surface",
                    contrast_ratio(pal.tab_close, surface),
                    ON_SURFACE,
                ),
                (
                    "tab_close_hover on cursor_bg",
                    contrast_ratio(pal.tab_close_hover, pal.cursor_bg),
                    HOVER_ON_CURSOR_BG,
                ),
            ] {
                assert!(ratio >= floor, "{id:?} {name} {ratio:.2} < {floor}");
            }
            assert!(
                relative_luminance(pal.tab_close_hover) > relative_luminance(pal.tab_close),
                "{id:?} tab_close_hover must be brighter than tab_close"
            );
        }
    }

    #[test]
    fn cursor_bg_inactive_is_darker_than_cursor_bg() {
        const LOCKED: [(ThemeId, &str); 13] = [
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
            assert!(
                inactive_l < cursor_l,
                "{id:?} luminance(cursor_bg_inactive)={inactive_l:.4} must be < luminance(cursor_bg)={cursor_l:.4}"
            );
            assert!(
                inactive_l > surface_l,
                "{id:?} luminance(cursor_bg_inactive)={inactive_l:.4} must be > luminance(surface)={surface_l:.4}"
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
}
