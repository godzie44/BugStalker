use crate::debugger::Debugger;
use crate::ui::tui::context;
use crate::ui::tui::window::{RenderOpts, TuiComponent};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use std::cell::{Cell, RefCell};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io::{BufRead, StdoutLock};
use std::{fs, io};

#[derive(Default)]
pub struct DebugeeView {
    file_cache: RefCell<HashMap<String, Vec<String>>>,
    current_file_len: Cell<u64>,
    current_scroll_pos: Cell<u64>,
    current_break_line: Cell<Option<u64>>,
}

impl DebugeeView {
    pub fn new() -> Self {
        Self {
            file_cache: RefCell::default(),
            current_file_len: Cell::default(),
            current_scroll_pos: Cell::default(),
            current_break_line: Cell::default(),
        }
    }
}

impl DebugeeView {
    fn default_view(&self) -> Vec<Line> {
        vec!["Welcome into BUG STALKER!".into()]
    }
}

impl TuiComponent for DebugeeView {
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

        let ctx = context::Context::current();

        let mut cache = self.file_cache.borrow_mut();
        let spans = match ctx.trap_file_name() {
            None => self.default_view(),
            Some(file) => {
                let lines = match cache.entry(file.clone()) {
                    Entry::Occupied(o) => o.into_mut(),
                    Entry::Vacant(v) => {
                        let lines = fs::File::open(file)
                            .map(|file| {
                                io::BufReader::new(file)
                                    .lines()
                                    .enumerate()
                                    .filter_map(|(num, line)| {
                                        let line = line.map(|l| format!("{ln} {l}", ln = num + 1));
                                        line.ok()
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_else(|_| vec!["Failed to open file".to_string()]);

                        v.insert(lines)
                    }
                };

                if let Some(break_line_number) = ctx.take_trap_text_pos() {
                    let break_line_number = break_line_number.saturating_sub(1);
                    self.current_break_line.set(Some(break_line_number));
                    self.current_scroll_pos.set(
                        break_line_number.saturating_sub((frame.size().height - 6) as u64 / 2),
                    );
                }

                self.current_file_len.set(lines.len() as u64);

                lines
                    .iter_mut()
                    .enumerate()
                    .map(|(i, l)| {
                        if self.current_break_line.get() == Some(i as u64) {
                            Line::from(Span::styled(
                                l.as_str(),
                                Style::default().bg(Color::LightRed),
                            ))
                        } else {
                            Line::from(l.as_str())
                        }
                    })
                    .collect()
            }
        };

        let view = Paragraph::new::<Text>(spans.into())
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border_style)
                    .style(Style::default().fg(Color::White))
                    .title(ctx.trap_file_name().unwrap_or_default()),
            )
            .scroll((self.current_scroll_pos.get() as u16, 0));

        frame.render_widget(view, rect);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match e.code {
            KeyCode::Up => {
                self.current_scroll_pos.set(
                    self.current_scroll_pos
                        .get()
                        .checked_sub(1)
                        .unwrap_or_default(),
                );
            }
            KeyCode::Down => {
                let next_pos = self.current_scroll_pos.get() + 1;
                if next_pos < self.current_file_len.get() {
                    self.current_scroll_pos.set(next_pos);
                }
            }
            _ => {}
        };
    }

    fn name(&self) -> &'static str {
        "debugee"
    }
}
