use crate::ui::tui::config::KeyMap;
use std::sync::OnceLock;
use strum_macros::{Display, EnumString, IntoStaticStr};

#[derive(Copy, Clone, PartialEq, Debug, EnumString, Display, IntoStaticStr)]
pub enum Theme {
    #[strum(serialize = "none")]
    None,
    #[strum(serialize = "inspired_github")]
    InspiredGitHub,
    #[strum(serialize = "solarized_dark")]
    SolarizedDark,
    #[strum(serialize = "solarized_light")]
    SolarizedLight,
    #[strum(serialize = "base16_eighties_dark")]
    Base16EightiesDark,
    #[strum(serialize = "base16_mocha_dark")]
    Base16MochaDark,
    #[strum(serialize = "base16_ocean_dark")]
    Base16OceanDark,
    #[strum(serialize = "base16_ocean_light")]
    Base16OceanLight,
}

impl Theme {
    pub fn to_syntect_name(self) -> Option<&'static str> {
        match self {
            Theme::None => None,
            Theme::InspiredGitHub => Some("InspiredGitHub"),
            Theme::SolarizedDark => Some("Solarized (dark)"),
            Theme::SolarizedLight => Some("Solarized (light)"),
            Theme::Base16EightiesDark => Some("base16-eighties.dark"),
            Theme::Base16MochaDark => Some("base16-mocha.dark"),
            Theme::Base16OceanDark => Some("base16-ocean.dark"),
            Theme::Base16OceanLight => Some("base16-ocean.light"),
        }
    }
}

/// Application user interface config.
#[derive(Debug)]
pub struct UIConfig {
    /// Theme for visualizing program data and source codes.
    pub theme: Theme,
    /// Keymap for TUI.
    pub tui_keymap: KeyMap,
    /// Save command history in a regular file.
    pub save_history: bool,
}

/// Read-only ui configuration (set only once, at debugger start).
static CONFIG: OnceLock<UIConfig> = OnceLock::new();

/// Set initial configuration.
pub fn set(config: UIConfig) {
    CONFIG.set(config).expect("should called once");
}

/// Return application ui config.
pub fn current() -> &'static UIConfig {
    CONFIG.get().expect("should already be set")
}
