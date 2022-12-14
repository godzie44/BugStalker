use crate::cui::context;
use crate::cui::window::{CuiComponent, RenderOpts};
use crossterm::event::{KeyCode, KeyEvent};
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

#[derive(Default)]
pub struct DebugeeView {}

impl CuiComponent for DebugeeView {
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
        let ctx = context::Context::current();
        let home = Paragraph::new(ctx.render_text())
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border_style)
                    .style(Style::default().fg(Color::White))
                    .title(ctx.render_file_name()),
            )
            .scroll((ctx.render_text_pos() as u16, 0));
        frame.render_widget(home, rect);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match e.code {
            KeyCode::Up => {
                let ctx = context::Context::current();
                let current_pos = ctx.render_text_pos();

                ctx.set_render_text_pos(current_pos.checked_sub(1).unwrap_or_default())
            }
            KeyCode::Down => {
                let ctx = context::Context::current();
                let current_pos = ctx.render_text_pos();

                ctx.set_render_text_pos(current_pos.checked_add(1).unwrap_or_default());
            }
            _ => {}
        };
    }

    fn name(&self) -> &'static str {
        "debugee"
    }
}
