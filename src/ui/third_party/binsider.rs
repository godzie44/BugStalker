use crate::debugger::{Debugger, NopHook};
use crate::ui::{console, supervisor, DebugeeOutReader};
use binsider::prelude::Event;
use binsider::prelude::*;
use ratatui::{crossterm::event::KeyCode, Frame};
use std::fs;
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

    pub fn extend(self, mut debugger: Debugger) -> BinsiderApplication {
        debugger.set_hook(NopHook {});
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
        let events = EventHandler::new(250);
        state.analyzer.extract_strings(events.sender.clone());

        let mut terminal = ratatui::init();
        loop {
            // Render the UI.
            terminal.draw(|frame: &mut Frame| {
                render(&mut state, frame);
            })?;

            let event = events.next()?;
            match event {
                Event::Key(key_event) => {
                    if key_event.code == KeyCode::Char('q') {
                        break;
                    }
                    binsider::handle_event(Event::Key(key_event), &events, &mut state)?;
                }
                Event::Restart(None) => {
                    break;
                }
                Event::Restart(Some(path)) => {
                    let Some(path) = path.to_str() else { break };
                    let file_data = std::fs::read(path)?;
                    let bytes = file_data.as_slice();
                    let file_info = FileInfo::new(path, Some(vec![]), bytes)?;
                    let analyzer = Analyzer::new(file_info, 15, vec![])?;

                    state.change_analyzer(analyzer);
                    state.handle_tab()?;
                    state.analyzer.extract_strings(events.sender.clone());
                }
                _ => {
                    binsider::handle_event(event, &events, &mut state)?;
                }
            }
        }
        events.stop();
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
