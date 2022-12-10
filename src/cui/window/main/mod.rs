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
        let home = Paragraph::new(ctx.data.debugee_text.clone().take())
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title(ctx.data.debugee_file_name.borrow().to_string())
                    .border_type(BorderType::Plain),
            )
            .scroll((ctx.data.debugee_text_pos.get() as u16, 0));
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
