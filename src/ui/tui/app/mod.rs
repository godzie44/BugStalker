pub mod port;

use crate::debugger::Error;
use crate::ui::command;
use crate::ui::command::r#break::BreakpointIdentity;
use crate::ui::command::watch::WatchpointIdentity;
use crate::ui::command::{CommandError, r#break, run, watch};
use crate::ui::tui::app::port::{
    AsyncResponsesPort, DebuggerEventQueue, DebuggerEventsPort, LoggerPort, OutputPort, UserEvent,
};
use crate::ui::tui::components::asm::Asm;
use crate::ui::tui::components::breakpoint::Breakpoints;
use crate::ui::tui::components::control::GlobalControl;
use crate::ui::tui::components::input::{Input, InputStringType};
use crate::ui::tui::components::logs::Logs;
use crate::ui::tui::components::oracle::make_oracle_tab_window;
use crate::ui::tui::components::output::Output;
use crate::ui::tui::components::popup::{Popup, YesNoLabels};
use crate::ui::tui::components::source::Source;
use crate::ui::tui::components::status::Status;
use crate::ui::tui::components::threads::Threads;
use crate::ui::tui::components::variables::Variables;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::utils::logger::TuiLogLine;
use crate::ui::tui::utils::tab;
use crate::ui::tui::utils::tab::TabWindow;
use anyhow::anyhow;
use chumsky::Parser;
use log::warn;
use std::borrow::Cow;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tuirealm::listener::SyncPort;
use tuirealm::props::{PropPayload, PropValue, TextSpan};
use tuirealm::ratatui::layout::Alignment;
use tuirealm::ratatui::layout::{Constraint, Direction, Layout};
use tuirealm::ratatui::style::Color;
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalBridge};
use tuirealm::{
    Application, AttrValue, Attribute, EventListenerCfg, Sub, SubClause, SubEventClause, props,
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
    pub terminal: TerminalBridge<CrosstermTerminalAdapter>,
    /// Message exchanger with tracer (debugger) thread
    exchanger: Arc<ClientExchanger>,
    /// Layout of main tabs
    tabs_layout: [Constraint; 2],
}

impl Model {
    const DEFAULT_TABS_LAYOUT: [Constraint; 2] =
        [Constraint::Percentage(25), Constraint::Percentage(75)];
    const LEFT_TAB_FOCUS_LAYOUT: [Constraint; 2] =
        [Constraint::Percentage(90), Constraint::Percentage(10)];
    const RIGHT_TAB_FOCUS_LAYOUT: [Constraint; 2] =
        [Constraint::Percentage(10), Constraint::Percentage(90)];

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
            terminal: TerminalBridge::init_crossterm().expect("Cannot initialize terminal"),
            exchanger,
            tabs_layout: Self::DEFAULT_TABS_LAYOUT,
        })
    }
}

impl Model {
    pub fn view(&mut self) {
        _ = self.terminal.raw_mut().draw(|f| {
            let input_in_focus = self.app.focus() == Some(&Id::Input);
            let popup_in_focus = self.app.focus() == Some(&Id::Popup);

            let mut constraints = vec![Constraint::Min(9), Constraint::Max(3)];
            if input_in_focus {
                constraints.push(Constraint::Max(3));
            }

            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(constraints)
                .split(f.area());

            let tabs_rect = main_chunks[0];
            let tab_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(self.tabs_layout)
                .split(tabs_rect);

            self.app.view(&Id::LeftTabs, f, tab_chunks[0]);
            self.app.view(&Id::RightTabs, f, tab_chunks[1]);

            if input_in_focus {
                self.app.view(&Id::Input, f, main_chunks[1]);
                self.app.view(&Id::Status, f, main_chunks[2]);
            } else {
                self.app.view(&Id::Status, f, main_chunks[1]);
            }

            if popup_in_focus {
                self.app.view(&Id::Popup, f, f.area());
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
                .crossterm_input_listener(Duration::from_millis(20), 3)
                .port(SyncPort::new(
                    Box::new(OutputPort::new(output_buf.data.clone())),
                    Duration::from_millis(10),
                    1,
                ))
                .port(SyncPort::new(
                    Box::new(DebuggerEventsPort::new(event_queue)),
                    Duration::from_millis(10),
                    1,
                ))
                .port(SyncPort::new(
                    Box::new(AsyncResponsesPort::new(exchanger.clone())),
                    Duration::from_millis(10),
                    1,
                ))
                .port(SyncPort::new(
                    Box::new(LoggerPort::new(log_buffer)),
                    Duration::from_millis(10),
                    1,
                ))
                .poll_timeout(Duration::from_millis(10))
                .tick_interval(Duration::from_millis(200)),
        );

        let pid = exchanger
            .request_sync(|dbg| dbg.process().pid())
            .expect("messaging enabled at tui start");
        app.mount(
            Id::GlobalControl,
            Box::new(GlobalControl::new(exchanger.clone(), pid)),
            GlobalControl::subscriptions(),
        )?;

        app.mount(Id::Popup, Box::<Popup>::default(), vec![])?;
        app.mount(Id::Input, Box::<Input>::default(), vec![])?;

        let mb_err = exchanger
            .request_sync(|dbg| run::Handler::new(dbg).handle(run::Command::DryStart))
            .expect("messaging enabled at tui start");
        let already_run = matches!(mb_err.err(), Some(CommandError::Handle(Error::AlreadyRun)));

        let output = output_buf.data.lock().unwrap().clone();
        let oracles: Vec<_> = exchanger
            .request_sync(|dbg| dbg.all_oracles_arc().collect())
            .expect("messaging enabled at tui start");

        app.mount(
            Id::Status,
            Box::new(Status::new(already_run)),
            Status::subscriptions(),
        )?;

        let mut left_tab_sub = Variables::subscriptions();
        left_tab_sub.extend(Threads::subscriptions());
        left_tab_sub.extend(vec![Sub::new(SubEventClause::Tick, SubClause::Always)]);

        let left_tab = TabWindow::new(
            "[1]",
            &["🔴 Breakpoints", "🧩 Variables", "🧵 Threads"],
            vec![
                Box::new(Breakpoints::new(exchanger.clone())),
                Box::new(Variables::new(exchanger.clone())),
                Box::new(Threads::new(exchanger.clone())),
            ],
            Some(|rewind_direction| match rewind_direction {
                tuirealm::command::Direction::Left => Msg::RightTabsInFocus {
                    reset_to: Some(props::Direction::Right),
                },
                tuirealm::command::Direction::Right => Msg::RightTabsInFocus {
                    reset_to: Some(props::Direction::Left),
                },
                _ => {
                    unreachable!()
                }
            }),
        );
        app.mount(Id::LeftTabs, Box::new(left_tab), left_tab_sub)?;

        let mut right_tab_sub = Source::subscriptions();
        right_tab_sub.extend(Asm::subscriptions());
        right_tab_sub.extend(Output::subscriptions());
        right_tab_sub.extend(vec![Sub::new(SubEventClause::Tick, SubClause::Always)]);

        let right_tab = TabWindow::new(
            "[2]",
            &["</> Source", "📃 Output", "🤖 Asm", "🔮 Oracles", "💾 Logs"],
            vec![
                Box::new(Source::new(exchanger.clone())?),
                Box::new(Output::new(&output)),
                Box::new(Asm::new(exchanger.clone())?),
                Box::new(make_oracle_tab_window(&oracles)),
                Box::<Logs>::default(),
            ],
            Some(|rewind_direction| match rewind_direction {
                tuirealm::command::Direction::Left => Msg::LeftTabsInFocus {
                    reset_to: Some(props::Direction::Right),
                },
                tuirealm::command::Direction::Right => Msg::LeftTabsInFocus {
                    reset_to: Some(props::Direction::Left),
                },
                _ => {
                    unreachable!()
                }
            }),
        );
        app.mount(Id::RightTabs, Box::new(right_tab), right_tab_sub)?;

        app.active(&Id::LeftTabs)?;

        Ok(app)
    }
}

impl Model {
    fn update_breakpoints(&mut self) -> anyhow::Result<()> {
        Ok(self.app.attr(
            &Id::LeftTabs,
            Attribute::Custom("update_breakpoints"),
            AttrValue::Flag(true),
        )?)
    }

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
                Msg::LeftTabsInFocus { reset_to } => {
                    self.app.active(&Id::LeftTabs)?;
                    if let Some(direction) = reset_to {
                        self.app.attr(
                            &Id::LeftTabs,
                            TabWindow::RESET_CHOICE_ATTR,
                            AttrValue::Direction(direction),
                        )?;
                    }
                    // change the focus again to prevent the situation
                    // when the focus was removed
                    // when call active for an already active component
                    _ = self
                        .app
                        .attr(&Id::LeftTabs, Attribute::Focus, AttrValue::Flag(true));
                }
                Msg::RightTabsInFocus { reset_to } => {
                    self.app.active(&Id::RightTabs)?;
                    if let Some(direction) = reset_to {
                        self.app.attr(
                            &Id::RightTabs,
                            TabWindow::RESET_CHOICE_ATTR,
                            AttrValue::Direction(direction),
                        )?;
                    }
                    // change the focus again to prevent the situation
                    // when the focus was removed
                    // when call active for an already active component
                    _ = self
                        .app
                        .attr(&Id::RightTabs, Attribute::Focus, AttrValue::Flag(true));
                }
                Msg::BreakpointAdd(r#type) => {
                    if !self.exchanger.is_messaging_enabled() {
                        warn!(target: "tui", "trying to add breakpoint but messaging is disabled");
                        return Ok(None);
                    }

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
                        BreakpointsAddType::Watchpoint => (
                            |s| -> bool {
                                command::parser::watchpoint_cond()
                                    .then(
                                        command::parser::watchpoint_at_dqe()
                                            .or(command::parser::watchpoint_at_address()),
                                    )
                                    .parse(s)
                                    .into_result()
                                    .is_ok()
                            },
                            InputStringType::Watchpoint,
                        ),
                    };

                    let title = if matches!(r#type, BreakpointsAddType::Watchpoint) {
                        "Add watchpoint".to_string()
                    } else {
                        "Add breakpoint".to_string()
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
                        AttrValue::Title((title, Alignment::Left)),
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
                                    BreakpointIdentity::Function(input.trim().to_string())
                                }
                                InputStringType::BreakpointAddAtAddress => {
                                    let input = input.trim().to_lowercase();
                                    let hex = input
                                        .strip_prefix("0x")
                                        .ok_or(anyhow!("invalid hex format"))?;
                                    let addr = usize::from_str_radix(hex, 16)
                                        .map_err(|e| anyhow!("invalid hex format: {e}"))?;
                                    BreakpointIdentity::Address(addr)
                                }
                                _ => unreachable!(),
                            };

                            let cmd = r#break::Command::Add(identity);
                            self.exchanger
                                .request_sync(move |dbg| -> anyhow::Result<()> {
                                    command::r#break::Handler::new(dbg).handle(&cmd)?;
                                    Ok(())
                                })
                                .expect("messaging enabled")?;

                            self.app.unlock_subs();
                            self.app.blur()?;
                            self.update_breakpoints()?;
                            self.app.active(&Id::LeftTabs)?;
                            self.app.attr(
                                &Id::LeftTabs,
                                TabWindow::ACTIVATE_TAB,
                                AttrValue::Flag(true),
                            )?;
                            Ok(None)
                        }
                        InputStringType::Watchpoint => {
                            // TODO two times parsing
                            let (cond, identity) = command::parser::watchpoint_cond()
                                .then(
                                    command::parser::watchpoint_at_dqe()
                                        .or(command::parser::watchpoint_at_address()),
                                )
                                .parse(&input)
                                .into_result()
                                .expect("infallible");
                            let cmd = watch::Command::Add(identity, cond);
                            self.exchanger
                                .request_sync(move |dbg| -> anyhow::Result<()> {
                                    command::watch::Handler::new(dbg).handle(cmd)?;
                                    Ok(())
                                })
                                .expect("messaging enabled")?;

                            self.app.unlock_subs();
                            self.app.blur()?;
                            self.update_breakpoints()?;
                            self.app.active(&Id::LeftTabs)?;
                            self.app.attr(
                                &Id::LeftTabs,
                                TabWindow::ACTIVATE_TAB,
                                AttrValue::Flag(true),
                            )?;
                            Ok(None)
                        }
                    };
                }
                Msg::InputCancel => {
                    self.app.unlock_subs();
                    self.app.blur()?;
                    self.update_breakpoints()?;
                    // FIXME: currently input cancel are possible only from breakpoints window,
                    // that's why breakpoints window come into focus here.
                    // This behaviour may be changed in future.
                    self.app.active(&Id::LeftTabs)?;
                    self.app.attr(
                        &Id::LeftTabs,
                        TabWindow::ACTIVATE_TAB,
                        AttrValue::Flag(true),
                    )?;
                }
                Msg::UpdateBreakpointList => {
                    self.update_breakpoints()?;
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
                Msg::PopupWatchpoint(wp) => {
                    let dqe = if let Some(ref dqe) = wp.source_dqe {
                        format!("For: {dqe}\n")
                    } else {
                        String::new()
                    };

                    let text = format!(
                        "Watchpoint #{}\n{dqe}Address: {}\nCondition: {}, size: {}",
                        wp.number, wp.address, wp.condition, wp.size
                    );

                    self.app
                        .attr(&Id::Popup, Attribute::Text, AttrValue::String(text))?;
                    let (attr, attr_val) = Popup::yes_no_attrs(YesNoLabels::new("OK", "Remove"));
                    self.app.attr(&Id::Popup, attr, attr_val)?;
                    self.app.attr(
                        &Id::Popup,
                        Attribute::Custom("action"),
                        AttrValue::String(ConfirmedAction::RemoveWatchpoint.to_string()),
                    )?;
                    self.app.active(&Id::Popup)?;
                }
                Msg::PopupYes(action) => match action {
                    ConfirmedAction::Restart => {
                        self.exchanger
                            .request_async(|dbg| {
                                Ok(run::Handler::new(dbg).handle(run::Command::Restart)?)
                            })
                            .expect("messaging enabled");
                        self.exchanger.disable_messaging();
                        self.app.blur()?;
                        return Ok(Some(Msg::AppRunning));
                    }
                    _ => {
                        self.app.blur()?;
                    }
                },
                Msg::PopupNo(action) => match action {
                    ConfirmedAction::RemoveBreakpoint => {
                        // the left tab must contain a breakpoints as active window
                        let brkpt_num = self.app.state(&Id::LeftTabs)?.unwrap_one().unwrap_u32();
                        self.exchanger
                            .request_sync(move |dbg| -> anyhow::Result<()> {
                                let cmd =
                                    r#break::Command::Remove(BreakpointIdentity::Number(brkpt_num));
                                command::r#break::Handler::new(dbg).handle(&cmd)?;
                                Ok(())
                            })
                            .expect("messaging enabled")?;
                        self.app.blur()?;
                        self.update_breakpoints()?;
                    }
                    ConfirmedAction::RemoveWatchpoint => {
                        // the left tab must contain a watchpoint as active window
                        let wp_num = self.app.state(&Id::LeftTabs)?.unwrap_one().unwrap_u32();
                        self.exchanger
                            .request_sync(move |dbg| -> anyhow::Result<()> {
                                let cmd =
                                    watch::Command::Remove(WatchpointIdentity::Number(wp_num));
                                command::watch::Handler::new(dbg).handle(cmd)?;
                                Ok(())
                            })
                            .expect("messaging enabled")?;
                        self.app.blur()?;
                        self.update_breakpoints()?;
                    }
                    _ => {
                        self.app.blur()?;
                    }
                },
                Msg::ExpandTab(tab_id) => {
                    debug_assert!(tab_id == Id::RightTabs || tab_id == Id::LeftTabs);
                    match tab_id {
                        Id::RightTabs
                            if self.tabs_layout == Self::DEFAULT_TABS_LAYOUT
                                || self.tabs_layout == Self::LEFT_TAB_FOCUS_LAYOUT =>
                        {
                            self.app.attr(
                                &Id::RightTabs,
                                TabWindow::VIEW_SIZE_ATTR,
                                tab::ViewSize::Expand.into(),
                            )?;
                            self.app.attr(
                                &Id::LeftTabs,
                                TabWindow::VIEW_SIZE_ATTR,
                                tab::ViewSize::Compacted.into(),
                            )?;

                            self.tabs_layout = Self::RIGHT_TAB_FOCUS_LAYOUT;
                        }
                        Id::LeftTabs
                            if self.tabs_layout == Self::DEFAULT_TABS_LAYOUT
                                || self.tabs_layout == Self::RIGHT_TAB_FOCUS_LAYOUT =>
                        {
                            self.app.attr(
                                &Id::LeftTabs,
                                TabWindow::VIEW_SIZE_ATTR,
                                tab::ViewSize::Expand.into(),
                            )?;
                            self.app.attr(
                                &Id::RightTabs,
                                TabWindow::VIEW_SIZE_ATTR,
                                tab::ViewSize::Compacted.into(),
                            )?;

                            self.tabs_layout = Self::LEFT_TAB_FOCUS_LAYOUT;
                        }
                        _ => {
                            for id in [&Id::LeftTabs, &Id::RightTabs] {
                                self.app.attr(
                                    id,
                                    TabWindow::VIEW_SIZE_ATTR,
                                    tab::ViewSize::Default.into(),
                                )?;
                            }
                            self.tabs_layout = Self::DEFAULT_TABS_LAYOUT;
                        }
                    }
                }

                Msg::None => {}
            }
        }

        Ok(None)
    }
}
