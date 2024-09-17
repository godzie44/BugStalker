use crate::debugger::process::{Child, Installed};
use crate::debugger::{Debugger, DebuggerBuilder};
use crate::ui::tui::app::port::DebuggerEventQueue;
use crate::ui::tui::{TuiApplication, TuiHook};
use crate::ui::{console, supervisor, DebugeeOutReader};
use heh::app::Application as Heh;
use heh::decoder::Encoding;
use ratatui::{
    crossterm::event::{self, Event, KeyCode},
    Frame,
};
use std::path::PathBuf;

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

    pub fn extend(self, mut debugger: Debugger) -> HehApplication {
        let debugger_event_queue = DebuggerEventQueue::default();
        debugger.set_hook(TuiHook::new(debugger_event_queue.clone()));

        HehApplication::new(debugger, self.debugee_out, self.debugee_err)
    }
}

pub struct HehApplication {
    debugger: Debugger,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl HehApplication {
    fn new(
        debugger: Debugger,
        debugee_out: DebugeeOutReader,
        debugee_err: DebugeeOutReader,
    ) -> Self {
        Self {
            debugger,
            debugee_out,
            debugee_err,
        }
    }

    pub fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        let path = PathBuf::from(self.debugger.process().program());
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let mut heh = Heh::new(file, Encoding::Ascii, 0).unwrap();

        let mut terminal = ratatui::init();
        loop {
            terminal
                .draw(|frame: &mut Frame| {
                    heh.render_frame(frame, frame.area());
                })
                .expect("failed to draw frame");
            if let Event::Key(key) = event::read().expect("failed to read event") {
                if key.code == KeyCode::Char('q') {
                    break;
                }
                heh.handle_input(&ratatui::crossterm::event::Event::Key(key))
                    .unwrap();
            }
        }
        ratatui::restore();

        let builder = console::AppBuilder::new(self.debugee_out, self.debugee_err);
        let app = builder
            .extend(self.debugger)
            .expect("build application fail");
        Ok(supervisor::ControlFlow::Switch(
            supervisor::Application::Terminal(app),
        ))
    }
}
