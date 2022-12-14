use crate::cui::context;
use crate::cui::window::{Action, CuiComponent, RenderOpts};
use crossterm::event::{KeyCode, KeyEvent};
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use tui::Frame;

#[derive(Default)]
pub(super) struct Alert {}

impl CuiComponent for Alert {
    fn render(&self, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect, _: RenderOpts) {
        if let Some(txt) = context::Context::current().alert() {
            let block = Block::default()
                .title("Alert!")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded);
            let paragraph = Paragraph::new(txt)
                .style(Style::default().bg(Color::Black))
                .block(block)
                .alignment(Alignment::Center);

            let alert_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Percentage(35),
                        Constraint::Percentage(30),
                        Constraint::Percentage(35),
                    ]
                    .as_ref(),
                )
                .split(rect);

            let alert_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(
                    [
                        Constraint::Percentage(20),
                        Constraint::Percentage(60),
                        Constraint::Percentage(20),
                    ]
                    .as_ref(),
                )
                .split(alert_layout[1])[1];

            frame.render_widget(Clear, alert_layout); //this clears out the background
            frame.render_widget(paragraph, alert_layout);
        }
    }

    fn handle_user_event(&mut self, e: KeyEvent) -> Vec<Action> {
        match e.code {
            KeyCode::Esc | KeyCode::Enter => {
                context::Context::current().drop_alert();
            }
            _ => {}
        }
        vec![]
    }

    fn name(&self) -> &'static str {
        "alert"
    }
}
