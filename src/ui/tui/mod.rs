use crate::debugger::{BreakpointViewOwned, Debugger};
use crate::ui::tui::app::port::DebuggerEventQueue;
use crate::ui::tui::app::Model;
use crate::ui::tui::output::{OutputLine, OutputStreamProcessor, StreamType};
use crate::ui::tui::proto::{exchanger, Request};
use crate::ui::{console, DebugeeOutReader};
use anyhow::anyhow;
use log::error;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use strum_macros::{Display, EnumString};
use timeout_readwrite::TimeoutReader;
use tuirealm::{AttrValue, Attribute, PollStrategy};

mod app;
pub mod components;
mod output;
mod proto;
pub mod utils;

pub use crate::ui::tui::app::port::TuiHook;
use crate::ui::tui::components::popup::Popup;

// Component ids for debugger application
#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub enum Id {
    LeftTabs,
    RightTabs,

    Breakpoints,
    Threads,
    Variables,
    Output,
    Source,
    Logs,

    Status,
    GlobalControl,

    Input,
    Popup,
}

#[derive(Debug, PartialEq, EnumString, Display)]
pub enum ConfirmedAction {
    Restart,
    RemoveBreakpoint,
}

#[derive(Debug, PartialEq)]
pub enum BreakpointsAddType {
    AtLine,
    AtFunction,
    AtAddress,
}

#[derive(Debug, PartialEq)]
pub enum Msg {
    None,
    AppClose,
    AppRunning,
    LeftTabsInFocus,
    RightTabsInFocus,
    SwitchUI,
    BreakpointsInFocus,
    BreakpointsUpdate,
    BreakpointAdd(BreakpointsAddType),
    VariablesInFocus,
    ThreadsInFocus,
    SourceInFocus,
    OutputInFocus,
    LogsInFocus,
    PopupConfirmDebuggerRestart,
    PopupBreakpoint(BreakpointViewOwned),
    ShowOkPopup(Option<String>, String),
    PopupOk,
    PopupYes(ConfirmedAction),
    PopupNo(ConfirmedAction),
    Input(String),
}

#[derive(Default, Clone)]
pub struct DebugeeStreamBuffer {
    data: Arc<Mutex<Vec<OutputLine>>>,
}

pub struct AppBuilder {
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    already_run: bool,
}

impl AppBuilder {
    pub fn new(debugee_out: DebugeeOutReader, debugee_err: DebugeeOutReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
            already_run: false,
        }
    }

    pub fn app_already_run(self) -> Self {
        Self {
            already_run: true,
            ..self
        }
    }

    pub fn build(self, debugger: Debugger) -> TuiApplication {
        TuiApplication::new(
            debugger,
            self.debugee_out,
            self.debugee_err,
            self.already_run,
        )
    }
}

pub struct TuiApplication {
    already_run: bool,
    debugger: Debugger,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl TuiApplication {
    pub fn new(
        debugger: Debugger,
        debugee_out: DebugeeOutReader,
        debugee_err: DebugeeOutReader,
        already_run: bool,
    ) -> Self {
        Self {
            debugger,
            debugee_out,
            debugee_err,
            already_run,
        }
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        // disable default logger
        crate::log::disable();

        let debugger_event_queue = DebuggerEventQueue::default();
        self.debugger
            .set_hook(TuiHook::new(debugger_event_queue.clone()));

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
        let already_run = self.already_run;
        let ui_jh = thread::spawn(move || -> anyhow::Result<()> {
            let mut model = Model::new(
                stream_buf,
                debugger_event_queue,
                client_exchanger,
                already_run,
            )?;
            model.terminal.enter_alternate_screen()?;
            model.terminal.enable_raw_mode()?;

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
                        model.popup = true;
                    }
                    Ok(messages) if !messages.is_empty() => {
                        // NOTE: redraw if at least one msg has been processed
                        model.redraw = true;
                        for msg in messages.into_iter() {
                            let mut msg = Some(msg);
                            while msg.is_some() {
                                msg = match model.update(msg) {
                                    Ok(msg) => msg,
                                    Err(e) => Some(Msg::ShowOkPopup(
                                        Some("Error".to_string()),
                                        e.to_string(),
                                    )),
                                };
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
        crate::log::enable();

        match exit_type {
            ExitType::Exit => {}
            ExitType::Shutdown => {
                let join_result = ui_jh.join();
                let join_result = join_result
                    .map_err(|_| anyhow!("unexpected: tui thread panic"))
                    .and_then(|r| r);
                if let Err(e) = join_result {
                    error!(target: "tui", "tui thread error: {e}");
                };
            }
            ExitType::SwitchUi => {
                _ = ui_jh.join();
                let mut builder = console::AppBuilder::new(self.debugee_out, self.debugee_err);
                if self.already_run {
                    builder = builder.app_already_run();
                }
                let app = builder
                    .build(self.debugger)
                    .expect("build application fail");
                app.run().expect("run application fail");
            }
        }

        Ok(())
    }
}
