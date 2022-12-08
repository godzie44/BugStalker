use crate::cui::window::{Action, CuiComponent};
use crate::cui::{AppContext, AppState};
use crossterm::event::KeyEvent;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Paragraph};
use tui::Frame;

pub(super) struct ContextHelp {}

impl CuiComponent for ContextHelp {
    fn render(&self, ctx: AppContext, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(rect);

        let help = Paragraph::new("Code walker, bug stalker")
            .style(Style::default().fg(Color::LightCyan))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(tui::widgets::Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title("Copyright")
                    .border_type(BorderType::Plain),
            );

        let state_text = match ctx.state.get() {
            AppState::Initial => "Application start",
            AppState::DebugeeRun => "Application run",
            AppState::DebugeeBreak => "Application break",
            AppState::UserInput => "Wait for input",
        };

        let app_state = Paragraph::new(state_text)
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(tui::widgets::Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title("State")
                    .border_type(BorderType::Rounded),
            );

        frame.render_widget(help, chunks[0]);
        frame.render_widget(app_state, chunks[1]);
    }

    fn handle_user_event(&mut self, _: AppContext, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "context-help"
    }
}
