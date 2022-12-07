use crate::cui::hook::CuiHook;
use crate::cui::window::{Action, CuiComponent};
use crate::cui::AppContext;
use crate::debugger::command::BreakpointType;
use crate::debugger::{command, Debugger};
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::RefCell;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, Borders, List, ListItem, ListState};
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
        _ctx: AppContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
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
        let list = List::new(items)
            .block(Block::default().title("List").borders(Borders::ALL))
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
            .highlight_symbol(">>");

        frame.render_stateful_widget(list, rect, &mut self.breakpoints.borrow_mut().state);
    }

    fn handle_user_event(&mut self, _: AppContext, e: KeyEvent) -> Vec<Action> {
        match e.code {
            KeyCode::Char('a') => {
                vec![Action::ActivateUserInput(self.name())]
            }
            KeyCode::Char('r') => {
                self.breakpoints.borrow_mut().remove_selected();
                vec![]
            }
            KeyCode::Up => {
                self.breakpoints.borrow_mut().previous();
                vec![]
            }
            KeyCode::Down => {
                self.breakpoints.borrow_mut().next();
                vec![]
            }
            _ => {
                vec![]
            }
        }
    }

    fn apply_app_action(&mut self, _: AppContext, actions: &[Action]) {
        for action in actions {
            match action {
                Action::HandleUserInput(component, input) if (*component == self.name()) => {
                    let b = command::Break::new(&self.debugger, vec!["", input]).unwrap();
                    b.run().unwrap();
                    self.breakpoints.borrow_mut().add(b.r#type);
                }
                _ => {}
            }
        }
    }

    fn name(&self) -> &'static str {
        "main.left.breakpoints"
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
