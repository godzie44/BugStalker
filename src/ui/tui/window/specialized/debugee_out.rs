use crate::debugger::Debugger;
use crate::ui::tui::output::OutputLine;
use crate::ui::tui::window::{RenderOpts, TuiComponent};
use crate::ui::tui::DebugeeStreamBuffer;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use std::io::StdoutLock;

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
        _: &mut Debugger,
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
                    OutputLine::Out(stdout_line) => Span::raw(stdout_line.to_string()),
                    OutputLine::Err(stderr_line) => {
                        Span::styled(stderr_line.to_string(), Style::default().fg(Color::Red))
                    }
                };
                Line::from(vec![span])
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
