use crate::ui;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::output::OutputLine;
use crate::ui::tui::{Id, Msg};
use tui_realm_stdlib::Textarea;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::props::{Borders, PropPayload, PropValue, Style, TextSpan};
use tuirealm::ratatui::layout::Alignment;
use tuirealm::ratatui::prelude::Color;
use tuirealm::ratatui::widgets::BorderType;
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
                        .color(Color::LightYellow),
                )
                .inactive(Style::default().fg(Color::Gray))
                .title("Program output", Alignment::Center)
                .highlighted_str("▶")
                .step(4)
                .text_rows(rows),
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
            Event::Keyboard(key_event) => {
                let keymap = &ui::config::current().tui_keymap;
                if let Some(action) = keymap.get_common(&key_event) {
                    match action {
                        CommonAction::Up => {
                            self.perform(Cmd::Move(Direction::Up));
                        }
                        CommonAction::Down => {
                            self.perform(Cmd::Move(Direction::Down));
                        }
                        CommonAction::ScrollUp => {
                            self.perform(Cmd::Scroll(Direction::Up));
                        }
                        CommonAction::ScrollDown => {
                            self.perform(Cmd::Scroll(Direction::Down));
                        }
                        CommonAction::GotoBegin => {
                            self.perform(Cmd::GoTo(Position::Begin));
                        }
                        CommonAction::GotoEnd => {
                            self.perform(Cmd::GoTo(Position::End));
                        }
                        _ => {}
                    }
                }
                CmdResult::None
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
                self.component.states.list_index_at_last();

                CmdResult::None
            }
            _ => CmdResult::None,
        };
        Some(Msg::None)
    }
}
