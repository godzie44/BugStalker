use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use ratatui::layout::Alignment;
use ratatui::style::Color;
use ratatui::widgets::BorderType;
use tui_realm_stdlib::Radio;
use tuirealm::command::{Cmd, CmdResult, Direction};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::Borders;
use tuirealm::{Component, Event, MockComponent, State, StateValue};

#[derive(MockComponent)]
pub struct LeftTab {
    component: Radio,
}

impl Default for LeftTab {
    fn default() -> Self {
        Self {
            component: Radio::default()
                .borders(
                    Borders::default()
                        .modifiers(BorderType::Rounded)
                        .color(Color::LightGreen),
                )
                .foreground(Color::LightGreen)
                .title("Select your ice cream flavour 🍦", Alignment::Center)
                .rewind(true)
                .choices(&["Breakpoints", "Variables", "Threads"]),
        }
    }
}

impl Component<Msg, UserEvent> for LeftTab {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Left, ..
            }) => self.perform(Cmd::Move(Direction::Left)),
            Event::Keyboard(KeyEvent {
                code: Key::Right, ..
            }) => self.perform(Cmd::Move(Direction::Right)),
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                let res = self.perform(Cmd::Submit);
                if let CmdResult::Submit(State::One(StateValue::Usize(idx))) = res {
                    match idx {
                        0 => return Some(Msg::BreakpointsInFocus),
                        1 => return Some(Msg::VariablesInFocus),
                        2 => return Some(Msg::ThreadsInFocus),
                        _ => unreachable!(),
                    }
                }
                CmdResult::None
            }
            _ => CmdResult::None,
        };

        Some(Msg::None)
    }
}

#[derive(MockComponent)]
pub struct RightTab {
    component: Radio,
}

impl Default for RightTab {
    fn default() -> Self {
        Self {
            component: Radio::default()
                .borders(
                    Borders::default()
                        .modifiers(BorderType::Rounded)
                        .color(Color::LightGreen),
                )
                .foreground(Color::LightGreen)
                .title("Select your ice cream flavour 🍦", Alignment::Center)
                .rewind(true)
                .choices(&["Source", "Output", "Logs"]),
        }
    }
}

impl Component<Msg, UserEvent> for RightTab {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Left, ..
            }) => self.perform(Cmd::Move(Direction::Left)),
            Event::Keyboard(KeyEvent {
                code: Key::Right, ..
            }) => self.perform(Cmd::Move(Direction::Right)),
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                let res = self.perform(Cmd::Submit);
                if let CmdResult::Submit(State::One(StateValue::Usize(idx))) = res {
                    match idx {
                        0 => return Some(Msg::SourceInFocus),
                        1 => return Some(Msg::OutputInFocus),
                        2 => return Some(Msg::LogsInFocus),
                        _ => unreachable!(),
                    }
                }
                CmdResult::None
            }
            _ => CmdResult::None,
        };

        Some(Msg::None)
    }
}
