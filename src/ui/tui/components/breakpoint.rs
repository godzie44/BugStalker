use crate::ui::command;
use crate::ui::command::r#break::Command as BreakpointCommand;
use crate::ui::command::r#break::ExecutionResult;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::Msg;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Color;
use std::collections::HashMap;
use std::sync::Arc;
use tui_realm_stdlib::List;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{Table, TableBuilder, TextSpan};
use tuirealm::{AttrValue, Attribute, Component, Event, Frame, MockComponent, State, StateValue};

pub struct Breakpoints {
    component: List,
    row_to_brkpt_num_map: HashMap<usize, u32>,
}

impl MockComponent for Breakpoints {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        self.component.view(frame, area)
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.component.query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if matches!(attr, Attribute::Content) {
            let breakpoints_table = match value {
                AttrValue::Table(ref t) => t,
                _ => panic!("AttrValue is not Table"),
            };

            self.row_to_brkpt_num_map = breakpoints_table
                .iter()
                .map(|row| &row[0])
                .enumerate()
                // skip zero row cause iy is an add button
                .skip(1)
                .filter_map(|(idx, brkpt_num_col)| {
                    let num: u32 = brkpt_num_col.content.parse().ok()?;
                    Some((idx, num))
                })
                .collect();
        }

        self.component.attr(attr, value)
    }

    fn state(&self) -> State {
        let list_idx = self.component.state().unwrap_one().unwrap_usize();
        State::One(StateValue::U32(self.row_to_brkpt_num_map[&list_idx]))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.component.perform(cmd)
    }
}

impl Breakpoints {
    pub fn breakpoint_table(exchanger: Arc<ClientExchanger>) -> Table {
        let breakpoints = exchanger.request_sync(|dbg| {
            let mut cmd = command::r#break::Handler::new(dbg);
            let brkpt_result = cmd.handle(&BreakpointCommand::Info).expect("unreachable");
            let ExecutionResult::Dump(breakpoints) =  brkpt_result else {
                unreachable!()
            };

            breakpoints
                .into_iter()
                .map(|snap| snap.to_owned())
                .collect::<Vec<_>>()
        });

        let mut table_builder = TableBuilder::default();
        table_builder.add_col(TextSpan::from(" "));
        table_builder.add_col(TextSpan::from("🚀"));
        table_builder.add_col(TextSpan::from("add new").fg(Color::Green).bold());
        table_builder.add_row();

        for brkpt in breakpoints.iter() {
            table_builder.add_col(
                TextSpan::from(brkpt.number.to_string())
                    .fg(Color::Cyan)
                    .italic(),
            );
            table_builder.add_col(TextSpan::from(" "));
            if let Some(ref place) = brkpt.place {
                table_builder.add_col(TextSpan::from(format!(
                    "{:?}:{}",
                    place.file, place.line_number
                )));
            } else {
                table_builder.add_col(TextSpan::from(format!("{}", brkpt.number)));
            }
            table_builder.add_row();
        }
        let mut tbl = table_builder.build();
        // remove last unused row
        tbl.pop();
        tbl
    }
}

impl Breakpoints {
    pub fn new(exchanger: Arc<ClientExchanger>) -> Self {
        let list = List::default()
            .title("Breakpoints", Alignment::Center)
            .scroll(true)
            .highlighted_color(Color::LightYellow)
            .highlighted_str("✖")
            .rewind(true)
            .step(4);

        let mut brkpts = Self {
            component: list,
            row_to_brkpt_num_map: HashMap::default(),
        };
        brkpts.attr(
            Attribute::Content,
            AttrValue::Table(Self::breakpoint_table(exchanger)),
        );

        brkpts
    }
}

impl Component<Msg, UserEvent> for Breakpoints {
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
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                let idx = self.component.state().unwrap_one().unwrap_usize();
                if idx == 0 {
                    return Some(Msg::AddBreakpointRequest);
                }
                return Some(Msg::RemoveBreakpointRequest(
                    self.state().unwrap_one().unwrap_u32(),
                ));
            }
            _ => CmdResult::None,
        };
        Some(Msg::None)
    }
}
