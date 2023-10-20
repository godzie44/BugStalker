use crate::debugger::variable::render::{RenderRepr, ValueLayout};
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger};
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

pub struct Variables {
    variables: RefCell<PersistentList<VariableIR>>,
}

impl Variables {
    pub fn new() -> Self {
        Self {
            variables: RefCell::default(),
        }
    }
}

impl TuiComponent for Variables {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
        debugger: &mut Debugger,
    ) {
        let cmd = command::Variables::new(debugger);
        let variables = cmd
            .handle(Expression::Variable(VariableSelector::Any))
            .unwrap_or_default();
        self.variables.borrow_mut().update_items(variables);

        let list_items = self
            .variables
            .borrow_mut()
            .items
            .iter_mut()
            .map(|view| {
                let val = match view.value() {
                    None => String::default(),
                    Some(ref val) => match val {
                        ValueLayout::PreRendered(r) => r.to_string(),
                        ValueLayout::Referential { addr, .. } => {
                            format!("{addr:p} (...)")
                        }
                        ValueLayout::Wrapped(_) => "(...)".to_string(),
                        ValueLayout::Nested { .. } => "(...)".to_string(),
                        ValueLayout::Map(_) => "(...)".to_string(),
                    },
                };

                let as_text = match view {
                    VariableIR::CEnum(_) | VariableIR::RustEnum(_) => {
                        format!("{}: {}::{}", view.name(), view.r#type(), val)
                    }
                    _ => format!("{}: {}({})", view.name(), view.r#type(), val),
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
            .highlight_style(
                Style::default()
                    .bg(Color::LightRed)
                    .add_modifier(Modifier::ITALIC),
            );

        frame.render_stateful_widget(list, rect, &mut self.variables.borrow_mut().state)
    }

    fn handle_user_event(&mut self, e: KeyEvent, _: &mut Debugger) {
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
