use crate::ui;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::utils::mstextarea::MultiSpanTextarea;
use crate::ui::tui::{Id, Msg};
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::props::{Borders, Style, TextSpan};
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::prelude::Color;
use tuirealm::tui::widgets::BorderType;
use tuirealm::{Component, Event, MockComponent, Sub, SubClause, SubEventClause};

#[derive(MockComponent)]
pub struct Logs {
    component: MultiSpanTextarea,
    log_view: Vec<Vec<TextSpan>>,
}

impl Default for Logs {
    fn default() -> Self {
        Self {
            log_view: vec![],
            component: MultiSpanTextarea::default()
                .borders(
                    Borders::default()
                        .modifiers(BorderType::Rounded)
                        .color(Color::LightYellow),
                )
                .inactive(Style::default().fg(Color::Gray))
                .highlighted_str("▶")
                .title("Debugger logs", Alignment::Center)
                .step(4),
        }
    }
}

impl Logs {
    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![Sub::new(
            SubEventClause::User(UserEvent::Logs(vec![])),
            SubClause::Always,
        )]
    }
}

impl Component<Msg, UserEvent> for Logs {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
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
            }
            Event::User(UserEvent::Logs(logs)) => {
                self.log_view
                    .extend(logs.into_iter().map(|l| l.to_text_spans()));
                self.component.text_rows(self.log_view.clone());
                self.component.states.list_index_at_last();
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
