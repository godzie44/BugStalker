use crate::cui::hook::CuiHook;
use crate::cui::window::{Action, CuiComponent, RenderContext};
use crate::debugger::Debugger;
use crossterm::event::KeyEvent;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

pub(super) mod breakpoint;

pub(super) struct DebugeeView {}

impl CuiComponent for DebugeeView {
    fn render(
        &self,
        ctx: RenderContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
    ) {
        let home = Paragraph::new(ctx.data.main_text.clone().take())
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title("Debugee")
                    .border_type(BorderType::Plain),
            );
        frame.render_widget(home, rect);
    }

    fn handle_user_event(&mut self, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "main.right.debugee"
    }
}

pub(super) struct MainLogs {}

impl CuiComponent for MainLogs {
    fn render(
        &self,
        _ctx: RenderContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
    ) {
        let home = Paragraph::new("todo").alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::White))
                .title("Logs")
                .border_type(BorderType::Plain),
        );
        frame.render_widget(home, rect);
    }

    fn handle_user_event(&mut self, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "main.right.logs"
    }
}

pub(super) struct Variables {
    debugger: Rc<Debugger<CuiHook>>,
}

impl Variables {
    pub(super) fn new(debugger: impl Into<Rc<Debugger<CuiHook>>>) -> Self {
        Self {
            debugger: debugger.into(),
        }
    }
}

impl CuiComponent for Variables {
    fn render(&self, _: RenderContext, _: &mut Frame<CrosstermBackend<StdoutLock>>, _: Rect) {}

    fn handle_user_event(&mut self, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "main.left.variables"
    }
}
