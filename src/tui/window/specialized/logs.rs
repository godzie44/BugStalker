use crate::tui::window::{RenderOpts, TuiComponent};
use crossterm::event::KeyEvent;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

#[derive(Default)]
pub(crate) struct Logs {}

impl TuiComponent for Logs {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        let border_style = if opts.in_focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let home = Paragraph::new("todo").alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .style(Style::default().fg(Color::White))
                .title("Logs"),
        );
        frame.render_widget(home, rect);
    }

    fn handle_user_event(&mut self, _: KeyEvent) {}

    fn name(&self) -> &'static str {
        "logs"
    }
}
