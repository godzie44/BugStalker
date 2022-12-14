use crate::cui::hook::CuiHook;
use crate::cui::window::{CuiComponent, RenderOpts};
use crate::debugger::variable::RenderView;
use crate::debugger::{command, Debugger};
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::RefCell;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};
use tui::Frame;

pub struct Variables {
    debugger: Rc<Debugger<CuiHook>>,
    variables: RefCell<VariableList>,
}

impl Variables {
    pub fn new(debugger: impl Into<Rc<Debugger<CuiHook>>>) -> Self {
        Self {
            debugger: debugger.into(),
            variables: RefCell::default(),
        }
    }
}

impl CuiComponent for Variables {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
    ) {
        self.variables.borrow_mut().update(&self.debugger);
        let list_items = self
            .variables
            .borrow()
            .items
            .iter()
            .map(|view| {
                let as_text = if view.children.is_empty() {
                    format!(
                        "{}: {}({})",
                        view.name,
                        view.r#type,
                        view.value.as_deref().unwrap_or_default()
                    )
                } else {
                    format!("{}: {}(...)", view.name, view.r#type)
                };

                ListItem::new(as_text)
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
                    .title("Variables")
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .border_type(BorderType::Rounded),
            )
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
            .highlight_symbol(">>");

        frame.render_stateful_widget(list, rect, &mut self.variables.borrow_mut().state)
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match e.code {
            KeyCode::Up => {
                self.variables.borrow_mut().previous();
            }
            KeyCode::Down => {
                self.variables.borrow_mut().next();
            }
            _ => {}
        }
    }

    fn name(&self) -> &'static str {
        "variables"
    }
}

#[derive(Default)]
struct VariableList {
    items: Vec<RenderView>,
    state: ListState,
}

impl VariableList {
    fn update(&mut self, debugger: &Debugger<CuiHook>) {
        let cmd = command::Variables::new(debugger);
        let variables = cmd.run().unwrap_or_default();
        let views = variables
            .iter()
            .map(|v| v.render(debugger.pid))
            .collect::<Vec<_>>();
        self.items = views;
    }

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
}
