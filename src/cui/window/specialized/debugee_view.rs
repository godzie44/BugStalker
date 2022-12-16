use crate::cui::context;
use crate::cui::window::{CuiComponent, RenderOpts};
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io::{BufRead, StdoutLock};
use std::{fs, io};
use tui::backend::CrosstermBackend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

#[derive(Default)]
pub struct DebugeeView {
    file_cache: RefCell<HashMap<String, Vec<String>>>,
    current_file: RefCell<Option<String>>,
    current_file_len: Cell<u64>,
    current_scroll_pos: Cell<u64>,
}

impl DebugeeView {
    pub fn new() -> Self {
        Self {
            file_cache: RefCell::default(),
            current_file_len: Cell::default(),
            current_scroll_pos: Cell::default(),
            current_file: RefCell::default(),
        }
    }
}

impl DebugeeView {
    fn default_view(&self) -> Vec<Spans> {
        vec!["Welcome into BUG STALKER!".into()]
    }
}

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

        let mut cache = self.file_cache.borrow_mut();
        let spans = match ctx.trap_file_name() {
            None => self.default_view(),
            Some(file) => {
                let lines = match cache.entry(file.clone()) {
                    Entry::Occupied(o) => o.into_mut(),
                    Entry::Vacant(v) => {
                        let file = fs::File::open(file.clone()).unwrap();
                        let lines = io::BufReader::new(file)
                            .lines()
                            .enumerate()
                            .filter_map(|(num, line)| {
                                let line = line.map(|l| format!("{ln} {l}", ln = num + 1));
                                line.ok()
                            })
                            .collect::<Vec<_>>();

                        v.insert(lines)
                    }
                };

                let line_number = ctx.trap_text_pos().saturating_sub(1);

                // update current_scroll_pos when viewing new file
                if self
                    .current_file
                    .borrow()
                    .as_ref()
                    .map(|f| f != &file)
                    .unwrap_or(true)
                {
                    self.current_scroll_pos
                        .set(line_number.saturating_sub((frame.size().height - 6) as u64 / 2));
                    *self.current_file.borrow_mut() = Some(file);
                };

                self.current_file_len.set(lines.len() as u64);

                lines
                    .iter_mut()
                    .enumerate()
                    .map(|(i, l)| {
                        if i == line_number as usize {
                            Spans::from(Span::styled(
                                l.as_str(),
                                Style::default().bg(Color::LightRed),
                            ))
                        } else {
                            Spans::from(l.as_str())
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
