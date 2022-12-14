use crate::cui::hook::CuiHook;
use crate::cui::window::app::AppWindow;
use crate::cui::window::message::Exchanger;
use crate::cui::{context, AppState, DebugeeStreamBuffer, Event};
use crate::debugger::command::Continue;
use crate::debugger::Debugger;
use crossterm::event::{DisableMouseCapture, KeyCode, KeyEvent};
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use std::io::StdoutLock;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::{Frame, Terminal};

mod alert;
mod app;
mod help;
mod input;
mod main;
mod message;
mod tabs;

#[derive(Default, Clone, Copy)]
pub struct RenderOpts {
    pub in_focus: bool,
}

trait CuiComponent {
    fn render(&self, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect, opts: RenderOpts);
    fn handle_user_event(&mut self, e: KeyEvent);
    #[allow(unused)]
    fn update(&mut self) {}
    fn name(&self) -> &'static str;
}

pub(super) fn run(
    mut terminal: Terminal<CrosstermBackend<StdoutLock>>,
    debugger: Rc<Debugger<CuiHook>>,
    event_chan: Receiver<Event<KeyEvent>>,
    stream_buff: DebugeeStreamBuffer,
) -> anyhow::Result<()> {
    let mut app_window = AppWindow::new(debugger.clone(), stream_buff);

    loop {
        terminal.draw(|frame| {
            let rect = frame.size();
            app_window.render(frame, rect, RenderOpts::default());
        })?;

        let ctx = context::Context::current();
        match event_chan.recv()? {
            Event::Input(e) => match e {
                KeyEvent {
                    code: KeyCode::Char('c'),
                    ..
                } if !ctx.assert_state(AppState::UserInput) => {
                    ctx.change_state(AppState::DebugeeRun);
                    Continue::new(&debugger).run()?;
                }
                KeyEvent {
                    code: KeyCode::Char('q'),
                    ..
                } if !ctx.assert_state(AppState::UserInput) => {
                    disable_raw_mode()?;
                    crossterm::execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        DisableMouseCapture,
                    )?;
                    terminal.show_cursor()?;
                    return Ok(());
                }
                _ => {
                    app_window.handle_user_event(e);
                }
            },
            Event::Tick => {}
        }

        while !Exchanger::current().is_empty() {
            app_window.update();
        }
    }
}
