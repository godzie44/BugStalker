use crate::ui;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::Msg;
use strum_macros::{Display, EnumString};
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Borders, InputType};
use tuirealm::ratatui::layout::Alignment;
use tuirealm::ratatui::style::{Color, Style};
use tuirealm::ratatui::widgets::BorderType;
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
        if let Event::Keyboard(key_event) = ev {
            let keymap = &ui::config::current().tui_keymap;
            if let Some(action) = keymap.get_common(&key_event) {
                match action {
                    CommonAction::Left => {
                        self.perform(Cmd::Move(Direction::Left));
                        return Some(Msg::None);
                    }
                    CommonAction::Right => {
                        self.perform(Cmd::Move(Direction::Right));
                        return Some(Msg::None);
                    }
                    CommonAction::GotoBegin => {
                        self.perform(Cmd::GoTo(Position::Begin));
                        return Some(Msg::None);
                    }
                    CommonAction::GotoEnd => {
                        self.perform(Cmd::GoTo(Position::End));
                        return Some(Msg::None);
                    }
                    CommonAction::Delete => {
                        self.perform(Cmd::Cancel);
                        return Some(Msg::None);
                    }
                    CommonAction::Backspace => {
                        self.perform(Cmd::Delete);
                        return Some(Msg::None);
                    }
                    CommonAction::Submit => {
                        let state = self.perform(Cmd::Submit);
                        if let CmdResult::Submit(State::One(StateValue::String(input))) = state {
                            return Some(Msg::Input(input));
                        };
                        return Some(Msg::None);
                    }
                    CommonAction::Cancel => return Some(Msg::InputCancel),
                    _ => {}
                }
            }

            if let KeyEvent {
                code: Key::Char(ch),
                ..
            } = key_event
            {
                self.perform(Cmd::Type(ch));
            }
        };
        Some(Msg::None)
    }
}
