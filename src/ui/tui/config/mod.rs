use crate::ui::tui::config::ser::KeyMapConfig;
use crate::{muted_error, weak_error};
use log::error;
use std::collections::HashMap;
use std::fs::read_to_string;
use tuirealm::event::KeyEvent;

mod ser;
pub(super) use ser::WrappedKeyEvent;

/// Common control actions (like up/down/scroll up/etc.)
#[derive(PartialEq, Clone, Copy, Hash, Debug)]
pub enum CommonAction {
    Up,
    Down,
    ScrollUp,
    ScrollDown,
    GotoBegin,
    GotoEnd,
    Submit,
    Left,
    Right,
    Delete,
    Backspace,
    Cancel,
}

/// Specialized debugger actions (like start/quit/steps/etc.)
#[derive(PartialEq, Clone, Copy, Hash, Debug)]
pub enum SpecialAction {
    SwitchWindowTab,
    ExpandLeftWindow,
    ExpandRightWindow,
    FocusLeftWindow,
    FocusRightWindow,
    SwitchUI,
    CloseApp,
    ContinueDebugee,
    RunDebugee,
    StepOver,
    StepInto,
    StepOut,
}

/// Configuration of key bindings for TUI.
#[derive(Debug)]
pub struct KeyMap {
    common_keys: HashMap<KeyEvent, CommonAction>,
    spec_keys: HashMap<KeyEvent, SpecialAction>,
}

impl Default for KeyMap {
    fn default() -> Self {
        let default_config = include_str!("preset/keymap.toml");
        let keybindings: KeyMapConfig = toml::de::from_str(default_config).expect("should de");
        keybindings.into()
    }
}

impl KeyMap {
    const DEFAULT_PATH: &'static str = ".config/bs/keymap.toml";

    /// Load keymap from file. Return [`None`] on errors.
    pub fn from_file(path: Option<&str>) -> Option<Self> {
        let data = match path {
            None => {
                let path = home::home_dir()?;
                let path = path.join(Self::DEFAULT_PATH);
                muted_error!(read_to_string(path))?
            }
            Some(path) => match read_to_string(path) {
                Ok(data) => data,
                Err(err) => {
                    error!("Error while load keymap file: {err}");
                    return None;
                }
            },
        };

        let bindings: KeyMapConfig = weak_error!(toml::de::from_str(&data))?;
        Some(bindings.into())
    }

    /// Get common action suitable for incoming key event.
    pub fn get_common(&self, key: &KeyEvent) -> Option<CommonAction> {
        self.common_keys.get(key).copied()
    }

    /// Get common special suitable for incoming key event.
    pub fn get_special(&self, key: &KeyEvent) -> Option<SpecialAction> {
        self.spec_keys.get(key).copied()
    }

    /// Return all possible key events for common action.
    pub fn keys_for_common_action(&self, act: CommonAction) -> Vec<&KeyEvent> {
        self.common_keys
            .iter()
            .filter_map(|(k, v)| if v == &act { Some(k) } else { None })
            .collect()
    }

    /// Return all possible key events for special action.
    pub fn keys_for_special_action(&self, act: SpecialAction) -> Vec<&KeyEvent> {
        self.spec_keys
            .iter()
            .filter_map(|(k, v)| if v == &act { Some(k) } else { None })
            .collect()
    }
}
