use crate::ui::tui::config::{CommonAction, KeyMap, SpecialAction};
use anyhow::bail;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use tuirealm::event::{Key, KeyEvent, KeyModifiers};

fn parse_key_code(raw: &str, is_upper: bool) -> anyhow::Result<Key> {
    let code = match raw {
        "esc" => Key::Esc,
        "enter" => Key::Enter,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "backtab" => Key::BackTab,
        "backspace" => Key::Backspace,
        "del" => Key::Delete,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "ins" => Key::Insert,
        "f1" => Key::Function(1),
        "f2" => Key::Function(2),
        "f3" => Key::Function(3),
        "f4" => Key::Function(4),
        "f5" => Key::Function(5),
        "f6" => Key::Function(6),
        "f7" => Key::Function(7),
        "f8" => Key::Function(8),
        "f9" => Key::Function(9),
        "f10" => Key::Function(10),
        "f11" => Key::Function(11),
        "f12" => Key::Function(12),
        "space" => Key::Char(' '),
        "hyphen" => Key::Char('-'),
        "minus" => Key::Char('-'),
        "tab" => Key::Tab,
        c if c.len() == 1 => {
            let mut c = c.chars().next().expect("infallible");
            if is_upper {
                c = c.to_ascii_uppercase();
            }
            Key::Char(c)
        }
        _ => {
            bail!("Unknown key kode: {raw}");
        }
    };
    Ok(code)
}

fn parse(raw: &str) -> anyhow::Result<KeyEvent> {
    let mut modifiers = KeyModifiers::empty();
    let raw = raw.to_ascii_lowercase();
    let mut raw: &str = raw.as_ref();
    loop {
        if let Some(end) = raw.strip_prefix("ctrl-") {
            raw = end;
            modifiers.insert(KeyModifiers::CONTROL);
        } else if let Some(end) = raw.strip_prefix("alt-") {
            raw = end;
            modifiers.insert(KeyModifiers::ALT);
        } else if let Some(end) = raw.strip_prefix("shift-") {
            raw = end;
            modifiers.insert(KeyModifiers::SHIFT);
        } else {
            break;
        }
    }

    let is_upper = modifiers.contains(KeyModifiers::SHIFT);
    let code = parse_key_code(raw, is_upper)?;
    if code == Key::BackTab {
        // Crossterm always sends SHIFT with backtab
        modifiers.insert(KeyModifiers::SHIFT);
    }
    Ok(KeyEvent { code, modifiers })
}

#[derive(PartialEq, Debug)]
pub struct WrappedKeyEvent(pub KeyEvent);

impl Deref for WrappedKeyEvent {
    type Target = KeyEvent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for WrappedKeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            write!(f, "Ctrl-")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            write!(f, "Alt-")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            write!(f, "Shift-")?;
        }

        match self.code {
            Key::Char(' ') => {
                write!(f, "Space")?;
            }
            Key::Char('-') => {
                write!(f, "Hyphen")?;
            }
            Key::Char('\r') | Key::Char('\n') | Key::Enter => {
                write!(f, "Enter")?;
            }
            Key::Char(c) if self.modifiers.contains(KeyModifiers::SHIFT) => {
                write!(f, "{}", c.to_ascii_uppercase())?;
            }
            Key::Char(c) => {
                write!(f, "{}", c.to_ascii_lowercase())?;
            }
            Key::Function(u) => {
                write!(f, "F{u}")?;
            }
            _ => {
                write!(f, "{:?}", self.code)?;
            }
        }

        Ok(())
    }
}

impl Serialize for WrappedKeyEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for WrappedKeyEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(WrappedKeyEvent(parse(&s).map_err(de::Error::custom)?))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Common {
    up: Vec<WrappedKeyEvent>,
    down: Vec<WrappedKeyEvent>,
    scroll_down: Vec<WrappedKeyEvent>,
    scroll_up: Vec<WrappedKeyEvent>,
    goto_begin: Vec<WrappedKeyEvent>,
    goto_end: Vec<WrappedKeyEvent>,
    submit: Vec<WrappedKeyEvent>,
    cancel: Vec<WrappedKeyEvent>,
    left: Vec<WrappedKeyEvent>,
    right: Vec<WrappedKeyEvent>,
    input_delete: Vec<WrappedKeyEvent>,
    input_backspace: Vec<WrappedKeyEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Special {
    switch_window_tab: Vec<WrappedKeyEvent>,
    expand_left: Vec<WrappedKeyEvent>,
    expand_right: Vec<WrappedKeyEvent>,
    focus_left: Vec<WrappedKeyEvent>,
    focus_right: Vec<WrappedKeyEvent>,
    switch_ui: Vec<WrappedKeyEvent>,
    close_app: Vec<WrappedKeyEvent>,
    r#continue: Vec<WrappedKeyEvent>,
    run: Vec<WrappedKeyEvent>,
    step_over: Vec<WrappedKeyEvent>,
    step_into: Vec<WrappedKeyEvent>,
    step_out: Vec<WrappedKeyEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct KeyMapConfig {
    special: Special,
    common: Common,
}

impl From<KeyMapConfig> for KeyMap {
    fn from(bindings: KeyMapConfig) -> Self {
        fn append_key<A: Copy>(
            map: &mut HashMap<KeyEvent, A>,
            keys: Vec<WrappedKeyEvent>,
            action: A,
        ) {
            for key in keys {
                map.insert(key.0, action);
            }
        }

        let mut keymap = KeyMap {
            common_keys: Default::default(),
            spec_keys: Default::default(),
        };

        let common_k = &mut keymap.common_keys;
        let cb = bindings.common;
        append_key(common_k, cb.up, CommonAction::Up);
        append_key(common_k, cb.down, CommonAction::Down);
        append_key(common_k, cb.scroll_up, CommonAction::ScrollUp);
        append_key(common_k, cb.scroll_down, CommonAction::ScrollDown);
        append_key(common_k, cb.goto_begin, CommonAction::GotoBegin);
        append_key(common_k, cb.goto_end, CommonAction::GotoEnd);
        append_key(common_k, cb.submit, CommonAction::Submit);
        append_key(common_k, cb.left, CommonAction::Left);
        append_key(common_k, cb.right, CommonAction::Right);
        append_key(common_k, cb.input_delete, CommonAction::Delete);
        append_key(common_k, cb.input_backspace, CommonAction::Backspace);
        append_key(common_k, cb.cancel, CommonAction::Cancel);

        let spec_k = &mut keymap.spec_keys;
        let sb = bindings.special;
        append_key(spec_k, sb.switch_window_tab, SpecialAction::SwitchWindowTab);
        append_key(spec_k, sb.expand_left, SpecialAction::ExpandLeftWindow);
        append_key(spec_k, sb.expand_right, SpecialAction::ExpandRightWindow);
        append_key(spec_k, sb.focus_left, SpecialAction::FocusLeftWindow);
        append_key(spec_k, sb.focus_right, SpecialAction::FocusRightWindow);
        append_key(spec_k, sb.switch_ui, SpecialAction::SwitchUI);
        append_key(spec_k, sb.close_app, SpecialAction::CloseApp);
        append_key(spec_k, sb.r#continue, SpecialAction::ContinueDebugee);
        append_key(spec_k, sb.run, SpecialAction::RunDebugee);
        append_key(spec_k, sb.step_over, SpecialAction::StepOver);
        append_key(spec_k, sb.step_into, SpecialAction::StepInto);
        append_key(spec_k, sb.step_out, SpecialAction::StepOut);

        keymap
    }
}

#[cfg(test)]
mod test {
    use crate::ui::tui::config::WrappedKeyEvent;
    use serde::{Deserialize, Serialize};
    use tuirealm::event::{Key, KeyEvent, KeyModifiers};

    #[test]
    fn test_serde() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Bindings {
            keys: Vec<WrappedKeyEvent>,
        }
        let bindings = Bindings {
            keys: vec![
                WrappedKeyEvent(KeyEvent {
                    code: Key::Backspace,
                    modifiers: KeyModifiers::empty(),
                }),
                WrappedKeyEvent(KeyEvent {
                    code: Key::Esc,
                    modifiers: KeyModifiers::CONTROL,
                }),
                WrappedKeyEvent(KeyEvent {
                    code: Key::Char('A'),
                    modifiers: KeyModifiers::SHIFT,
                }),
                WrappedKeyEvent(KeyEvent {
                    code: Key::Char('a'),
                    modifiers: KeyModifiers::NONE,
                }),
                WrappedKeyEvent(KeyEvent {
                    code: Key::Function(1),
                    modifiers: KeyModifiers::ALT,
                }),
            ],
        };

        let ser = toml::ser::to_string(&bindings).unwrap();
        let de: Bindings = toml::de::from_str(&ser).unwrap();

        assert_eq!(bindings, de);
    }
}
