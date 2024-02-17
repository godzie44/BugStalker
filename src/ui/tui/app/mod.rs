pub mod port;

use crate::debugger::Error;
use crate::ui::command;
use crate::ui::command::r#break::BreakpointIdentity;
use crate::ui::command::{r#break, run, CommandError};
use crate::ui::tui::app::port::{
    AsyncResponsesPort, DebuggerEventQueue, DebuggerEventsPort, LoggerPort, OutputPort, UserEvent,
};
use crate::ui::tui::components::asm::Asm;
use crate::ui::tui::components::breakpoint::Breakpoints;
use crate::ui::tui::components::control::GlobalControl;
use crate::ui::tui::components::input::{Input, InputStringType};
use crate::ui::tui::components::logs::Logs;
use crate::ui::tui::components::oracle::Oracles;
use crate::ui::tui::components::output::Output;
use crate::ui::tui::components::popup::{Popup, YesNoLabels};
use crate::ui::tui::components::source::Source;
use crate::ui::tui::components::status::Status;
use crate::ui::tui::components::tabs::{LeftTab, RightTab};
use crate::ui::tui::components::threads::Threads;
use crate::ui::tui::components::variables::Variables;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::utils::logger::TuiLogLine;
use chumsky::Parser;
use std::borrow::Cow;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tuirealm::props::{PropPayload, PropValue, TextSpan};
use tuirealm::terminal::TerminalBridge;
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::layout::{Constraint, Direction, Layout};
use tuirealm::tui::style::Color;
use tuirealm::{
    props, Application, AttrValue, Attribute, EventListenerCfg, State, StateValue, Sub, SubClause,
    SubEventClause,
};

use super::{BreakpointsAddType, ConfirmedAction, DebugeeStreamBuffer, Id, Msg};

pub struct Model {
    /// Application
    pub app: Application<Id, Msg, UserEvent>,
    /// Indicates that the application must quit
    pub quit: bool,
    /// Tells whether to redraw interface
    pub redraw: bool,
    /// Used to draw to terminal
    pub terminal: TerminalBridge,

    exchanger: Arc<ClientExchanger>,
}

impl Model {
    pub fn new(
        output_buf: DebugeeStreamBuffer,
        event_queue: DebuggerEventQueue,
        client_exchanger: ClientExchanger,
        log_buffer: Arc<Mutex<Vec<TuiLogLine>>>,
    ) -> anyhow::Result<Self> {
        let exchanger = Arc::new(client_exchanger);
        Ok(Self {
            app: Self::init_app(output_buf, event_queue, exchanger.clone(), log_buffer)?,
            quit: false,
            redraw: true,
            terminal: TerminalBridge::new().expect("Cannot initialize terminal"),
            exchanger,
        })
    }
}

impl Model {
    pub fn view(&mut self) {
        _ = self.terminal.raw_mut().draw(|f| {
            let input_in_focus = self.app.focus() == Some(&Id::Input);
            let popup_in_focus = self.app.focus() == Some(&Id::Popup);

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
                            2 => Id::Asm,
                            3 => Id::Oracles,
                            4 => Id::Logs,
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

            if popup_in_focus {
                self.app.view(&Id::Popup, f, f.size());
            }
        });
    }

    fn init_app(
        output_buf: DebugeeStreamBuffer,
        event_queue: DebuggerEventQueue,
        exchanger: Arc<ClientExchanger>,
        log_buffer: Arc<Mutex<Vec<TuiLogLine>>>,
    ) -> anyhow::Result<Application<Id, Msg, UserEvent>> {
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
                .port(
                    Box::new(LoggerPort::new(log_buffer)),
                    Duration::from_millis(10),
                )
                .poll_timeout(Duration::from_millis(10))
                .tick_interval(Duration::from_secs(1)),
        );

        let pid = exchanger.request_sync(|dbg| dbg.process().pid());
        app.mount(
            Id::GlobalControl,
            Box::new(GlobalControl::new(exchanger.clone(), pid)),
            GlobalControl::subscriptions(),
        )?;

        app.mount(Id::Popup, Box::<Popup>::default(), vec![])?;
        app.mount(Id::Input, Box::<Input>::default(), vec![])?;

        let mb_err =
            exchanger.request_sync(|dbg| run::Handler::new(dbg).handle(run::Command::DryStart));
        let already_run = matches!(mb_err.err(), Some(CommandError::Handle(Error::AlreadyRun)));

        app.mount(
            Id::Status,
            Box::new(Status::new(already_run)),
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
            Box::new(Source::new(exchanger.clone())?),
            Source::subscriptions(),
        )?;
        app.mount(
            Id::Asm,
            Box::new(Asm::new(exchanger.clone())?),
            Asm::subscriptions(),
        )?;

        let output = output_buf.data.lock().unwrap().clone();
        app.mount(
            Id::Output,
            Box::new(Output::new(&output)),
            Output::subscriptions(),
        )?;
        app.mount(Id::Logs, Box::<Logs>::default(), Logs::subscriptions())?;

        app.active(&Id::LeftTabs)?;

        let oracles: Vec<_> = exchanger.request_sync(|dbg| dbg.all_oracles_arc().collect());
        app.mount(
            Id::Oracles,
            Box::new(Oracles::new(&oracles)),
            vec![Sub::new(SubEventClause::Tick, SubClause::Always)],
        )?;

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
                Msg::BreakpointsUpdate => {
                    self.app.attr(
                        &Id::Breakpoints,
                        Attribute::Custom("update_breakpoints"),
                        AttrValue::Flag(true),
                    )?;
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
                Msg::AsmInFocus => {
                    self.app.active(&Id::Asm)?;
                }
                Msg::OraclesInFocus => {
                    self.app.active(&Id::Oracles)?;
                }
                Msg::LogsInFocus => {
                    self.app.active(&Id::Logs)?;
                }
                Msg::BreakpointAdd(r#type) => {
                    let (input_validator, input_data_type): (fn(&str) -> bool, _) = match r#type {
                        BreakpointsAddType::AtLine => (
                            |s| -> bool {
                                command::parser::brkpt_at_line_parser()
                                    .parse(s)
                                    .into_result()
                                    .is_ok()
                            },
                            InputStringType::BreakpointAddAtLine,
                        ),
                        BreakpointsAddType::AtFunction => (
                            |s| -> bool {
                                command::parser::brkpt_at_fn()
                                    .parse(s)
                                    .into_result()
                                    .is_ok()
                            },
                            InputStringType::BreakpointAddAtFunction,
                        ),
                        BreakpointsAddType::AtAddress => (
                            |s| -> bool {
                                command::parser::brkpt_at_addr_parser()
                                    .parse(s)
                                    .into_result()
                                    .is_ok()
                            },
                            InputStringType::BreakpointAddAtAddress,
                        ),
                    };

                    self.app.attr(
                        &Id::Input,
                        Attribute::InputType,
                        AttrValue::InputType(props::InputType::Custom(
                            input_validator,
                            |_, _| -> bool { true },
                        )),
                    )?;
                    self.app.attr(
                        &Id::Input,
                        Attribute::Title,
                        AttrValue::Title(("Add breakpoint".to_string(), Alignment::Left)),
                    )?;
                    self.app.attr(
                        &Id::Input,
                        Attribute::Custom("input_data_type"),
                        AttrValue::String(input_data_type.to_string()),
                    )?;

                    self.app.active(&Id::Input)?;
                    self.app.lock_subs();
                }
                Msg::Input(input) => {
                    let input_data_type = InputStringType::from_str(
                        &self
                            .app
                            .query(&Id::Input, Attribute::Custom("input_data_type"))?
                            .expect("infallible")
                            .unwrap_string(),
                    )
                    .expect("infallible");
                    return match input_data_type {
                        InputStringType::BreakpointAddAtFunction
                        | InputStringType::BreakpointAddAtLine
                        | InputStringType::BreakpointAddAtAddress => {
                            let identity = match input_data_type {
                                InputStringType::BreakpointAddAtLine => {
                                    let file_line: Vec<_> = input.split(':').collect();
                                    let file = file_line[0];
                                    let line: u64 = file_line[1].parse().expect("infallible");
                                    BreakpointIdentity::Line(file.to_string(), line)
                                }
                                InputStringType::BreakpointAddAtFunction => {
                                    BreakpointIdentity::Function(input)
                                }
                                InputStringType::BreakpointAddAtAddress => {
                                    BreakpointIdentity::Address(input.parse().expect("infallible"))
                                }
                            };

                            let cmd = r#break::Command::Add(identity);
                            self.exchanger
                                .request_sync(move |dbg| -> anyhow::Result<()> {
                                    command::r#break::Handler::new(dbg).handle(&cmd)?;
                                    Ok(())
                                })?;

                            self.app.unlock_subs();
                            self.app.blur()?;
                            Ok(Some(Msg::BreakpointsUpdate))
                        }
                    };
                }
                Msg::InputCancel => {
                    self.app.unlock_subs();
                    self.app.blur()?;
                    return Ok(Some(Msg::BreakpointsUpdate));
                }
                Msg::ShowOkPopup(title, text) => {
                    if let Some(title) = title {
                        self.app
                            .attr(&Id::Popup, Attribute::Title, AttrValue::String(title))?;
                    }
                    self.app
                        .attr(&Id::Popup, Attribute::Text, AttrValue::String(text))?;
                    let (ok_attr, ok_attr_val) = Popup::ok_attrs();
                    self.app.attr(&Id::Popup, ok_attr, ok_attr_val)?;
                    self.app.active(&Id::Popup)?;
                }
                Msg::PopupOk => {
                    self.app.blur()?;
                }
                Msg::PopupConfirmDebuggerRestart => {
                    self.app.attr(
                        &Id::Popup,
                        Attribute::Text,
                        AttrValue::String("Restart a program?".to_string()),
                    )?;
                    let (attr, attr_val) = Popup::yes_no_attrs(YesNoLabels::default());
                    self.app.attr(&Id::Popup, attr, attr_val)?;
                    let action = ConfirmedAction::Restart;
                    self.app.attr(
                        &Id::Popup,
                        Attribute::Custom("action"),
                        AttrValue::String(action.to_string()),
                    )?;
                    self.app.active(&Id::Popup)?;
                }
                Msg::PopupBreakpoint(brkpt) => {
                    let place = &brkpt.place;
                    let file = place
                        .as_ref()
                        .map(|p| p.file.to_string_lossy())
                        .unwrap_or(Cow::from("unknown"));
                    let line = place
                        .as_ref()
                        .map(|p| p.line_number.to_string())
                        .unwrap_or("unknown".to_string());
                    let text = format!(
                        "Breakpoint #{}\nAt: {:?}:{}\nAddress: {}",
                        brkpt.number, file, line, brkpt.addr
                    );

                    self.app
                        .attr(&Id::Popup, Attribute::Text, AttrValue::String(text))?;
                    let (attr, attr_val) = Popup::yes_no_attrs(YesNoLabels::new("OK", "Remove"));
                    self.app.attr(&Id::Popup, attr, attr_val)?;
                    self.app.attr(
                        &Id::Popup,
                        Attribute::Custom("action"),
                        AttrValue::String(ConfirmedAction::RemoveBreakpoint.to_string()),
                    )?;
                    self.app.active(&Id::Popup)?;
                }
                Msg::PopupYes(action) => match action {
                    ConfirmedAction::Restart => {
                        self.exchanger.request_async(|dbg| {
                            Ok(run::Handler::new(dbg).handle(run::Command::Restart)?)
                        });
                        self.app.blur()?;
                        return Ok(Some(Msg::AppRunning));
                    }
                    _ => {
                        self.app.blur()?;
                    }
                },
                Msg::PopupNo(action) => match action {
                    ConfirmedAction::RemoveBreakpoint => {
                        let brkpt_num = self.app.state(&Id::Breakpoints)?.unwrap_one().unwrap_u32();
                        self.exchanger
                            .request_sync(move |dbg| -> anyhow::Result<()> {
                                let cmd =
                                    r#break::Command::Remove(BreakpointIdentity::Number(brkpt_num));
                                command::r#break::Handler::new(dbg).handle(&cmd)?;
                                Ok(())
                            })?;
                        self.app.blur()?;
                        return Ok(Some(Msg::BreakpointsUpdate));
                    }
                    _ => {
                        self.app.blur()?;
                    }
                },

                Msg::None => {}
            }
        }

        Ok(None)
    }
}
