pub mod port;

use std::sync::Arc;
use std::time::Duration;
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::style::Color;

use crate::ui::command;
use crate::ui::command::r#break;
use crate::ui::command::r#break::BreakpointIdentity;
use crate::ui::command::Command;
use crate::ui::tui::app::port::{
    AsyncResponsesPort, DebuggerEventQueue, DebuggerEventsPort, OutputPort, UserEvent,
};
use crate::ui::tui::components::alert::Alert;
use crate::ui::tui::components::breakpoint::Breakpoints;
use crate::ui::tui::components::control::GlobalControl;
use crate::ui::tui::components::input::Input;
use crate::ui::tui::components::output::Output;
use crate::ui::tui::components::source::Source;
use crate::ui::tui::components::status::Status;
use crate::ui::tui::components::stub::Stub;
use crate::ui::tui::components::tabs::{LeftTab, RightTab};
use crate::ui::tui::components::threads::Threads;
use crate::ui::tui::components::variables::Variables;
use crate::ui::tui::proto::ClientExchanger;
use tuirealm::props::{PropPayload, PropValue, TextSpan};
use tuirealm::terminal::TerminalBridge;
use tuirealm::tui::layout::{Constraint, Direction, Layout};
use tuirealm::{Application, AttrValue, Attribute, EventListenerCfg, State, StateValue};

use super::{DebugeeStreamBuffer, Id, Msg};

enum InputType {
    Breakpoint,
}

pub struct Model {
    /// Application
    pub app: Application<Id, Msg, UserEvent>,
    /// Indicates that the application must quit
    pub quit: bool,
    /// Tells whether to redraw interface
    pub redraw: bool,

    input_state: Option<InputType>,

    pub alert: bool,

    /// Used to draw to terminal
    pub terminal: TerminalBridge,

    exchanger: Arc<ClientExchanger>,
}

impl Model {
    pub fn new(
        output_buf: DebugeeStreamBuffer,
        event_queue: DebuggerEventQueue,
        client_exchanger: ClientExchanger,
    ) -> anyhow::Result<Self> {
        let exchanger = Arc::new(client_exchanger);
        Ok(Self {
            app: Self::init_app(output_buf, event_queue, exchanger.clone())?,
            quit: false,
            redraw: true,
            input_state: None,
            alert: false,
            terminal: TerminalBridge::new().expect("Cannot initialize terminal"),
            exchanger,
        })
    }
}

impl Model {
    pub fn view(&mut self) {
        _ = self.terminal.raw_mut().draw(|f| {
            let input_in_focus = self.app.focus() == Some(&Id::Input);

            let mut constraints = vec![
                Constraint::Max(3),
                Constraint::Length(9),
                Constraint::Max(3),
            ];
            if input_in_focus {
                constraints.push(Constraint::Max(3));
            }

            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(constraints)
                .split(f.size());

            let tabs_rect = main_chunks[0];
            let tab_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
                .split(tabs_rect);

            let window_rect = main_chunks[1];
            let window_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
                .split(window_rect);

            self.app.view(&Id::LeftTabs, f, tab_chunks[0]);
            self.app.view(&Id::RightTabs, f, tab_chunks[1]);

            match self.app.state(&Id::LeftTabs) {
                Ok(state) => {
                    if let State::One(StateValue::Usize(n)) = state {
                        let id = match n {
                            0 => Id::Breakpoints,
                            1 => Id::Variables,
                            2 => Id::Threads,
                            _ => unreachable!(),
                        };
                        self.app.view(&id, f, window_chunks[0]);
                    }
                }
                Err(_) => {
                    unreachable!()
                }
            }

            match self.app.state(&Id::RightTabs) {
                Ok(state) => {
                    if let State::One(StateValue::Usize(n)) = state {
                        let id = match n {
                            0 => Id::Source,
                            1 => Id::Output,
                            2 => Id::Logs,
                            _ => unreachable!(),
                        };
                        self.app.view(&id, f, window_chunks[1]);
                    }
                }
                Err(_) => {
                    unreachable!()
                }
            }

            if input_in_focus {
                self.app.view(&Id::Input, f, main_chunks[2]);
                self.app.view(&Id::Status, f, main_chunks[3]);
            } else {
                self.app.view(&Id::Status, f, main_chunks[2]);
            }

            if self.alert {
                self.app.view(&Id::Alert, f, f.size());
            }
        });
    }

    fn init_app(
        output_buf: DebugeeStreamBuffer,
        event_queue: DebuggerEventQueue,
        exchanger: Arc<ClientExchanger>,
    ) -> anyhow::Result<Application<Id, Msg, UserEvent>> {
        // NOTE: the event listener is configured to use the default crossterm input listener and to raise a Tick event each second
        // which we will use to update the clock

        let mut app: Application<Id, Msg, UserEvent> = Application::init(
            EventListenerCfg::default()
                .default_input_listener(Duration::from_millis(20))
                .port(
                    Box::new(OutputPort::new(output_buf.data.clone())),
                    Duration::from_millis(10),
                )
                .port(
                    Box::new(DebuggerEventsPort::new(event_queue)),
                    Duration::from_millis(10),
                )
                .port(
                    Box::new(AsyncResponsesPort::new(exchanger.clone())),
                    Duration::from_millis(10),
                )
                .poll_timeout(Duration::from_millis(10))
                .tick_interval(Duration::from_secs(1)),
        );

        app.mount(
            Id::GlobalControl,
            Box::new(GlobalControl::new(exchanger.clone())),
            GlobalControl::subscriptions(),
        )?;

        app.mount(Id::Alert, Box::<Alert>::default(), vec![])?;
        app.mount(Id::Input, Box::<Input>::default(), vec![])?;
        app.mount(
            Id::Status,
            Box::<Status>::default(),
            Status::subscriptions(),
        )?;
        app.mount(Id::LeftTabs, Box::<LeftTab>::default(), vec![])?;
        app.mount(Id::RightTabs, Box::<RightTab>::default(), vec![])?;

        app.mount(
            Id::Breakpoints,
            Box::new(Breakpoints::new(exchanger.clone())),
            vec![],
        )?;
        app.mount(
            Id::Variables,
            Box::new(Variables::new(exchanger.clone())),
            Variables::subscriptions(),
        )?;
        app.mount(
            Id::Threads,
            Box::new(Threads::new(exchanger.clone())),
            Threads::subscriptions(),
        )?;

        app.mount(
            Id::Source,
            Box::new(Source::new(exchanger)?),
            Source::subscriptions(),
        )?;

        let output = output_buf.data.lock().unwrap().clone();
        app.mount(
            Id::Output,
            Box::new(Output::new(&output)),
            Output::subscriptions(),
        )?;
        app.mount(Id::Logs, Box::new(Stub::new("Logs")), Vec::default())?;

        app.active(&Id::LeftTabs)?;

        Ok(app)
    }
}

impl Model {
    pub fn update(&mut self, msg: Option<Msg>) -> anyhow::Result<Option<Msg>> {
        if let Some(msg) = msg {
            // Set redraw
            self.redraw = true;
            // Match message
            match msg {
                Msg::AppClose => {
                    self.exchanger.send_exit();
                    self.quit = true;
                }
                Msg::AppRunning => {
                    self.app.attr(
                        &Id::Status,
                        Attribute::Text,
                        AttrValue::Payload(PropPayload::Vec(vec![PropValue::TextSpan(
                            TextSpan::new("running").fg(Color::Red),
                        )])),
                    )?;
                }
                Msg::SwitchUI => {
                    self.exchanger.send_switch_ui();
                    self.quit = true;
                }
                Msg::BreakpointsInFocus => {
                    self.app.active(&Id::Breakpoints)?;
                }
                Msg::VariablesInFocus => {
                    self.app.active(&Id::Variables)?;
                }
                Msg::ThreadsInFocus => {
                    self.app.active(&Id::Threads)?;
                }
                Msg::LeftTabsInFocus => {
                    self.app.active(&Id::LeftTabs)?;
                }
                Msg::RightTabsInFocus => {
                    self.app.active(&Id::RightTabs)?;
                }
                Msg::SourceInFocus => {
                    self.app.active(&Id::Source)?;
                }
                Msg::OutputInFocus => {
                    self.app.active(&Id::Output)?;
                }
                Msg::LogsInFocus => {
                    self.app.active(&Id::Logs)?;
                }
                Msg::AddBreakpointRequest => {
                    self.app.attr(
                        &Id::Input,
                        Attribute::Title,
                        AttrValue::Title(("Add breakpoint".to_string(), Alignment::Left)),
                    )?;
                    self.app.active(&Id::Input)?;
                    self.input_state = Some(InputType::Breakpoint);
                    self.app.lock_subs();
                }
                Msg::RemoveBreakpointRequest(brkpt_num) => {
                    self.exchanger
                        .request_sync(move |dbg| -> anyhow::Result<()> {
                            let cmd =
                                r#break::Command::Remove(BreakpointIdentity::Number(brkpt_num));
                            command::r#break::Handler::new(dbg).handle(&cmd)?;
                            Ok(())
                        })?;

                    self.app.attr(
                        &Id::Breakpoints,
                        Attribute::Content,
                        AttrValue::Table(Breakpoints::breakpoint_table(self.exchanger.clone())),
                    )?;
                }
                Msg::Input(input) => match self.input_state {
                    None => {}
                    Some(InputType::Breakpoint) => {
                        self.exchanger
                            .request_sync(move |dbg| -> anyhow::Result<()> {
                                let command = Command::parse(&input)?;
                                if let Command::Breakpoint(r#break::Command::Add(brkpt)) = command {
                                    command::r#break::Handler::new(dbg)
                                        .handle(&r#break::Command::Add(brkpt))?;
                                };
                                Ok(())
                            })?;

                        self.input_state = None;
                        self.app.unlock_subs();
                        self.app.blur()?;

                        self.app.attr(
                            &Id::Breakpoints,
                            Attribute::Content,
                            AttrValue::Table(Breakpoints::breakpoint_table(self.exchanger.clone())),
                        )?;
                    }
                },
                Msg::ShowAlert(text) => {
                    self.alert = true;
                    self.app
                        .attr(&Id::Alert, Attribute::Text, AttrValue::String(text))?;
                    self.app.active(&Id::Alert)?;
                }
                Msg::CloseAlert => {
                    self.alert = false;
                    self.app.blur()?;
                }

                Msg::None => {}
            }
        }

        Ok(None)
    }
}
