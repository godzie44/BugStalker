use crate::cui::window::{Action, CuiComponent};
use crate::cui::AppContext;
use crossterm::event::KeyEvent;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Paragraph};
use tui::Frame;

pub(super) struct ContextHelp {}

impl CuiComponent for ContextHelp {
    fn render(
        &self,
        _ctx: AppContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
    ) {
        let copyright = Paragraph::new("Code walker, bug stalker")
            .style(Style::default().fg(Color::LightCyan))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(tui::widgets::Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title("Copyright")
                    .border_type(BorderType::Plain),
            );

        frame.render_widget(copyright, rect);
    }

    fn handle_user_event(&mut self, _: AppContext, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "context-help"
    }
}
