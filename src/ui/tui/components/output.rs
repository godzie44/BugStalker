use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::output::OutputLine;
use crate::ui::tui::{Id, Msg};
use ratatui::layout::Alignment;
use ratatui::prelude::Color;
use ratatui::widgets::BorderType;
use tui_realm_stdlib::Textarea;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Borders, PropPayload, PropValue, TextSpan};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(MockComponent)]
pub struct Output {
    component: Textarea,
}

impl Output {
    pub fn new(output: &[OutputLine]) -> Self {
        let rows: Vec<_> = output
            .iter()
            .map(|line| match line {
                OutputLine::Out(text) => TextSpan::new(text),
                OutputLine::Err(err_text) => TextSpan::new(err_text).fg(Color::LightRed),
            })
            .collect();

        Self {
            component: Textarea::default()
                .borders(
                    Borders::default()
                        .modifiers(BorderType::Rounded)
                        .color(Color::LightBlue),
                )
                .foreground(Color::LightBlue)
                .title("Program output", Alignment::Center)
                .step(4)
                .text_rows(&rows),
        }
    }

    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![Sub::new(
            SubEventClause::User(UserEvent::GotOutput(vec![], 0)),
            SubClause::Always,
        )]
    }
}

impl Component<Msg, UserEvent> for Output {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        let _ = match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Down, ..
            }) => self.perform(Cmd::Move(Direction::Down)),
            Event::Keyboard(KeyEvent { code: Key::Up, .. }) => {
                self.perform(Cmd::Move(Direction::Up))
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageDown,
                ..
            }) => self.perform(Cmd::Scroll(Direction::Down)),
            Event::Keyboard(KeyEvent {
                code: Key::PageUp, ..
            }) => self.perform(Cmd::Scroll(Direction::Up)),
            Event::Keyboard(KeyEvent {
                code: Key::Home, ..
            }) => self.perform(Cmd::GoTo(Position::Begin)),
            Event::Keyboard(KeyEvent { code: Key::End, .. }) => {
                self.perform(Cmd::GoTo(Position::End))
            }
            Event::User(UserEvent::GotOutput(output, _)) => {
                let rows: Vec<_> = output
                    .into_iter()
                    .map(|line| match line {
                        OutputLine::Out(text) => TextSpan::new(text),
                        OutputLine::Err(err_text) => TextSpan::new(err_text).fg(Color::LightRed),
                    })
                    .collect();

                self.component.attr(
                    Attribute::Text,
                    AttrValue::Payload(PropPayload::Vec(
                        rows.iter().cloned().map(PropValue::TextSpan).collect(),
                    )),
                );

                CmdResult::None
            }
            _ => CmdResult::None,
        };
        Some(Msg::None)
    }
}
