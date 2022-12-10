use crate::cui::hook::CuiHook;
use crate::cui::window::app::AppWindow;
use crate::cui::{AppContext, AppState, Event};
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

mod app;
mod help;
mod input;
mod main;
mod tabs;

#[derive(Default, Clone, Copy)]
pub struct RenderOpts {
    pub in_focus: bool,
}

trait CuiComponent {
    fn render(
        &self,
        ctx: AppContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    );
    fn handle_user_event(&mut self, ctx: AppContext, e: KeyEvent) -> Vec<Action>;
    #[allow(unused)]
    fn apply_app_action(&mut self, ctx: AppContext, actions: &[Action]) {}
    fn name(&self) -> &'static str;
}

#[derive(Clone, Debug)]
enum Action {
    ActivateComponent(&'static str),
    FocusOnComponent(&'static str),
    ActivateUserInput(/* activate requester */ &'static str),
    HandleUserInput(/* activate requester */ &'static str, String),
    CancelUserInput,
}

pub(super) fn run(
    ctx: AppContext,
    mut terminal: Terminal<CrosstermBackend<StdoutLock>>,
    debugger: Rc<Debugger<CuiHook>>,
    rx: Receiver<Event<KeyEvent>>,
) -> anyhow::Result<()> {
    let mut app_window = AppWindow::new(debugger.clone());

    loop {
        terminal.draw(|frame| {
            let rect = frame.size();
            app_window.render(ctx.clone(), frame, rect, RenderOpts::default());
        })?;

        match rx.recv()? {
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
                    app_window.handle_user_event(ctx.clone(), e);
                }
            },
            Event::Tick => {}
        }
    }
}
