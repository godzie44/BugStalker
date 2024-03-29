use crate::debugger::BreakpointViewOwned;
use crate::ui::command;
use crate::ui::command::r#break::Command as BreakpointCommand;
use crate::ui::command::r#break::ExecutionResult;
use crate::ui::short::Abbreviator;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{BreakpointsAddType, Msg};
use std::collections::HashMap;
use std::sync::Arc;
use tui_realm_stdlib::List;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{BorderType, Borders, Style, TableBuilder, TextSpan};
use tuirealm::tui::layout::{Alignment, Rect};
use tuirealm::tui::style::Color;
use tuirealm::{AttrValue, Attribute, Component, Event, Frame, MockComponent, State, StateValue};

#[derive(PartialEq)]
enum AddState {
    SelectType,
}

pub struct Breakpoints {
    state: Option<AddState>,
    component: List,
    row_to_brkpt_map: HashMap<usize, BreakpointViewOwned>,
    exchanger: Arc<ClientExchanger>,
}

impl MockComponent for Breakpoints {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        self.component.view(frame, area)
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.component.query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if matches!(attr, Attribute::Custom("update_breakpoints")) {
            return self.update_list();
        }

        self.component.attr(attr, value)
    }

    // return a breakpoint index if breakpoint is select, panics elsewhere.
    fn state(&self) -> State {
        let list_idx = self.component.state().unwrap_one().unwrap_usize();
        State::One(StateValue::U32(self.row_to_brkpt_map[&list_idx].number))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.component.perform(cmd)
    }
}

impl Breakpoints {
    /// Update breakpoint list. Triggered by custom attribute "update_breakpoints".
    pub fn update_list(&mut self) {
        let Ok(breakpoints) = self.exchanger.request_sync(|dbg| {
            let mut cmd = command::r#break::Handler::new(dbg);
            let brkpt_result = cmd.handle(&BreakpointCommand::Info).expect("unreachable");
            let ExecutionResult::Dump(breakpoints) = brkpt_result else {
                unreachable!()
            };

            breakpoints
                .into_iter()
                .map(|snap| snap.to_owned())
                .collect::<Vec<_>>()
        }) else {
            return;
        };

        let skip = if self.state == Some(AddState::SelectType) {
            // skip the first 4 rows because it is an added buttons
            4
        } else {
            // skip zero row because it is an add button
            1
        };
        self.row_to_brkpt_map = breakpoints
            .iter()
            .enumerate()
            .map(|(idx, brkpt)| (idx + skip, brkpt.clone()))
            .collect();

        let mut table_builder = TableBuilder::default();
        table_builder.add_col(TextSpan::from(" "));
        table_builder.add_col(TextSpan::from(" "));
        table_builder.add_col(TextSpan::from("NEW").fg(Color::Green).bold());
        table_builder.add_row();

        if self.state == Some(AddState::SelectType) {
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   at file:line").fg(Color::Green).bold());
            table_builder.add_row();
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   at function").fg(Color::Green).bold());
            table_builder.add_row();
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   at address").fg(Color::Green).bold());
            table_builder.add_row();
        }

        let abbreviator = Abbreviator::new("/", "/..", 50);

        for brkpt in breakpoints.iter() {
            table_builder.add_col(TextSpan::from(brkpt.number.to_string()).fg(Color::Cyan));
            table_builder.add_col(TextSpan::from(" "));
            if let Some(ref place) = brkpt.place {
                let breakpoint_path =
                    format!("{}:{}", place.file.to_string_lossy(), place.line_number);
                let breakpoint_path = abbreviator.apply(&breakpoint_path);
                table_builder.add_col(TextSpan::from(breakpoint_path));
            } else {
                table_builder.add_col(TextSpan::from(format!("{}", brkpt.number)));
            }
            table_builder.add_row();
        }

        let mut table = table_builder.build();
        // remove last unused row
        table.pop();

        self.component
            .attr(Attribute::Content, AttrValue::Table(table));
    }
}

impl Breakpoints {
    pub fn new(exchanger: Arc<ClientExchanger>) -> Self {
        let list = List::default()
            .borders(
                Borders::default()
                    .modifiers(BorderType::Rounded)
                    .color(Color::LightYellow),
            )
            .title("Breakpoints", Alignment::Center)
            .scroll(true)
            .inactive(Style::default().fg(Color::Gray))
            .highlighted_color(Color::LightYellow)
            .highlighted_str("▶")
            .rewind(true)
            .step(4);

        let mut brkpts = Self {
            state: None,
            component: list,
            row_to_brkpt_map: HashMap::default(),
            exchanger,
        };
        brkpts.attr(
            Attribute::Custom("update_breakpoints"),
            AttrValue::Flag(true),
        );

        brkpts
    }
}

impl Component<Msg, UserEvent> for Breakpoints {
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
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                let idx = self.component.state().unwrap_one().unwrap_usize();

                match self.state {
                    None => {
                        if idx == 0 {
                            self.state = Some(AddState::SelectType);
                            self.attr(
                                Attribute::Custom("update_breakpoints"),
                                AttrValue::Flag(true),
                            );
                            return Some(Msg::None);
                        }
                    }
                    Some(AddState::SelectType) => {
                        if !self.exchanger.is_messaging_enabled() {
                            return Some(Msg::None);
                        }

                        self.state = None;
                        match idx {
                            0 => {
                                self.attr(
                                    Attribute::Custom("update_breakpoints"),
                                    AttrValue::Flag(true),
                                );
                                return Some(Msg::None);
                            }
                            1 => {
                                return Some(Msg::BreakpointAdd(BreakpointsAddType::AtLine));
                            }
                            2 => {
                                return Some(Msg::BreakpointAdd(BreakpointsAddType::AtFunction));
                            }
                            3 => {
                                return Some(Msg::BreakpointAdd(BreakpointsAddType::AtAddress));
                            }
                            _ => {}
                        }
                    }
                }

                if let Some(brkpt) = self.row_to_brkpt_map.get(&idx) {
                    return Some(Msg::PopupBreakpoint(brkpt.clone()));
                }
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
