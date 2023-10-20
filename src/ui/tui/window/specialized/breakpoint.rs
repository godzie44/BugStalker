use crate::debugger::command::r#break::Break;
use crate::debugger::command::r#break::Command as BreakpointCommand;
use crate::debugger::command::Command;
use crate::debugger::{BreakpointViewOwned, Debugger};
use crate::fire;
use crate::ui::tui::window::message::{ActionMessage, Exchanger};
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

pub struct Breakpoints {
    breakpoints: RefCell<PersistentList<BreakpointViewOwned>>,
}

impl Breakpoints {
    pub fn new() -> Self {
        Self {
            breakpoints: RefCell::default(),
        }
    }
}

impl TuiComponent for Breakpoints {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        opts: RenderOpts,
        debugger: &mut Debugger,
    ) {
        self.breakpoints.borrow_mut().items = debugger
            .breakpoints_snapshot()
            .into_iter()
            .map(|view| view.to_owned())
            .collect();

        let items: Vec<ListItem> = self
            .breakpoints
            .borrow()
            .items
            .iter()
            .map(|brkpt| {
                let bp_index = brkpt.number;
                let view = if let Some(ref place) = brkpt.place {
                    format!("{bp_index}. {:?}:{}", place.file, place.line_number)
                } else {
                    format!("{bp_index}. {}", brkpt.addr)
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

    fn handle_user_event(&mut self, e: KeyEvent, debugger: &mut Debugger) {
        match e.code {
            KeyCode::Char('a') => {
                fire!(ActionMessage::ActivateUserInput {sender: self.name()} => "app_window")
            }
            KeyCode::Char('r') => {
                let removed = self.breakpoints.borrow_mut().remove_selected();
                _ = debugger.remove_breakpoints_at_addresses(removed.map(|r| r.addr).into_iter());
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

    fn update(&mut self, debugger: &mut Debugger) -> anyhow::Result<()> {
        for action in Exchanger::current().pop(self.name()) {
            if let ActionMessage::HandleUserInput { input } = action {
                let command = Command::parse(&input)?;
                if let Command::Breakpoint(BreakpointCommand::Add(brkpt)) = command {
                    Break::new(debugger).handle(&BreakpointCommand::Add(brkpt.clone()))?;
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "breakpoints"
    }
}
