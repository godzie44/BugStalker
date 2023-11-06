use crate::debugger::command;
use crate::debugger::variable::render::{RenderRepr, ValueLayout};
use crate::debugger::variable::select;
use crate::debugger::variable::select::VariableSelector;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{Id, Msg};
use nix::sys::signal::Signal;
use ratatui::layout::Alignment;
use ratatui::style::Color;
use std::sync::Arc;
use tui_realm_stdlib::List;
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{TableBuilder, TextSpan};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(MockComponent)]
pub struct Variables {
    component: List,
    exchanger: Arc<ClientExchanger>,
}

impl Variables {
    fn update_variables(&mut self) {
        let variables = self.exchanger.request_sync(|dbg| {
            let expr = select::Expression::Variable(VariableSelector::Any);
            let vars = command::Variables::new(dbg)
                .handle(expr)
                .unwrap_or_default();
            vars
        });

        let mut table_builder = TableBuilder::default();
        for var in variables.iter() {
            let val_view = match var.value() {
                None => "unknown".to_string(),
                Some(layout) => match layout {
                    ValueLayout::PreRendered(view) => view.to_string(),
                    ValueLayout::Referential { addr, .. } => {
                        format!("{addr:p} (...)")
                    }
                    ValueLayout::Wrapped(_) => "(...)".to_string(),
                    ValueLayout::Nested { .. } => "(...)".to_string(),
                    ValueLayout::Map(_) => "(...)".to_string(),
                },
            };

            table_builder.add_col(TextSpan::from(var.name()).fg(Color::Cyan).italic());

            table_builder.add_col(
                TextSpan::from(format!(" {}({val_view})", var.r#type()))
                    .fg(Color::Cyan)
                    .italic(),
            );

            table_builder.add_row();
        }

        self.attr(Attribute::Content, AttrValue::Table(table_builder.build()));
    }

    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![
            Sub::new(
                // concrete signal doesn't meter
                SubEventClause::User(UserEvent::Signal(Signal::SIGUSR2)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::User(UserEvent::Breakpoint {
                    pc: Default::default(),
                    num: 0,
                    file: None,
                    line: None,
                    function: None,
                }),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::User(UserEvent::Step {
                    pc: Default::default(),
                    file: None,
                    line: None,
                    function: None,
                }),
                SubClause::Always,
            ),
            // concrete code doesn't meter
            Sub::new(SubEventClause::User(UserEvent::Exit(0)), SubClause::Always),
        ]
    }

    pub fn new(exchanger: Arc<ClientExchanger>) -> Self {
        let list = List::default()
            .title("Variables", Alignment::Center)
            .scroll(true)
            .highlighted_color(Color::LightYellow)
            .highlighted_str("🚀")
            .rewind(true)
            .step(4);

        let mut this = Self {
            component: list,
            exchanger,
        };
        this.update_variables();
        this
    }
}

impl Component<Msg, UserEvent> for Variables {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Down, ..
            }) => {
                self.perform(Cmd::Move(Direction::Down));
            }
            Event::Keyboard(KeyEvent { code: Key::Up, .. }) => {
                self.perform(Cmd::Move(Direction::Up));
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageDown,
                ..
            }) => {
                self.perform(Cmd::Scroll(Direction::Down));
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageUp, ..
            }) => {
                self.perform(Cmd::Scroll(Direction::Up));
            }
            Event::Keyboard(KeyEvent {
                code: Key::Home, ..
            }) => {
                self.perform(Cmd::GoTo(Position::Begin));
            }
            Event::Keyboard(KeyEvent { code: Key::End, .. }) => {
                self.perform(Cmd::GoTo(Position::End));
            }
            Event::User(_) => {
                self.update_variables();
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
