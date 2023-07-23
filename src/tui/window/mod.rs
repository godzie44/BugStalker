use crate::debugger::command::{Continue, Run, StepInto, StepOut, StepOver};
use crate::debugger::Debugger;
use crate::tui::window::app::AppWindow;
use crate::tui::window::message::Exchanger;
use crate::tui::{context, AppState, DebugeeStreamBuffer, Event};
use crossterm::event::{DisableMouseCapture, KeyCode, KeyEvent};
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use std::cell::RefCell;
use std::io::StdoutLock;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::{Frame, Terminal};

mod app;
mod general;
mod message;
mod specialized;

#[derive(Default, Clone, Copy)]
pub struct RenderOpts {
    pub in_focus: bool,
}

pub trait TuiComponent {
    fn render(&self, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect, opts: RenderOpts);
    fn handle_user_event(&mut self, e: KeyEvent);
    #[allow(unused)]
    fn update(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn name(&self) -> &'static str;
}

macro_rules! try_else_alert {
    ($e: expr) => {
        if let Err(e) = $e {
            context::Context::current().set_alert(format!("An error occurred: {e}").into());
        }
    };
}

pub(super) fn run(
    mut terminal: Terminal<CrosstermBackend<StdoutLock>>,
    debugger: Rc<RefCell<Debugger>>,
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
        if ctx.assert_state(AppState::UserInput) {
            match event_chan.recv()? {
                Event::Input(e) => app_window.handle_user_event(e),
                Event::Tick => {}
            }
        } else {
            match event_chan.recv()? {
                Event::Input(e) => match e {
                    KeyEvent {
                        code: KeyCode::Char('c'),
                        ..
                    } => {
                        ctx.change_state(AppState::DebugeeRun);
                        try_else_alert!(Continue::new(&mut debugger.borrow_mut()).handle());
                    }
                    KeyEvent {
                        code: KeyCode::Char('r'),
                        ..
                    } => {
                        ctx.change_state(AppState::DebugeeRun);
                        try_else_alert!(Run::new(&mut debugger.borrow_mut()).start());
                    }
                    KeyEvent {
                        code: KeyCode::Char('q'),
                        ..
                    } => {
                        disable_raw_mode()?;
                        crossterm::execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture,
                        )?;
                        terminal.show_cursor()?;
                        return Ok(());
                    }
                    KeyEvent {
                        code: KeyCode::F(8),
                        ..
                    } => {
                        try_else_alert!(StepOver::new(&mut debugger.borrow_mut()).handle());
                    }
                    KeyEvent {
                        code: KeyCode::F(7),
                        ..
                    } => {
                        try_else_alert!(StepInto::new(&mut debugger.borrow_mut()).handle());
                    }
                    KeyEvent {
                        code: KeyCode::F(6),
                        ..
                    } => {
                        try_else_alert!(StepOut::new(&mut debugger.borrow_mut()).handle());
                    }
                    _ => {
                        app_window.handle_user_event(e);
                    }
                },
                Event::Tick => {}
            }
        }

        while !Exchanger::current().is_empty() {
            if let Err(e) = app_window.update() {
                context::Context::current().set_alert(format!("An error occurred: {e}").into());
                Exchanger::current().clear();
                break;
            }
        }
    }
}
