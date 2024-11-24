use crate::debugger::register::debug::BreakCondition;
use crate::ui;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::{SpecialAction, WrappedKeyEvent};
use crate::ui::tui::{Id, Msg};
use itertools::Itertools;
use nix::sys::signal::Signal;
use tui_realm_stdlib::Container;
use tuirealm::command::{Cmd, CmdResult};
use tuirealm::props::{Borders, Layout, PropPayload, PropValue, TextSpan};
use tuirealm::ratatui::layout::{Alignment, Rect};
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::widgets::BorderType;
use tuirealm::{
    AttrValue, Attribute, Component, Event, Frame, MockComponent, State, Sub, SubClause,
    SubEventClause,
};

pub struct Status {
    component: Container,
}

impl Status {
    pub fn new(app_already_run: bool) -> Self {
        let initial_state = if app_already_run {
            TextSpan::new("stopped").fg(Color::Red)
        } else {
            TextSpan::new("not running").fg(Color::Red)
        };

        let app_state = tui_realm_stdlib::Paragraph::default()
            .text(&[initial_state])
            .alignment(Alignment::Center)
            .title("Process", Alignment::Center)
            .borders(
                Borders::default()
                    .color(Color::White)
                    .modifiers(BorderType::Rounded),
            );

        let keymap = &ui::config::current().tui_keymap;
        let render_keys = |action: SpecialAction| -> String {
            keymap
                .keys_for_special_action(action)
                .into_iter()
                .map(|key| WrappedKeyEvent(*key).to_string())
                .join(", ")
        };

        let keymap_help = format!(
            "<{} / {}> expand left/right window | <{}> step out | <{}> step | <{}> step over | <{}> continue | <{}> start/restart | <{}> go to console | <{}> quit",
            render_keys(SpecialAction::ExpandLeftWindow),
            render_keys(SpecialAction::ExpandRightWindow),
            render_keys(SpecialAction::StepOut),
            render_keys(SpecialAction::StepInto),
            render_keys(SpecialAction::StepOver),
            render_keys(SpecialAction::ContinueDebugee),
            render_keys(SpecialAction::RunDebugee),
            render_keys(SpecialAction::SwitchUI),
            render_keys(SpecialAction::CloseApp),
        );

        let help = tui_realm_stdlib::Paragraph::default()
            .text(&[TextSpan::new(keymap_help).fg(Color::Green).bold()])
            .alignment(Alignment::Left)
            .title("Help", Alignment::Center)
            .borders(
                Borders::default()
                    .color(Color::White)
                    .modifiers(BorderType::Rounded),
            );

        Self {
            component: Container::default()
                .layout(
                    Layout::default()
                        .direction(tuirealm::ratatui::layout::Direction::Horizontal)
                        .constraints(
                            [
                                tuirealm::ratatui::layout::Constraint::Percentage(80),
                                tuirealm::ratatui::layout::Constraint::Percentage(20),
                            ]
                            .as_ref(),
                        ),
                )
                .children(vec![Box::new(help), Box::new(app_state)]),
        }
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
                SubEventClause::User(UserEvent::Watchpoint {
                    pc: Default::default(),
                    num: 0,
                    file: None,
                    line: None,
                    cond: BreakCondition::DataReadsWrites,
                    old_value: None,
                    new_value: None,
                    end_of_scope: false,
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
}

impl MockComponent for Status {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        self.component.view(frame, area)
    }

    fn query(&self, attr: Attribute) -> Option<AttrValue> {
        self.component.query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.component.children[1].attr(attr, value)
    }

    fn state(&self) -> State {
        self.component.state()
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        self.component.perform(cmd)
    }
}

impl Component<Msg, UserEvent> for Status {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        let mut set_text_fn = |text: &str| {
            self.attr(
                Attribute::Text,
                AttrValue::Payload(PropPayload::Vec(vec![PropValue::TextSpan(
                    TextSpan::new(text).fg(Color::Red),
                )])),
            )
        };

        match ev {
            Event::User(user_event) => match user_event {
                UserEvent::Breakpoint { num, .. } => {
                    set_text_fn(&format!("stopped at breakpoint #{num}"));
                    Some(Msg::None)
                }
                UserEvent::Watchpoint {
                    num, end_of_scope, ..
                } => {
                    set_text_fn(&format!("stopped at watchpoint #{num}"));
                    end_of_scope
                        .then_some(Msg::UpdateBreakpointList)
                        .or(Some(Msg::None))
                }
                UserEvent::Step { .. } => {
                    set_text_fn("stopped");
                    Some(Msg::None)
                }
                UserEvent::Signal(_) => {
                    set_text_fn("stopped at signal");
                    Some(Msg::None)
                }
                UserEvent::Exit(_) => {
                    set_text_fn("finished");
                    Some(Msg::None)
                }
                _ => None,
            },
            _ => None,
        }
    }
}
