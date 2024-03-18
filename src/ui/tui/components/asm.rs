use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::utils::mstextarea::MultiSpanTextarea;
use crate::ui::tui::{Id, Msg};
use std::sync::Arc;
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Borders, Style, TextSpan};
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::prelude::Color;
use tuirealm::tui::widgets::BorderType;
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(MockComponent)]
pub struct Asm {
    component: MultiSpanTextarea,
    exchanger: Arc<ClientExchanger>,
}

impl Asm {
    pub fn new(exchanger: Arc<ClientExchanger>) -> anyhow::Result<Self> {
        let component = MultiSpanTextarea::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightYellow),
            )
            .inactive(Style::default().fg(Color::Gray))
            .title("Assembler code for function", Alignment::Center)
            .step(4)
            .highlighted_str("▶");

        let mut this = Self {
            component,
            exchanger,
        };

        this.update_asm_view();

        Ok(this)
    }

    fn update_asm_view(&mut self) {
        let Ok(asm) = self.exchanger.request_sync(|dbg| dbg.disasm()) else {
            return;
        };

        if let Ok(asm) = asm {
            if let Some(ref fn_name) = asm.name {
                self.component.attr(
                    Attribute::Title,
                    AttrValue::Title((
                        format!("Assembler code for function ({fn_name})"),
                        Alignment::Center,
                    )),
                );
            }

            let mut line_in_focus = None;
            let mut lines = vec![];
            for instr in asm.instructions.into_iter() {
                let addr_span = TextSpan::new(format!("{} ", instr.address)).fg(Color::Blue);
                let mnemonic_span =
                    TextSpan::new(format!("{} ", instr.mnemonic.as_deref().unwrap_or("???")))
                        .fg(Color::Red);
                let operands_span =
                    TextSpan::new(instr.operands.as_deref().unwrap_or("???")).fg(Color::Green);

                let mut line = vec![addr_span, mnemonic_span, operands_span];

                if asm.addr_in_focus == instr.address {
                    line_in_focus = Some(lines.len());
                    line.iter_mut().for_each(|text| text.fg = Color::LightRed)
                }

                lines.push(line);
            }

            self.component.text_rows(lines);

            if let Some(line) = line_in_focus {
                self.component.states.list_index = line;
            }
        }
    }

    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![
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
        ]
    }
}

impl Component<Msg, UserEvent> for Asm {
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
            Event::User(UserEvent::Breakpoint { .. }) | Event::User(UserEvent::Step { .. }) => {
                self.update_asm_view();
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
