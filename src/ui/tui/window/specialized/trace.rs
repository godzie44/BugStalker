use crate::debugger::command::BacktraceCommand;
use crate::debugger::{command, Debugger, ThreadSnapshot};
use crate::ui::tui::window::specialized::PersistentList;
use crate::ui::tui::window::{RenderOpts, TuiComponent};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem};
use ratatui::Frame;
use std::cell::RefCell;
use std::io::StdoutLock;

pub struct ThreadTrace {
    thread_list: RefCell<PersistentList<ThreadSnapshot>>,
}

impl ThreadTrace {
    pub fn new() -> Self {
        Self {
            thread_list: RefCell::default(),
        }
    }
}

impl TuiComponent for ThreadTrace {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
        debugger: &mut Debugger,
    ) {
        let trace_cmd = command::Backtrace::new(debugger);
        let threads = trace_cmd.handle(BacktraceCommand::All).unwrap_or_default();
        self.thread_list.borrow_mut().update_items(threads);

        let list_items = self
            .thread_list
            .borrow()
            .items
            .iter()
            .map(|t_dump| {
                let as_text = format!("thread {}", t_dump.thread.pid);
                let mut list_item = ListItem::new(as_text);
                if t_dump.in_focus {
                    list_item = list_item.style(Style::default().fg(Color::Cyan))
                }
                list_item
            })
            .collect::<Vec<_>>();

        let border_style = if opts.in_focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let list = List::new(list_items)
            .block(
                Block::default()
                    .title("Threads")
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .border_type(BorderType::Rounded),
            )
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .bg(Color::LightRed)
                    .add_modifier(Modifier::ITALIC),
            );

        frame.render_stateful_widget(list, rect, &mut self.thread_list.borrow_mut().state)
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match e.code {
            KeyCode::Up => {
                self.thread_list.borrow_mut().previous();
            }
            KeyCode::Down => {
                self.thread_list.borrow_mut().next();
            }
            _ => {}
        }
    }

    fn name(&self) -> &'static str {
        "thread_trace"
    }
}
