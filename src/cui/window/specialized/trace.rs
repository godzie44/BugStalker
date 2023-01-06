use crate::cui::hook::CuiHook;
use crate::cui::window::specialized::PersistentList;
use crate::cui::window::{CuiComponent, RenderOpts};
use crate::debugger::{command, Debugger, ThreadDump};
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::RefCell;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, BorderType, Borders, List, ListItem};
use tui::Frame;

pub struct ThreadTrace {
    debugger: Rc<RefCell<Debugger<CuiHook>>>,
    thread_list: RefCell<PersistentList<ThreadDump>>,
}

impl ThreadTrace {
    pub fn new(debugger: impl Into<Rc<RefCell<Debugger<CuiHook>>>>) -> Self {
        Self {
            debugger: debugger.into(),
            thread_list: RefCell::default(),
        }
    }
}

impl CuiComponent for ThreadTrace {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        let debugger = self.debugger.borrow();
        let trace_cmd = command::Trace::new(&debugger);
        let threads = trace_cmd.run();
        self.thread_list.borrow_mut().update_items(threads);

        let list_items = self
            .thread_list
            .borrow()
            .items
            .iter()
            .map(|t_dump| {
                let as_text = format!("thread {} ({})", t_dump.thread.num, t_dump.thread.pid);
                let mut list_item = ListItem::new(as_text);
                if t_dump.on_focus {
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
