//! Built-in TUI colour themes.
//!
//! Thirteen dark palettes, each with a light twin ([`ThemeId::twin`]).
//! Secondary tokens (`muted`, line numbers, graph meta) stay readable on
//! the theme surface and on the `sidebar`, `chrome`, and `panel`
//! backgrounds. Launch seed is `WS_STATUS_THEME`, then the last-used theme
//! from the state file, then the config `theme` key, then
//! [`DEFAULT_THEME_ID`] (Slate) ([`theme_id_from`]). The command palette
//! has a `Theme: …` row per theme, dark then light ([`THEME_IDS`]):
//! focusing one previews it, Enter sets it. The last-used theme lives in
//! the UI-state file ([`super::ui_state_store`]), never in the config file.

use ratatui::style::Color;

use super::watch::FlashKind;

/// Built-in theme identifiers: thirteen dark themes and their light twins.
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
    /// Light twin of [`Self::TokyoNight`].
    TokyoNightDay,
    /// Light twin of [`Self::Monokai`].
    MonokaiLight,
    /// Light twin of [`Self::Dracula`] (the Alucard palette).
    DraculaLight,
    /// Light twin of [`Self::GruvboxDark`].
    GruvboxLight,
    /// Light twin of [`Self::CatppuccinMocha`].
    CatppuccinLatte,
    /// Light twin of [`Self::Slate`].
    SlateLight,
    /// Light twin of [`Self::SolarizedDark`].
    SolarizedLight,
    /// Light twin of [`Self::Nord`].
    NordLight,
    /// Light twin of [`Self::RosePine`].
    RosePineDawn,
    /// Light twin of [`Self::Kanagawa`].
    KanagawaLotus,
    /// Light twin of [`Self::Everforest`].
    EverforestLight,
    /// Light twin of [`Self::OneDark`].
    OneLight,
    /// Light twin of [`Self::GithubDarkDimmed`].
    GithubLight,
}

/// Dark themes in palette row order. [`THEME_IDS`] lists them first,
/// then [`LIGHT_THEME_IDS`].
pub const DARK_THEME_IDS: [ThemeId; 13] = [
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

/// Light themes in palette row order: index `i` is the twin of
/// `DARK_THEME_IDS[i]`.
pub const LIGHT_THEME_IDS: [ThemeId; 13] = [
    ThemeId::TokyoNightDay,
    ThemeId::MonokaiLight,
    ThemeId::DraculaLight,
    ThemeId::GruvboxLight,
    ThemeId::CatppuccinLatte,
    ThemeId::SlateLight,
    ThemeId::SolarizedLight,
    ThemeId::NordLight,
    ThemeId::RosePineDawn,
    ThemeId::KanagawaLotus,
    ThemeId::EverforestLight,
    ThemeId::OneLight,
    ThemeId::GithubLight,
];

/// Every built-in theme in palette row order (`Theme: …` rows):
/// [`DARK_THEME_IDS`], then [`LIGHT_THEME_IDS`].
pub const THEME_IDS: [ThemeId; 26] = {
    let mut all = [ThemeId::TokyoNight; 26];
    let mut i = 0;
    while i < DARK_THEME_IDS.len() {
        all[i] = DARK_THEME_IDS[i];
        all[DARK_THEME_IDS.len() + i] = LIGHT_THEME_IDS[i];
        i += 1;
    }
    all
};

/// Default when no `WS_STATUS_THEME`, last-used state, or config `theme`
/// key sets a theme.
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
    /// Unfocused pane border. Near-surface gray: less contrast on the surface
    /// than [`Self::muted`].
    pub border_dim: Color,
    /// Idle compare-tab close `[✗]`. Neutral gray, dimmer (less contrast on
    /// the surface) than [`Self::muted`], still readable on the surface and
    /// on [`Self::cursor_bg`] (active tab).
    pub tab_close: Color,
    /// Hovered compare-tab close `[✗]`. Red, the theme's [`Self::deleted`].
    pub tab_close_hover: Color,
    pub added: Color,
    pub modified: Color,
    pub deleted: Color,
    pub renamed: Color,
    /// Reviewed eye (`ICON_VIEWED`). Teal/cyan.
    pub viewed: Color,
    pub branch_default: Color,
    pub branch_feature: Color,
    pub head_mark: Color,
    pub cursor: Color,
    pub cursor_bg: Color,
    /// Selected-row background on an unfocused list. Between the surface and
    /// [`Self::cursor_bg`] (a weaker shift).
    pub cursor_bg_inactive: Color,
    /// Pane surface (background): the right pane. The base
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
    /// gray, less contrast on the surface than [`Self::muted`]. Not a
    /// readable-text token.
    pub border_dim: &'static str,
    /// Idle compare-tab close `[✗]` hex. Neutral gray, dimmer than
    /// [`Self::muted`].
    pub tab_close: &'static str,
    /// Hovered compare-tab close `[✗]` hex. Red, the theme's `deleted` hex.
    pub tab_close_hover: &'static str,
    pub added: &'static str,
    pub modified: &'static str,
    pub deleted: &'static str,
    pub renamed: &'static str,
    /// Reviewed eye hex. Teal/cyan.
    pub viewed: &'static str,
    pub branch_default: &'static str,
    pub branch_feature: &'static str,
    pub head_mark: &'static str,
    pub cursor: &'static str,
    pub cursor_bg: &'static str,
    /// Unfocused selected-row background hex. Between the surface and
    /// [`Self::cursor_bg`].
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
    /// mixed 22% toward black. Light presets must set it: the default rule
    /// suits dark surfaces only.
    pub sidebar: Option<&'static str>,
    /// Tab strip, breadcrumb, prompt, and footer row background hex. `None`
    /// uses the default rule: the surface mixed 38% toward black. Light
    /// presets must set it.
    pub chrome: Option<&'static str>,
    /// Popup background hex. `None` uses the default rule: the surface mixed
    /// 5% toward white. Light presets must set it.
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
            ThemeId::TokyoNightDay => "tokyo-night-day",
            ThemeId::MonokaiLight => "monokai-light",
            ThemeId::DraculaLight => "dracula-light",
            ThemeId::GruvboxLight => "gruvbox-light",
            ThemeId::CatppuccinLatte => "catppuccin-latte",
            ThemeId::SlateLight => "slate-light",
            ThemeId::SolarizedLight => "solarized-light",
            ThemeId::NordLight => "nord-light",
            ThemeId::RosePineDawn => "rose-pine-dawn",
            ThemeId::KanagawaLotus => "kanagawa-lotus",
            ThemeId::EverforestLight => "everforest-light",
            ThemeId::OneLight => "one-light",
            ThemeId::GithubLight => "github-light",
        }
    }

    /// True for a light theme (one of [`LIGHT_THEME_IDS`]).
    pub fn is_light(self) -> bool {
        LIGHT_THEME_IDS.contains(&self)
    }

    /// The same theme in the other polarity: dark to light and light to
    /// dark. `twin(twin(x)) == x`.
    pub fn twin(self) -> ThemeId {
        use ThemeId::*;
        match self {
            TokyoNight => TokyoNightDay,
            Monokai => MonokaiLight,
            Dracula => DraculaLight,
            GruvboxDark => GruvboxLight,
            CatppuccinMocha => CatppuccinLatte,
            Slate => SlateLight,
            SolarizedDark => SolarizedLight,
            Nord => NordLight,
            RosePine => RosePineDawn,
            Kanagawa => KanagawaLotus,
            Everforest => EverforestLight,
            OneDark => OneLight,
            GithubDarkDimmed => GithubLight,
            TokyoNightDay => TokyoNight,
            MonokaiLight => Monokai,
            DraculaLight => Dracula,
            GruvboxLight => GruvboxDark,
            CatppuccinLatte => CatppuccinMocha,
            SlateLight => Slate,
            SolarizedLight => SolarizedDark,
            NordLight => Nord,
            RosePineDawn => RosePine,
            KanagawaLotus => Kanagawa,
            EverforestLight => Everforest,
            OneLight => OneDark,
            GithubLight => GithubDarkDimmed,
        }
    }

    /// The dark theme of this pair: `self` when dark, else its twin.
    pub fn dark_variant(self) -> ThemeId {
        if self.is_light() {
            self.twin()
        } else {
            self
        }
    }

    /// The light theme of this pair: `self` when light, else its twin.
    pub fn light_variant(self) -> ThemeId {
        if self.is_light() {
            self
        } else {
            self.twin()
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
            ThemeId::TokyoNightDay => TOKYO_NIGHT_DAY,
            ThemeId::MonokaiLight => MONOKAI_LIGHT,
            ThemeId::DraculaLight => DRACULA_LIGHT,
            ThemeId::GruvboxLight => GRUVBOX_LIGHT,
            ThemeId::CatppuccinLatte => CATPPUCCIN_LATTE,
            ThemeId::SlateLight => SLATE_LIGHT,
            ThemeId::SolarizedLight => SOLARIZED_LIGHT,
            ThemeId::NordLight => NORD_LIGHT,
            ThemeId::RosePineDawn => ROSE_PINE_DAWN,
            ThemeId::KanagawaLotus => KANAGAWA_LOTUS,
            ThemeId::EverforestLight => EVERFOREST_LIGHT,
            ThemeId::OneLight => ONE_LIGHT,
            ThemeId::GithubLight => GITHUB_LIGHT,
        }
    }

    /// Graph gutter colours of this theme.
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
// Light twins. Accents start from each upstream light palette (Tokyo Night
// Day, Catppuccin Latte, Gruvbox light, Solarized light, Rosé Pine Dawn,
// Kanagawa Lotus, Everforest light, Atom One Light, GitHub Light, Dracula
// Alucard; Slate, Nord, and Monokai light are hand-made) and are darkened
// only as far as the contrast tests below need. Every light preset sets
// `sidebar`, `chrome`, and `panel`: the default rule suits dark surfaces.

const TOKYO_NIGHT_DAY: Theme = Theme {
    id: ThemeId::TokyoNightDay,
    label: "Tokyo Night Day",
    surface: "#e1e2e7",
    sidebar: Some("#d9dbe2"),
    chrome: Some("#d0d5e3"),
    panel: Some("#eaebef"),
    palette: ThemePalette {
        heading: "#006385",
        repo: "#343b58",
        dir: "#1359b8",
        file: "#3258af",
        muted: "#47578f",
        border_dim: "#bec1ca",
        tab_close: "#808080",
        tab_close_hover: "#c60940",
        added: "#4b6330",
        modified: "#715732",
        deleted: "#c60940",
        renamed: "#006385",
        viewed: "#0c6453",
        branch_default: "#1359b8",
        branch_feature: "#761bec",
        head_mark: "#715732",
        cursor: "#1359b8",
        cursor_bg: "#c4d2e7",
        cursor_bg_inactive: "#d2dae7",
        diff_hunk: "#006385",
        diff_add_bg: "#e4f2d3",
        diff_del_bg: "#f2d3dc",
        diff_add_word_bg: "#bbdf92",
        diff_del_word_bg: "#e9b4c3",
        flash: "#bbdf92",
        flash_ramp: ["#bbdf92", "#c6e0ab", "#d2e1c5", "#dbe2da"],
        flash_update: "#dfbf92",
        flash_update_ramp: ["#dfbf92", "#e0c9ab", "#e0d4c5", "#e1ddda"],
        flash_remove: "#e9b4c3",
        flash_remove_ramp: ["#e9b4c3", "#e7c2ce", "#e4d0d9", "#e2dbe2"],
    },
    pill: ThemePill {
        mode_bg: "#1359b8",
        mode_fg: "#e1e2e7",
        diff_bg: "#b4c4dd",
        diff_fg: "#343b58",
        filter_bg: "#761bec",
        filter_fg: "#e1e2e7",
    },
    lane_colors: [
        "#2d7ae4", "#9854f1", "#007197", "#587539", "#8c6c3e", "#eb2861", "#b15c00", "#118c74",
    ],
};

const MONOKAI_LIGHT: Theme = Theme {
    id: ThemeId::MonokaiLight,
    label: "Monokai Light",
    surface: "#fafaf5",
    sidebar: Some("#f0f0eb"),
    chrome: Some("#e6e6e1"),
    panel: Some("#f5f5f0"),
    palette: ThemePalette {
        heading: "#167087",
        repo: "#272822",
        dir: "#167087",
        file: "#49483e",
        muted: "#656251",
        border_dim: "#d0d0cb",
        tab_close: "#8b8b8b",
        tab_close_hover: "#d32257",
        added: "#1c744e",
        modified: "#945807",
        deleted: "#d32257",
        renamed: "#167087",
        viewed: "#1d6d63",
        branch_default: "#167087",
        branch_feature: "#6d55bd",
        head_mark: "#1c744e",
        cursor: "#49483e",
        cursor_bg: "#deded8",
        cursor_bg_inactive: "#ecece6",
        diff_hunk: "#167087",
        diff_add_bg: "#ddf5eb",
        diff_del_bg: "#f5dde5",
        diff_add_word_bg: "#9ee3c5",
        diff_del_word_bg: "#ecbecc",
        flash: "#9ee3c5",
        flash_ramp: ["#9ee3c5", "#baead3", "#d5f1e2", "#ecf7ee"],
        flash_update: "#e3c69e",
        flash_update_ramp: ["#e3c69e", "#ead6b8", "#f1e5d2", "#f7f2e8"],
        flash_remove: "#ecbecc",
        flash_remove_ramp: ["#ecbecc", "#f0d0d8", "#f4e2e5", "#f8f1ef"],
    },
    pill: ThemePill {
        mode_bg: "#49483e",
        mode_fg: "#fafaf5",
        diff_bg: "#d3d3cd",
        diff_fg: "#272822",
        filter_bg: "#6d55bd",
        filter_fg: "#fafaf5",
    },
    lane_colors: [
        "#1c8ca8", "#7058be", "#269d69", "#c6760a", "#e14775", "#e16032", "#49483e", "#299a8c",
    ],
};

const DRACULA_LIGHT: Theme = Theme {
    id: ThemeId::DraculaLight,
    label: "Dracula Light",
    surface: "#fffbeb",
    sidebar: Some("#f4f0de"),
    chrome: Some("#ece8d4"),
    panel: Some("#faf6e6"),
    palette: ThemePalette {
        heading: "#036a96",
        repo: "#1f1f1f",
        dir: "#644ac9",
        file: "#1f1f1f",
        muted: "#6a6449",
        border_dim: "#d2cfc2",
        tab_close: "#8d8d8d",
        tab_close_hover: "#cb3a2a",
        added: "#14710a",
        modified: "#796513",
        deleted: "#cb3a2a",
        renamed: "#036a96",
        viewed: "#0a6f64",
        branch_default: "#644ac9",
        branch_feature: "#644ac9",
        head_mark: "#14710a",
        cursor: "#644ac9",
        cursor_bg: "#e6dfe6",
        cursor_bg_inactive: "#f2ede8",
        diff_hunk: "#036a96",
        diff_add_bg: "#e0f5dd",
        diff_del_bg: "#f5e0dd",
        diff_add_word_bg: "#a9e4a2",
        diff_del_word_bg: "#ecc2be",
        flash: "#a9e4a2",
        flash_ramp: ["#a9e4a2", "#c3ebb8", "#ddf2ce", "#f2f8e0"],
        flash_update: "#e4d7a2",
        flash_update_ramp: ["#e4d7a2", "#ece2b8", "#f4edce", "#fbf6e0"],
        flash_remove: "#ecc2be",
        flash_remove_ramp: ["#ecc2be", "#f2d3cb", "#f7e4d9", "#fcf2e4"],
    },
    pill: ThemePill {
        mode_bg: "#644ac9",
        mode_fg: "#fffbeb",
        diff_bg: "#ddd4e4",
        diff_fg: "#1f1f1f",
        filter_bg: "#644ac9",
        filter_fg: "#fffbeb",
    },
    lane_colors: [
        "#036a96", "#644ac9", "#14710a", "#846e15", "#cb3a2a", "#a34d14", "#a3144d", "#635d9f",
    ],
};

const GRUVBOX_LIGHT: Theme = Theme {
    id: ThemeId::GruvboxLight,
    label: "Gruvbox Light",
    surface: "#fbf1c7",
    sidebar: Some("#f2e5bc"),
    chrome: Some("#ebdbb2"),
    panel: Some("#fdf7dd"),
    palette: ThemePalette {
        heading: "#076678",
        repo: "#3c3836",
        dir: "#396b4d",
        file: "#3c3836",
        muted: "#675c53",
        border_dim: "#d5ccaa",
        tab_close: "#868686",
        tab_close_hover: "#9d0006",
        added: "#67620c",
        modified: "#85570f",
        deleted: "#9d0006",
        renamed: "#076678",
        viewed: "#406725",
        branch_default: "#076678",
        branch_feature: "#8f3f71",
        head_mark: "#ad3903",
        cursor: "#ad3903",
        cursor_bg: "#efd4a8",
        cursor_bg_inactive: "#f5e2b7",
        diff_hunk: "#076678",
        diff_add_bg: "#f1efce",
        diff_del_bg: "#f1ced0",
        diff_add_word_bg: "#dbd681",
        diff_del_word_bg: "#e7abad",
        flash: "#dbd681",
        flash_ramp: ["#dbd681", "#e5de96", "#eee6ab", "#f6edbc"],
        flash_update: "#dbb881",
        flash_update_ramp: ["#dbb881", "#e5c996", "#eedaab", "#f6e8bc"],
        flash_remove: "#e7abad",
        flash_remove_ramp: ["#e7abad", "#edc0b5", "#f3d5bd", "#f8e6c3"],
    },
    pill: ThemePill {
        mode_bg: "#ad3903",
        mode_fg: "#fbf1c7",
        diff_bg: "#eac99c",
        diff_fg: "#3c3836",
        filter_bg: "#8f3f71",
        filter_fg: "#fbf1c7",
    },
    lane_colors: [
        "#076678", "#8f3f71", "#79740e", "#b57614", "#9d0006", "#af3a03", "#427b58", "#aa791a",
    ],
};

const CATPPUCCIN_LATTE: Theme = Theme {
    id: ThemeId::CatppuccinLatte,
    label: "Catppuccin Latte",
    surface: "#eff1f5",
    sidebar: Some("#e6e9ef"),
    chrome: Some("#dce0e8"),
    panel: Some("#f5f7f9"),
    palette: ThemePalette {
        heading: "#036a93",
        repo: "#4c4f69",
        dir: "#0a55ea",
        file: "#4c4f69",
        muted: "#5c5f72",
        border_dim: "#ced1d9",
        tab_close: "#888888",
        tab_close_hover: "#d20f39",
        added: "#2d711e",
        modified: "#8a5812",
        deleted: "#d20f39",
        renamed: "#036a93",
        viewed: "#11696e",
        branch_default: "#0a55ea",
        branch_feature: "#7f2aee",
        head_mark: "#8a5812",
        cursor: "#0a55ea",
        cursor_bg: "#cedbf5",
        cursor_bg_inactive: "#dee6f5",
        diff_hunk: "#036a93",
        diff_add_bg: "#e2f5dd",
        diff_del_bg: "#f5dde3",
        diff_add_word_bg: "#aee4a2",
        diff_del_word_bg: "#ecbec8",
        flash: "#aee4a2",
        flash_ramp: ["#aee4a2", "#c1e8bb", "#d5ecd4", "#e5efe9"],
        flash_update: "#e4c8a2",
        flash_update_ramp: ["#e4c8a2", "#e7d4bb", "#ebe1d4", "#edebe9"],
        flash_remove: "#ecbec8",
        flash_remove_ramp: ["#ecbec8", "#edcdd5", "#eedde3", "#efe9ee"],
    },
    pill: ThemePill {
        mode_bg: "#0a55ea",
        mode_fg: "#eff1f5",
        diff_bg: "#bdcff3",
        diff_fg: "#4c4f69",
        filter_bg: "#7f2aee",
        filter_fg: "#eff1f5",
    },
    lane_colors: [
        "#1e66f5", "#8839ef", "#3c9829", "#ba7618", "#d20f39", "#e65401", "#048ec5", "#db4cb4",
    ],
};

const SLATE_LIGHT: Theme = Theme {
    id: ThemeId::SlateLight,
    label: "Slate Light",
    surface: "#f5f7fa",
    sidebar: Some("#eceff4"),
    chrome: Some("#e3e7ed"),
    panel: Some("#f9fafc"),
    palette: ThemePalette {
        heading: "#1c7083",
        repo: "#1f2733",
        dir: "#2f63a8",
        file: "#3a4555",
        muted: "#596372",
        border_dim: "#cacdd2",
        tab_close: "#8b8b8b",
        tab_close_hover: "#c03a3a",
        added: "#2c7443",
        modified: "#865f12",
        deleted: "#c03a3a",
        renamed: "#1c7083",
        viewed: "#146d6a",
        branch_default: "#2f63a8",
        branch_feature: "#6b4fb0",
        head_mark: "#865f12",
        cursor: "#2f63a8",
        cursor_bg: "#d5dfed",
        cursor_bg_inactive: "#e5ebf3",
        diff_hunk: "#1c7083",
        diff_add_bg: "#ddf5e5",
        diff_del_bg: "#f5dddd",
        diff_add_word_bg: "#a2e4b7",
        diff_del_word_bg: "#ecbebe",
        flash: "#a2e4b7",
        flash_ramp: ["#a2e4b7", "#bbeacb", "#d4efdf", "#e9f4f0"],
        flash_update: "#e4cea2",
        flash_update_ramp: ["#e4cea2", "#e9dabc", "#eee7d7", "#f2f1ed"],
        flash_remove: "#ecbebe",
        flash_remove_ramp: ["#ecbebe", "#efcfd0", "#f1e0e2", "#f4eef1"],
    },
    pill: ThemePill {
        mode_bg: "#2f63a8",
        mode_fg: "#f5f7fa",
        diff_bg: "#c9d6e8",
        diff_fg: "#1f2733",
        filter_bg: "#6b4fb0",
        filter_fg: "#f5f7fa",
    },
    lane_colors: [
        "#2f63a8", "#6b4fb0", "#1f7a8f", "#2f7d48", "#8a6212", "#c03a3a", "#b0592a", "#18807c",
    ],
};

const SOLARIZED_LIGHT: Theme = Theme {
    id: ThemeId::SolarizedLight,
    label: "Solarized Light",
    surface: "#fdf6e3",
    sidebar: Some("#f5eedb"),
    chrome: Some("#eee8d5"),
    panel: Some("#fefaee"),
    palette: ThemePalette {
        heading: "#1e746d",
        repo: "#073642",
        dir: "#1e6da5",
        file: "#566c73",
        muted: "#55686e",
        border_dim: "#ccd0c3",
        tab_close: "#8e8e8e",
        tab_close_hover: "#d82824",
        added: "#606e00",
        modified: "#826200",
        deleted: "#d82824",
        renamed: "#1e746d",
        viewed: "#2a7150",
        branch_default: "#1e6da5",
        branch_feature: "#595fbc",
        head_mark: "#826200",
        cursor: "#1e6da5",
        cursor_bg: "#dbe5e0",
        cursor_bg_inactive: "#ecede1",
        diff_hunk: "#1e746d",
        diff_add_bg: "#f2f5dd",
        diff_del_bg: "#f5dedd",
        diff_add_word_bg: "#d6df92",
        diff_del_word_bg: "#ecbfbe",
        flash: "#d6df92",
        flash_ramp: ["#d6df92", "#e2e6aa", "#ededc3", "#f7f3d7"],
        flash_update: "#dfcc92",
        flash_update_ramp: ["#dfcc92", "#e8d9aa", "#f1e5c3", "#f8f0d7"],
        flash_remove: "#ecbfbe",
        flash_remove_ramp: ["#ecbfbe", "#f1cfc9", "#f6e0d4", "#faeedd"],
    },
    pill: ThemePill {
        mode_bg: "#1e6da5",
        mode_fg: "#fdf6e3",
        diff_bg: "#ccd8d5",
        diff_fg: "#073642",
        filter_bg: "#595fbc",
        filter_fg: "#fdf6e3",
    },
    lane_colors: [
        "#268bd2", "#6c71c4", "#27988f", "#7f9200", "#ac8200", "#dc322f", "#cb4b16", "#d33682",
    ],
};

const NORD_LIGHT: Theme = Theme {
    id: ThemeId::NordLight,
    label: "Nord Light",
    surface: "#eceff4",
    sidebar: Some("#e5e9f0"),
    chrome: Some("#d8dee9"),
    panel: Some("#f4f5f8"),
    palette: ThemePalette {
        heading: "#396875",
        repo: "#2e3440",
        dir: "#466488",
        file: "#3b4252",
        muted: "#4c566a",
        border_dim: "#c6cad0",
        tab_close: "#898989",
        tab_close_hover: "#b04751",
        added: "#50683c",
        modified: "#7a5d21",
        deleted: "#b04751",
        renamed: "#396875",
        viewed: "#3c6868",
        branch_default: "#466488",
        branch_feature: "#7b5674",
        head_mark: "#7a5d21",
        cursor: "#466488",
        cursor_bg: "#d5dde8",
        cursor_bg_inactive: "#e0e6ee",
        diff_hunk: "#396875",
        diff_add_bg: "#e8f5dd",
        diff_del_bg: "#f5dde0",
        diff_add_word_bg: "#bce29c",
        diff_del_word_bg: "#ecbec2",
        flash: "#bce29c",
        flash_ramp: ["#bce29c", "#cae6b6", "#d9ead1", "#e5ede7"],
        flash_update: "#e2cb9c",
        flash_update_ramp: ["#e2cb9c", "#e5d6b6", "#e8e1d1", "#eaeae7"],
        flash_remove: "#ecbec2",
        flash_remove_ramp: ["#ecbec2", "#eccdd1", "#ecdbe0", "#ece8ec"],
    },
    pill: ThemePill {
        mode_bg: "#466488",
        mode_fg: "#eceff4",
        diff_bg: "#c7d0dc",
        diff_fg: "#2e3440",
        filter_bg: "#7b5674",
        filter_fg: "#eceff4",
    },
    lane_colors: [
        "#5e81ac", "#a3789b", "#4c8a9b", "#6a8a4f", "#a07a2c", "#bf616a", "#c26f54", "#4f8a89",
    ],
};

const ROSE_PINE_DAWN: Theme = Theme {
    id: ThemeId::RosePineDawn,
    label: "Rosé Pine Dawn",
    surface: "#faf4ed",
    sidebar: Some("#f2e9e1"),
    chrome: Some("#ebe1d8"),
    panel: Some("#fffaf3"),
    palette: ThemePalette {
        heading: "#b03c37",
        repo: "#575279",
        dir: "#3e6b73",
        file: "#575279",
        muted: "#625e79",
        border_dim: "#d9d4d6",
        tab_close: "#8a8a8a",
        tab_close_hover: "#a85169",
        added: "#3e6b73",
        modified: "#8f590f",
        deleted: "#a85169",
        renamed: "#b03c37",
        viewed: "#36695f",
        branch_default: "#3e6b73",
        branch_feature: "#725b8d",
        head_mark: "#8f590f",
        cursor: "#286983",
        cursor_bg: "#d8dedc",
        cursor_bg_inactive: "#e9e9e4",
        diff_hunk: "#b03c37",
        diff_add_bg: "#ddf2f5",
        diff_del_bg: "#f5dde4",
        diff_add_word_bg: "#a8dce6",
        diff_del_word_bg: "#ecbecb",
        flash: "#a8dce6",
        flash_ramp: ["#a8dce6", "#c1e3e8", "#d9eaea", "#eef0ec"],
        flash_update: "#e6cca8",
        flash_update_ramp: ["#e6cca8", "#ecd8bd", "#f2e4d1", "#f7eee3"],
        flash_remove: "#ecbecb",
        flash_remove_ramp: ["#ecbecb", "#f0ced5", "#f4dedf", "#f8ece8"],
    },
    pill: ThemePill {
        mode_bg: "#286983",
        mode_fg: "#faf4ed",
        diff_bg: "#ccd5d6",
        diff_fg: "#575279",
        filter_bg: "#725b8d",
        filter_fg: "#faf4ed",
    },
    lane_colors: [
        "#286983", "#907aa9", "#c86b68", "#53909a", "#be7614", "#b4637a", "#ba7467", "#4b9283",
    ],
};

const KANAGAWA_LOTUS: Theme = Theme {
    id: ThemeId::KanagawaLotus,
    label: "Kanagawa Lotus",
    surface: "#f2ecbc",
    sidebar: Some("#e7dba0"),
    chrome: Some("#e5ddb0"),
    panel: Some("#f5f0cc"),
    palette: ThemePalette {
        heading: "#386575",
        repo: "#43436c",
        dir: "#46608d",
        file: "#545464",
        muted: "#5d5c55",
        border_dim: "#cfcaac",
        tab_close: "#848484",
        tab_close_hover: "#ba3648",
        added: "#526539",
        modified: "#645f35",
        deleted: "#ba3648",
        renamed: "#386575",
        viewed: "#46615c",
        branch_default: "#46608d",
        branch_feature: "#624c83",
        head_mark: "#6e5d3e",
        cursor: "#46608d",
        cursor_bg: "#d8d7b7",
        cursor_bg_inactive: "#e5e1b9",
        diff_hunk: "#386575",
        diff_add_bg: "#d9ecbf",
        diff_del_bg: "#ecbfc5",
        diff_add_word_bg: "#b2d97e",
        diff_del_word_bg: "#e4a1aa",
        flash: "#b2d97e",
        flash_ramp: ["#b2d97e", "#c5df91", "#d8e4a3", "#e8e9b3"],
        flash_update: "#d9d07e",
        flash_update_ramp: ["#d9d07e", "#e0d891", "#e8e1a3", "#eee8b3"],
        flash_remove: "#e4a1aa",
        flash_remove_ramp: ["#e4a1aa", "#e8b7af", "#ecceb5", "#f0e1b9"],
    },
    pill: ThemePill {
        mode_bg: "#46608d",
        mode_fg: "#f2ecbc",
        diff_bg: "#cccdb2",
        diff_fg: "#43436c",
        filter_bg: "#624c83",
        filter_fg: "#f2ecbc",
    },
    lane_colors: [
        "#4d699b", "#624c83", "#498297", "#69824a", "#836f4a", "#c84053", "#bc6400", "#597b75",
    ],
};

const EVERFOREST_LIGHT: Theme = Theme {
    id: ThemeId::EverforestLight,
    label: "Everforest Light",
    surface: "#fffbef",
    sidebar: Some("#f4f0d9"),
    chrome: Some("#efebd4"),
    panel: Some("#faf6ea"),
    palette: ThemePalette {
        heading: "#267758",
        repo: "#4f5b62",
        dir: "#2c7096",
        file: "#5c6a72",
        muted: "#5f6c5e",
        border_dim: "#dcdbd3",
        tab_close: "#919191",
        tab_close_hover: "#e30d09",
        added: "#627001",
        modified: "#896300",
        deleted: "#e30d09",
        renamed: "#267758",
        viewed: "#377542",
        branch_default: "#2c7096",
        branch_feature: "#bc288e",
        head_mark: "#896300",
        cursor: "#2c7096",
        cursor_bg: "#dfebe8",
        cursor_bg_inactive: "#eff3eb",
        diff_hunk: "#267758",
        diff_add_bg: "#f2f5dd",
        diff_del_bg: "#f5dedd",
        diff_add_word_bg: "#d6df92",
        diff_del_word_bg: "#ecbfbe",
        flash: "#d6df92",
        flash_ramp: ["#d6df92", "#e2e7ae", "#eff0ca", "#f9f7e1"],
        flash_update: "#dfca92",
        flash_update_ramp: ["#dfca92", "#e9d9ae", "#f2e7ca", "#faf4e1"],
        flash_remove: "#ecbfbe",
        flash_remove_ramp: ["#ecbfbe", "#f2d1cd", "#f7e3db", "#fcf2e8"],
    },
    pill: ThemePill {
        mode_bg: "#2c7096",
        mode_fg: "#fffbef",
        diff_bg: "#d1dcdb",
        diff_fg: "#4f5b62",
        filter_bg: "#bc288e",
        filter_fg: "#fffbef",
    },
    lane_colors: [
        "#3991c1", "#d361af", "#319a73", "#809201", "#b38000", "#f15250", "#e1650a", "#499a57",
    ],
};

const ONE_LIGHT: Theme = Theme {
    id: ThemeId::OneLight,
    label: "One Light",
    surface: "#fafafa",
    sidebar: Some("#f0f0f1"),
    chrome: Some("#e5e5e6"),
    panel: Some("#f5f5f5"),
    palette: ThemePalette {
        heading: "#016d9c",
        repo: "#383a42",
        dir: "#1258ef",
        file: "#383a42",
        muted: "#636670",
        border_dim: "#d3d4d5",
        tab_close: "#909090",
        tab_close_hover: "#d12e1f",
        added: "#397238",
        modified: "#8a5e01",
        deleted: "#d12e1f",
        renamed: "#016d9c",
        viewed: "#0b7364",
        branch_default: "#1258ef",
        branch_feature: "#a626a4",
        head_mark: "#8a5e01",
        cursor: "#1258ef",
        cursor_bg: "#dce5f9",
        cursor_bg_inactive: "#ebeff9",
        diff_hunk: "#016d9c",
        diff_add_bg: "#def5dd",
        diff_del_bg: "#f5dfdd",
        diff_add_word_bg: "#a3e4a2",
        diff_del_word_bg: "#ebbeba",
        flash: "#a3e4a2",
        flash_ramp: ["#a3e4a2", "#bdebbc", "#d7f1d7", "#edf7ed"],
        flash_update: "#e4cfa2",
        flash_update_ramp: ["#e4cfa2", "#ebdcbc", "#f1e9d7", "#f7f4ed"],
        flash_remove: "#ebbeba",
        flash_remove_ramp: ["#ebbeba", "#efd0cd", "#f4e2e0", "#f8f1f0"],
    },
    pill: ThemePill {
        mode_bg: "#1258ef",
        mode_fg: "#fafafa",
        diff_bg: "#c7d6f8",
        diff_fg: "#383a42",
        filter_bg: "#a626a4",
        filter_fg: "#fafafa",
    },
    lane_colors: [
        "#4078f2", "#a626a4", "#0184bc", "#4d9b4c", "#b97f01", "#e45649", "#986801", "#0f9a86",
    ],
};

const GITHUB_LIGHT: Theme = Theme {
    id: ThemeId::GithubLight,
    label: "GitHub Light",
    surface: "#ffffff",
    sidebar: Some("#f6f8fa"),
    chrome: Some("#eff2f5"),
    panel: Some("#fafbfc"),
    palette: ThemePalette {
        heading: "#0550ae",
        repo: "#1f2328",
        dir: "#0969da",
        file: "#1f2328",
        muted: "#59636e",
        border_dim: "#d2d3d4",
        tab_close: "#909090",
        tab_close_hover: "#d1242f",
        added: "#1a7f37",
        modified: "#966400",
        deleted: "#d1242f",
        renamed: "#0550ae",
        viewed: "#197177",
        branch_default: "#0969da",
        branch_feature: "#814edf",
        head_mark: "#966400",
        cursor: "#0969da",
        cursor_bg: "#d8e7f9",
        cursor_bg_inactive: "#ebf3fc",
        diff_hunk: "#0550ae",
        diff_add_bg: "#ddf5e4",
        diff_del_bg: "#f5dddf",
        diff_add_word_bg: "#a2e4b5",
        diff_del_word_bg: "#ecbec1",
        flash: "#a2e4b5",
        flash_ramp: ["#a2e4b5", "#beeccb", "#daf4e1", "#f1fbf4"],
        flash_update: "#e4cea2",
        flash_update_ramp: ["#e4cea2", "#ecddbe", "#f4ebda", "#fbf8f1"],
        flash_remove: "#ecbec1",
        flash_remove_ramp: ["#ecbec1", "#f2d1d4", "#f7e5e6", "#fcf5f6"],
    },
    pill: ThemePill {
        mode_bg: "#0969da",
        mode_fg: "#ffffff",
        diff_bg: "#c9def7",
        diff_fg: "#1f2328",
        filter_bg: "#814edf",
        filter_fg: "#ffffff",
    },
    lane_colors: [
        "#0969da", "#8250df", "#0550ae", "#1a7f37", "#9a6700", "#d1242f", "#bc4c00", "#1b7c83",
    ],
};

/// Built-in theme id (dark or light) for an exact slug (`tokyo-night`,
/// `tokyo-night-day`). `None` when unknown.
pub fn parse_theme_id(raw: &str) -> Option<ThemeId> {
    THEME_IDS.into_iter().find(|id| id.as_str() == raw)
}

/// Launch theme: a known `WS_STATUS_THEME` slug wins, then `last_used` (the
/// UI-state file, see [`super::ui_state_store::load_ui_state`]), then the
/// config `theme` key, then [`DEFAULT_THEME_ID`]. An unset or unknown env
/// value falls through to the last-used value, then the config value.
pub fn theme_id_from(
    env_value: Option<&str>,
    last_used: Option<ThemeId>,
    config: Option<ThemeId>,
) -> ThemeId {
    env_value
        .and_then(parse_theme_id)
        .or(last_used)
        .or(config)
        .unwrap_or(DEFAULT_THEME_ID)
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
pub(crate) fn mix_toward(base: Color, target: Color, percent: i32) -> Color {
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
    /// Underline colour under the code of a line with an open comment (the
    /// curly underline, SGR 58).
    ///
    /// The comment accent: `heading`, the colour of the graph's open-comment
    /// mark and the comment dialog title.
    pub fn comment_underline(self) -> Color {
        self.heading
    }

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
mod tests;
