use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use strum_macros::{Display, EnumString};
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Borders, InputType};
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::style::{Color, Style};
use tuirealm::tui::widgets::BorderType;
use tuirealm::{Component, Event, MockComponent, State, StateValue};

#[derive(Debug, Display, EnumString)]
pub enum InputStringType {
    BreakpointAddAtLine,
    BreakpointAddAtFunction,
    BreakpointAddAtAddress,
    Watchpoint,
}

#[derive(MockComponent)]
pub struct Input {
    component: tui_realm_stdlib::Input,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            component: tui_realm_stdlib::Input::default()
                .borders(
                    Borders::default()
                        .modifiers(BorderType::Rounded)
                        .color(Color::LightYellow),
                )
                .foreground(Color::LightYellow)
                .input_type(InputType::Text)
                .title("", Alignment::Left)
                .value("")
                .invalid_style(Style::default().fg(Color::Red)),
        }
    }
}

impl Component<Msg, UserEvent> for Input {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        let _ = match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Left, ..
            }) => self.perform(Cmd::Move(Direction::Left)),
            Event::Keyboard(KeyEvent {
                code: Key::Right, ..
            }) => self.perform(Cmd::Move(Direction::Right)),
            Event::Keyboard(KeyEvent {
                code: Key::Home, ..
            }) => self.perform(Cmd::GoTo(Position::Begin)),
            Event::Keyboard(KeyEvent { code: Key::End, .. }) => {
                self.perform(Cmd::GoTo(Position::End))
            }
            Event::Keyboard(KeyEvent {
                code: Key::Delete, ..
            }) => self.perform(Cmd::Cancel),
            Event::Keyboard(KeyEvent {
                code: Key::Backspace,
                ..
            }) => self.perform(Cmd::Delete),
            Event::Keyboard(KeyEvent {
                code: Key::Char(ch),
                ..
            }) => self.perform(Cmd::Type(ch)),
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                let state = self.perform(Cmd::Submit);
                if let CmdResult::Submit(State::One(StateValue::String(input))) = state {
                    return Some(Msg::Input(input));
                }
                CmdResult::None
            }
            Event::Keyboard(KeyEvent { code: Key::Esc, .. }) => return Some(Msg::InputCancel),
            _ => CmdResult::None,
        };
        Some(Msg::None)
    }
}
