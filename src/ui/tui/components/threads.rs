use crate::debugger::ThreadSnapshot;
use crate::ui::command;
use crate::ui::command::thread::ExecutionResult as ThreadResult;
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
pub struct Threads {
    component: List,
    exchanger: Arc<ClientExchanger>,
}

impl Threads {
    fn update_threads(&mut self) {
        let threads = self.exchanger.request_sync(|dbg| {
            let thread_result = command::thread::Handler::new(dbg)
                .handle(command::thread::Command::Info)
                .unwrap_or(ThreadResult::List(vec![]));

            let ThreadResult::List(threads) = thread_result else {
                unreachable!()
            };

            threads
        });

        let mut table_builder = TableBuilder::default();
        for thread_info in threads.iter() {
            table_builder.add_col(
                TextSpan::from(thread_info.thread.number.to_string())
                    .fg(Color::Cyan)
                    .italic(),
            );

            fn make_span(text: String, t_info: &ThreadSnapshot) -> TextSpan {
                if t_info.in_focus {
                    TextSpan::from(text).bold()
                } else {
                    TextSpan::from(text)
                }
            }

            table_builder.add_col(make_span(
                format!(" [{}] ", thread_info.thread.pid),
                thread_info,
            ));
            let func_name = thread_info
                .bt
                .as_ref()
                .and_then(|bt| bt[0].func_name.clone())
                .unwrap_or("unknown".to_string());
            let line = thread_info
                .place
                .as_ref()
                .map(|l| l.line_number.to_string())
                .unwrap_or("???".to_string());
            table_builder.add_col(make_span(format!("{func_name}(:{line})"), thread_info));

            table_builder.add_row();
        }

        self.attr(Attribute::Content, AttrValue::Table(table_builder.build()))
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
            .title("Trace", Alignment::Center)
            .scroll(true)
            .highlighted_color(Color::LightYellow)
            .highlighted_str("🚀")
            .rewind(true)
            .step(4);

        let mut this = Self {
            component: list,
            exchanger,
        };
        this.update_threads();
        this
    }
}

impl Component<Msg, UserEvent> for Threads {
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
                self.update_threads();
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
