use crate::debugger::Debugger;
use crate::tui::window::{RenderOpts, TuiComponent};
use crate::tui::{context, AppState};
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use std::io::StdoutLock;

pub(in crate::tui::window) struct ContextHelp {}

impl TuiComponent for ContextHelp {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        _: RenderOpts,
        _: &mut Debugger,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
            .split(rect);

        let help = Paragraph::new("Code walker, bug stalker")
            .style(Style::default().fg(Color::LightCyan))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(ratatui::widgets::Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title("Copyright")
                    .border_type(BorderType::Plain),
            );

        let state_text = match context::Context::current().state() {
            AppState::Initial => "Prepare to run",
            AppState::DebugeeRun => "Application running",
            AppState::DebugeeBreak => "Application paused",
            AppState::UserInput => "Wait for input",
            AppState::Finish => "Application finish",
        };

        let app_state = Paragraph::new(state_text)
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(ratatui::widgets::Borders::ALL)
                    .style(Style::default().fg(Color::White))
                    .title("State")
                    .border_type(BorderType::Rounded),
            );

        frame.render_widget(help, chunks[0]);
        frame.render_widget(app_state, chunks[1]);
    }

    fn handle_user_event(&mut self, _: KeyEvent) {}

    fn name(&self) -> &'static str {
        "context-help"
    }
}
