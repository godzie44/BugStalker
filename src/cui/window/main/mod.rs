use crate::cui::window::{Action, CuiComponent};
use crate::cui::AppContext;
use crossterm::event::KeyEvent;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

pub(super) mod breakpoint;
pub(super) mod variable;

pub(super) struct DebugeeView {}

impl CuiComponent for DebugeeView {
    fn render(&self, ctx: AppContext, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect) {
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

    fn handle_user_event(&mut self, _: AppContext, _: KeyEvent) -> Vec<Action> {
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
        _ctx: AppContext,
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

    fn handle_user_event(&mut self, _: AppContext, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "main.right.logs"
    }
}
