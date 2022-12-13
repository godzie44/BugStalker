use crate::cui::window::{Action, CuiComponent, RenderOpts};
use crate::cui::{AppContext, DebugeeStreamBuffer, StreamLine};
use crossterm::event::{KeyCode, KeyEvent};
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

pub(super) mod breakpoint;
pub(super) mod variable;

pub(super) struct DebugeeView {}

impl DebugeeView {
    pub(super) fn new() -> Self {
        Self {}
    }
}

impl CuiComponent for DebugeeView {
    fn render(
        &self,
        ctx: AppContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        let border_style = if opts.in_focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let home = Paragraph::new(ctx.data.debugee_text.clone().take())
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border_style)
                    .style(Style::default().fg(Color::White))
                    .title(ctx.data.debugee_file_name.borrow().to_string()),
            )
            .scroll((ctx.data.debugee_text_pos.get() as u16, 0));
        frame.render_widget(home, rect);
    }

    fn handle_user_event(&mut self, ctx: AppContext, e: KeyEvent) -> Vec<Action> {
        match e.code {
            KeyCode::Up => ctx.data.debugee_text_pos.set(
                ctx.data
                    .debugee_text_pos
                    .get()
                    .checked_sub(1)
                    .unwrap_or_default(),
            ),
            KeyCode::Down => ctx.data.debugee_text_pos.set(
                ctx.data
                    .debugee_text_pos
                    .get()
                    .checked_add(1)
                    .unwrap_or_default(),
            ),
            _ => {}
        };
        vec![]
    }

    fn name(&self) -> &'static str {
        "debugee"
    }
}

pub(super) struct Logs {}

impl CuiComponent for Logs {
    fn render(
        &self,
        _ctx: AppContext,
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

    fn handle_user_event(&mut self, _: AppContext, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "logs"
    }
}

pub(super) struct DebugeeOut {
    stream_buff: DebugeeStreamBuffer,
}

impl DebugeeOut {
    pub(super) fn new(stream_buff: DebugeeStreamBuffer) -> Self {
        Self { stream_buff }
    }
}

impl CuiComponent for DebugeeOut {
    fn render(
        &self,
        _: AppContext,
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

    fn handle_user_event(&mut self, _: AppContext, _: KeyEvent) -> Vec<Action> {
        vec![]
    }

    fn name(&self) -> &'static str {
        "output"
    }
}
