use crate::debugger::{BreakpointViewOwned, WatchpointViewOwned};
use crate::ui;
use crate::ui::command;
use crate::ui::command::r#break::Command as BreakpointCommand;
use crate::ui::command::r#break::ExecutionResult;
use crate::ui::command::watch::Command as WatchpointCommand;
use crate::ui::command::watch::ExecutionResult as WatchpointExecutionResult;
use crate::ui::short::Abbreviator;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{BreakpointsAddType, Msg};
use std::collections::HashMap;
use std::sync::Arc;
use tui_realm_stdlib::List;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
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
    row_to_watchpoint_map: HashMap<usize, WatchpointViewOwned>,
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

    // return a breakpoint or watchpoint index if breakpoint/watchpoint is select, panics elsewhere.
    fn state(&self) -> State {
        let list_idx = self.component.state().unwrap_one().unwrap_usize();
        let number = if let Some(brkpt) = self.row_to_brkpt_map.get(&list_idx) {
            brkpt.number
        } else {
            self.row_to_watchpoint_map[&list_idx].number
        };

        State::One(StateValue::U32(number))
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.component.perform(cmd)
    }
}

impl Breakpoints {
    /// Update a breakpoint list. Triggered by custom attribute "update_breakpoints".
    pub fn update_list(&mut self) {
        let skip = if self.state == Some(AddState::SelectType) {
            // skip the first 4 rows because it is an added buttons
            5
        } else {
            // skip zero row because it is an add button
            1
        };

        let Ok(breakpoints) = self.exchanger.request_sync(|dbg| {
            let mut cmd = command::r#break::Handler::new(dbg);
            let brkpt_result = cmd.handle(&BreakpointCommand::Info).expect("infallible");
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
        self.row_to_brkpt_map = breakpoints
            .iter()
            .enumerate()
            .map(|(idx, brkpt)| (idx + skip, brkpt.clone()))
            .collect();

        let Ok(watchpoints) = self.exchanger.request_sync(|dbg| {
            let mut cmd = command::watch::Handler::new(dbg);
            let pw_result = cmd.handle(WatchpointCommand::Info).expect("infallible");
            let WatchpointExecutionResult::Dump(watchpoints) = pw_result else {
                unreachable!()
            };

            watchpoints
                .into_iter()
                .map(|snap| snap.to_owned())
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        self.row_to_watchpoint_map = watchpoints
            .iter()
            .enumerate()
            .map(|(idx, wp)| (idx + breakpoints.len() + skip, wp.clone()))
            .collect();

        let mut table_builder = TableBuilder::default();
        table_builder.add_col(TextSpan::from(" "));
        table_builder.add_col(TextSpan::from(" "));
        table_builder.add_col(TextSpan::from("NEW").fg(Color::Green).bold());
        table_builder.add_row();

        if self.state == Some(AddState::SelectType) {
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   at file:line").fg(Color::Green).bold());
            table_builder.add_row();
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   at function").fg(Color::Green).bold());
            table_builder.add_row();
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   at address").fg(Color::Green).bold());
            table_builder.add_row();
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("   watchpoint").fg(Color::Green).bold());
            table_builder.add_row();
        }

        let abbreviator = Abbreviator::new("/", "/..", 50);

        for brkpt in breakpoints.iter() {
            table_builder.add_col(TextSpan::from(brkpt.number.to_string()).fg(Color::Cyan));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("B").fg(Color::LightGreen));
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

        for wp in watchpoints.iter() {
            table_builder.add_col(TextSpan::from(wp.number.to_string()).fg(Color::Cyan));
            table_builder.add_col(TextSpan::from(" "));
            table_builder.add_col(TextSpan::from("W").fg(Color::LightBlue));
            table_builder.add_col(TextSpan::from(" "));
            if let Some(ref dqe_string) = wp.source_dqe {
                table_builder.add_col(TextSpan::from(format!("{} ({})", dqe_string, wp.condition)));
            } else {
                table_builder.add_col(TextSpan::from(format!(
                    "{}:{} ({})",
                    wp.address, wp.size, wp.condition
                )));
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
            row_to_watchpoint_map: HashMap::default(),
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
        if let Event::Keyboard(key_event) = ev {
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
                    CommonAction::Submit => {
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
                                        return Some(Msg::BreakpointAdd(
                                            BreakpointsAddType::AtLine,
                                        ));
                                    }
                                    2 => {
                                        return Some(Msg::BreakpointAdd(
                                            BreakpointsAddType::AtFunction,
                                        ));
                                    }
                                    3 => {
                                        return Some(Msg::BreakpointAdd(
                                            BreakpointsAddType::AtAddress,
                                        ));
                                    }
                                    4 => {
                                        return Some(Msg::BreakpointAdd(
                                            BreakpointsAddType::Watchpoint,
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }

                        if let Some(brkpt) = self.row_to_brkpt_map.get(&idx) {
                            return Some(Msg::PopupBreakpoint(brkpt.clone()));
                        }
                        if let Some(wp) = self.row_to_watchpoint_map.get(&idx) {
                            return Some(Msg::PopupWatchpoint(wp.clone()));
                        }
                    }
                    _ => {}
                }
            }
        };
        Some(Msg::None)
    }
}
