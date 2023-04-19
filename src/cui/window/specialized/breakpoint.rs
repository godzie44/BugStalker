use crate::cui::window::message::{ActionMessage, Exchanger};
use crate::cui::window::specialized::PersistentList;
use crate::cui::window::{CuiComponent, RenderOpts};
use crate::debugger::command::{Breakpoint, BreakpointCommand, Command};
use crate::debugger::{command, Debugger};
use crate::fire;
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::RefCell;
use std::io::StdoutLock;
use std::rc::Rc;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, BorderType, Borders, List, ListItem};
use tui::Frame;

pub struct Breakpoints {
    debugger: Rc<RefCell<Debugger>>,
    breakpoints: RefCell<PersistentList<Breakpoint>>,
}

impl Breakpoints {
    pub fn new(debugger: impl Into<Rc<RefCell<Debugger>>>) -> Self {
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
                    Breakpoint::Address(addr) => {
                        format!("{bp_index}. {addr:#016X}")
                    }
                    Breakpoint::Line(file, line) => {
                        format!("{bp_index}. {file}:{line}")
                    }
                    Breakpoint::Function(function) => {
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
            .highlight_style(
                Style::default()
                    .bg(Color::LightRed)
                    .add_modifier(Modifier::ITALIC),
            );

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

    fn update(&mut self) -> anyhow::Result<()> {
        for action in Exchanger::current().pop(self.name()) {
            if let ActionMessage::HandleUserInput { input } = action {
                let dbg = &mut (*self.debugger).borrow_mut();
                let command = Command::parse(&input)?;
                if let Command::Breakpoint(BreakpointCommand::Add(brkpt)) = command {
                    command::Break::new(dbg).handle(BreakpointCommand::Add(brkpt.clone()))?;
                    self.breakpoints.borrow_mut().items().push(brkpt);
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "breakpoints"
    }
}
