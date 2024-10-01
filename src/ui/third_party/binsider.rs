use crate::debugger::Debugger;
use crate::ui::tui::app::port::DebuggerEventQueue;
use crate::ui::tui::TuiHook;
use crate::ui::{console, supervisor, DebugeeOutReader};
use binsider::prelude::{Command, Event};
use binsider::{prelude::*, tui::ui::Tab};
use ratatui::{
    crossterm::event::{self, Event as CrosstermEvent, KeyCode},
    Frame,
};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

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

    pub fn extend(self, mut debugger: Debugger) -> BinsiderApplication {
        let debugger_event_queue = DebuggerEventQueue::default();
        debugger.set_hook(TuiHook::new(debugger_event_queue.clone()));

        BinsiderApplication::new(debugger, self.debugee_out, self.debugee_err)
    }
}

/// This is a wrapper over `binsider` application (see https://github.com/orhun/binsider) for running
/// it inside a `BugStalker`.
pub struct BinsiderApplication {
    debugger: Debugger,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl BinsiderApplication {
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

    pub fn run(self) -> anyhow::Result<supervisor::ControlFlow> {
        let path = PathBuf::from(self.debugger.process().program());
        let file_data = fs::read(self.debugger.process().program())?;
        let file_info = FileInfo::new(
            path.to_str().unwrap_or_default(),
            None,
            file_data.as_slice(),
        )?;
        let analyzer = Analyzer::new(file_info, 15, vec![])?;
        let mut state = State::new(analyzer)?;
        let (sender, receiver) = mpsc::channel();
        state.analyzer.extract_strings(sender.clone());

        let mut terminal = ratatui::init();
        loop {
            // Render the UI.
            terminal.draw(|frame: &mut Frame| {
                render(&mut state, frame);
            })?;

            // Handle terminal events.
            if event::poll(Duration::from_millis(16))? {
                if let CrosstermEvent::Key(key) = event::read()? {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    let command = Command::from(key);
                    state.run_command(command, sender.clone())?;
                }
            }

            // Handle binsider events.
            if let Ok(Event::FileStrings(strings)) = receiver.try_recv() {
                state.strings_loaded = true;
                state.analyzer.strings = Some(strings?.into_iter().map(|(v, l)| (l, v)).collect());
                if state.tab == Tab::Strings {
                    state.handle_tab()?;
                }
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
