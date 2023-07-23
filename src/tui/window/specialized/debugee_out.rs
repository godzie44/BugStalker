use crate::tui::window::{RenderOpts, TuiComponent};
use crate::tui::{DebugeeStreamBuffer, StreamLine};
use crossterm::event::KeyEvent;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

pub struct DebugeeOut {
    stream_buff: DebugeeStreamBuffer,
}

impl DebugeeOut {
    pub fn new(stream_buff: DebugeeStreamBuffer) -> Self {
        Self { stream_buff }
    }
}

impl TuiComponent for DebugeeOut {
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

        let text = self
            .stream_buff
            .data
            .lock()
            .unwrap()
            .iter()
            .map(|line| {
                let span = match line {
                    StreamLine::Out(stdout_line) => Span::raw(stdout_line.to_string()),
                    StreamLine::Err(stderr_line) => {
                        Span::styled(stderr_line.to_string(), Style::default().fg(Color::Red))
                    }
                };
                Spans::from(vec![span])
            })
            .collect::<Vec<_>>();

        let home = Paragraph::new(text).alignment(Alignment::Left).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .style(Style::default().fg(Color::White))
                .title("Output"),
        );
        frame.render_widget(home, rect);
    }

    fn handle_user_event(&mut self, _: KeyEvent) {}

    fn name(&self) -> &'static str {
        "output"
    }
}
