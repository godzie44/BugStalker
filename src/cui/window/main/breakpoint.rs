use crate::cui::hook::CuiHook;
use crate::cui::window::message::{ActionMessage, Exchanger};
use crate::cui::window::{CuiComponent, RenderOpts};
use crate::debugger::command::BreakpointType;
use crate::debugger::{command, Debugger};
use crate::fire;
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::RefCell;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};
use tui::Frame;

pub struct Breakpoints {
    debugger: Rc<Debugger<CuiHook>>,
    breakpoints: RefCell<BreakpointList>,
}

impl Breakpoints {
    pub fn new(debugger: impl Into<Rc<Debugger<CuiHook>>>) -> Self {
        Self {
            debugger: debugger.into(),
            breakpoints: RefCell::default(),
        }
    }
}

impl CuiComponent for Breakpoints {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        let items: Vec<ListItem> = self
            .breakpoints
            .borrow()
            .items
            .iter()
            .enumerate()
            .map(|(i, bp)| {
                let bp_index = i + 1;
                let view = match bp {
                    BreakpointType::Address(addr) => {
                        format!("{bp_index}. {addr:#016X}")
                    }
                    BreakpointType::Line(file, line) => {
                        format!("{bp_index}. {file}:{line}")
                    }
                    BreakpointType::Function(function) => {
                        format!("{bp_index}. {function}")
                    }
                };
                ListItem::new(view)
            })
            .collect();

        let border_style = if opts.in_focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .title("Breakpoints")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(border_style),
            )
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
            .highlight_symbol(">>");

        frame.render_stateful_widget(list, rect, &mut self.breakpoints.borrow_mut().state);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match e.code {
            KeyCode::Char('a') => {
                fire!(ActionMessage::ActivateUserInput {sender: self.name()} => "app_window")
            }
            KeyCode::Char('r') => {
                self.breakpoints.borrow_mut().remove_selected();
            }
            KeyCode::Up => {
                self.breakpoints.borrow_mut().previous();
            }
            KeyCode::Down => {
                self.breakpoints.borrow_mut().next();
            }
            _ => {}
        }
    }

    fn update(&mut self) {
        for action in Exchanger::current().pop(self.name()) {
            if let ActionMessage::HandleUserInput { input } = action {
                let b = command::Break::new(&self.debugger, vec!["", &input]).unwrap();
                b.run().unwrap();
                self.breakpoints.borrow_mut().add(b.r#type);
            }
        }
    }

    fn name(&self) -> &'static str {
        "breakpoints"
    }
}

#[derive(Default)]
struct BreakpointList {
    items: Vec<BreakpointType>,
    state: ListState,
}

impl BreakpointList {
    fn next(&mut self) {
        if self.items.is_empty() {
            self.state.select(None);
            return;
        }

        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.items.is_empty() {
            self.state.select(None);
            return;
        }

        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn add(&mut self, bp: BreakpointType) {
        self.items.push(bp);
    }

    fn remove_selected(&mut self) {
        let i = match self.state.selected() {
            Some(i) => i,
            None => return,
        };
        self.items.remove(i);
        self.previous();
    }
}
