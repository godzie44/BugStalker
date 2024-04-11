use crate::debugger::process::{Child, Installed};
use crate::debugger::{BreakpointViewOwned, Debugger, DebuggerBuilder};
pub use crate::ui::tui::app::port::TuiHook;
use crate::ui::tui::app::port::{DebuggerEventQueue, UserEvent};
use crate::ui::tui::app::Model;
use crate::ui::tui::components::popup::Popup;
use crate::ui::tui::output::{OutputLine, OutputStreamProcessor, StreamType};
use crate::ui::tui::proto::{exchanger, Request};
use crate::ui::{console, supervisor, DebugeeOutReader};
use crate::weak_error;
use anyhow::anyhow;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use log::error;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{io, thread};
use strum_macros::{Display, EnumString};
use timeout_readwrite::TimeoutReader;
use tuirealm::{props, AttrValue, Attribute, PollStrategy};

pub mod app;
pub mod components;
mod output;
mod proto;
pub mod utils;

// Component ids for debugger application
#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub enum Id {
    LeftTabs,
    RightTabs,

    Status,
    GlobalControl,

    Input,
    Popup,
}

#[derive(Debug, PartialEq, EnumString, Display, Clone)]
pub enum ConfirmedAction {
    Restart,
    RemoveBreakpoint,
}

#[derive(Debug, PartialEq, Clone)]
pub enum BreakpointsAddType {
    AtLine,
    AtFunction,
    AtAddress,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Msg {
    None,
    AppClose,
    AppRunning,
    LeftTabsInFocus { reset_to: Option<props::Direction> },
    RightTabsInFocus { reset_to: Option<props::Direction> },
    SwitchUI,
    BreakpointAdd(BreakpointsAddType),
    ExpandTab(Id),

    PopupConfirmDebuggerRestart,
    PopupBreakpoint(BreakpointViewOwned),
    ShowOkPopup(Option<String>, String),
    PopupOk,
    PopupYes(ConfirmedAction),
    PopupNo(ConfirmedAction),

    Input(String),
    InputCancel,
}

#[derive(Default, Clone)]
pub struct DebugeeStreamBuffer {
    data: Arc<Mutex<Vec<OutputLine>>>,
}

pub struct AppBuilder {
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl AppBuilder {
    pub fn new(debugee_out: DebugeeOutReader, debugee_err: DebugeeOutReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
        }
    }

    pub fn build(
        self,
        dbg_builder: DebuggerBuilder<TuiHook>,
        process: Child<Installed>,
    ) -> anyhow::Result<TuiApplication> {
        let debugger_event_queue = DebuggerEventQueue::default();
        let debugger = dbg_builder
            .with_hooks(TuiHook::new(debugger_event_queue.clone()))
            .build(process)?;

        Ok(TuiApplication::new(
            debugger,
            self.debugee_out,
            self.debugee_err,
            debugger_event_queue,
        ))
    }

    pub fn extend(self, mut debugger: Debugger) -> TuiApplication {
        let debugger_event_queue = DebuggerEventQueue::default();
        debugger.set_hook(TuiHook::new(debugger_event_queue.clone()));

        TuiApplication::new(
            debugger,
            self.debugee_out,
            self.debugee_err,
            debugger_event_queue,
        )
    }
}

pub struct TuiApplication {
    debugger: Debugger,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    debugger_event_queue: Arc<Mutex<Vec<UserEvent>>>,
}

impl TuiApplication {
    pub fn new(
        debugger: Debugger,
        debugee_out: DebugeeOutReader,
        debugee_err: DebugeeOutReader,
        debugger_event_queue: Arc<Mutex<Vec<UserEvent>>>,
    ) -> Self {
        Self {
            debugger,
            debugee_out,
            debugee_err,
            debugger_event_queue,
        }
    }

    pub fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        let log_buffer = Arc::new(Mutex::default());
        let logger = utils::logger::TuiLogger::new(log_buffer.clone());
        let filter = logger.filter();
        crate::log::LOGGER_SWITCHER.switch(logger, filter);

        let stream_buf = DebugeeStreamBuffer::default();

        // init debugee stdout handler
        let out = TimeoutReader::new(self.debugee_out.clone(), Duration::from_millis(1));
        let std_out_handle =
            OutputStreamProcessor::new(StreamType::StdOut).run(out, stream_buf.data.clone());

        // init debugee stderr handler
        let out = TimeoutReader::new(self.debugee_err.clone(), Duration::from_millis(1));
        let std_err_handle =
            OutputStreamProcessor::new(StreamType::StdErr).run(out, stream_buf.data.clone());

        let (srv_exchanger, client_exchanger) = exchanger();

        // tui thread
        let ui_jh = thread::spawn(move || -> anyhow::Result<()> {
            let mut model = Model::new(
                stream_buf,
                self.debugger_event_queue,
                client_exchanger,
                log_buffer,
            )?;
            model.terminal.enter_alternate_screen()?;
            model.terminal.enable_raw_mode()?;
            weak_error!(execute!(io::stdout(), DisableMouseCapture));

            while !model.quit {
                match model.app.tick(PollStrategy::Once) {
                    Err(err) => {
                        model.app.attr(
                            &Id::Popup,
                            Attribute::Title,
                            AttrValue::String("TUI error".to_string()),
                        )?;
                        model.app.attr(
                            &Id::Popup,
                            Attribute::Text,
                            AttrValue::String(err.to_string()),
                        )?;
                        let (ok_attr, ok_attr_val) = Popup::ok_attrs();
                        model.app.attr(&Id::Popup, ok_attr, ok_attr_val)?;
                        model.app.active(&Id::Popup)?;
                    }
                    Ok(messages) if !messages.is_empty() => {
                        // NOTE: redraw if at least one msg has been processed
                        model.redraw = true;
                        for msg in messages.into_iter() {
                            let mut msg = Some(msg);
                            while msg.is_some() {
                                msg = model.update(msg).unwrap_or_else(|e| {
                                    Some(Msg::ShowOkPopup(Some("Error".to_string()), e.to_string()))
                                });
                            }
                        }
                    }
                    _ => {}
                }
                // Redraw
                if model.redraw {
                    model.view();
                    model.redraw = false;
                }
            }

            weak_error!(execute!(io::stdout(), EnableMouseCapture));
            model.terminal.leave_alternate_screen()?;
            model.terminal.disable_raw_mode()?;
            model.terminal.clear_screen()?;

            Ok(())
        });

        enum ExitType {
            Exit,
            Shutdown,
            SwitchUi,
        }

        let exit_type;
        loop {
            match srv_exchanger.next_request() {
                Some(Request::Exit) => {
                    exit_type = ExitType::Exit;
                    break;
                }
                Some(Request::SwitchUi) => {
                    exit_type = ExitType::SwitchUi;
                    break;
                }
                Some(Request::DebuggerSyncTask(task)) => {
                    let result = task(&mut self.debugger);
                    srv_exchanger.send_response(result);
                }
                Some(Request::DebuggerAsyncTask(task)) => {
                    if let Err(e) = task(&mut self.debugger) {
                        srv_exchanger.send_async_response(e);
                    }
                }
                None => {
                    exit_type = ExitType::Shutdown;
                    break;
                }
            }
        }

        drop(std_out_handle);
        drop(std_err_handle);

        match exit_type {
            ExitType::Exit => Ok(supervisor::ControlFlow::Exit),
            ExitType::Shutdown => {
                let join_result = ui_jh.join();
                let join_result = join_result
                    .map_err(|_| anyhow!("unexpected: tui thread panic"))
                    .and_then(|r| r);
                if let Err(e) = join_result {
                    error!(target: "tui", "tui thread error: {e}");
                };
                Ok(supervisor::ControlFlow::Exit)
            }
            ExitType::SwitchUi => {
                _ = ui_jh.join();
                let builder = console::AppBuilder::new(self.debugee_out, self.debugee_err);
                let app = builder
                    .extend(self.debugger)
                    .expect("build application fail");
                Ok(supervisor::ControlFlow::Switch(
                    supervisor::Application::Terminal(app),
                ))
            }
        }
    }
}
