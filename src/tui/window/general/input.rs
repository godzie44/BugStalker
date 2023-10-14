use crate::debugger::Debugger;
use crate::fire;
use crate::tui::window::message::ActionMessage;
use crate::tui::window::{message, RenderOpts, TuiComponent};
use crate::tui::{context, AppState};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use std::io::StdoutLock;
use tui_textarea::TextArea;

pub(in crate::tui::window) struct UserInput {
    textarea: TextArea<'static>,
    input_recipient_component: &'static str,
}

impl UserInput {
    pub(in crate::tui::window) fn new() -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        textarea.set_style(Style::default().fg(Color::LightGreen));
        textarea.set_block(Block::default().borders(Borders::ALL).title("OK"));

        Self {
            textarea,
            input_recipient_component: "",
        }
    }

    fn clear(&mut self) {
        self.textarea.delete_line_by_head();
        self.textarea.delete_line_by_end();
    }
}

impl TuiComponent for UserInput {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        _: RenderOpts,
        _: &mut Debugger,
    ) {
        frame.render_widget(self.textarea.widget(), rect);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        match e.code {
            KeyCode::Esc => {
                self.clear();
                //todo state history
                context::Context::current().change_state(AppState::DebugeeRun);
                fire!(message::ActionMessage::CancelUserInput {} => "app_window");
            }
            KeyCode::Enter => {
                let handle_action = message::ActionMessage::HandleUserInput {
                    input: self.textarea.lines()[0].to_string(),
                };
                self.clear();
                context::Context::current().change_state(AppState::DebugeeRun);
                fire!(handle_action => self.input_recipient_component);
                fire!(message::ActionMessage::CancelUserInput {} => "app_window");
            }
            _ => {
                self.textarea.input(e);
            }
        }
    }

    fn update(&mut self, _: &mut Debugger) -> anyhow::Result<()> {
        message::Exchanger::current()
            .pop(self.name())
            .into_iter()
            .for_each(|act| {
                if let ActionMessage::ActivateUserInput { sender } = act {
                    context::Context::current().change_state(AppState::UserInput);
                    self.input_recipient_component = sender;
                }
            });

        Ok(())
    }

    fn name(&self) -> &'static str {
        "user-input"
    }
}
