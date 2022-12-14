use crate::cui::window::message::ActionMessage;
use crate::cui::window::{message, CuiComponent, RenderOpts};
use crate::cui::{context, AppState};
use crate::fire;
use crossterm::event::{KeyCode, KeyEvent};
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Style};
use tui::widgets::{Block, Borders};
use tui::Frame;
use tui_textarea::TextArea;

pub(in crate::cui::window) struct UserInput {
    textarea: TextArea<'static>,
    input_recipient_component: &'static str,
}

impl UserInput {
    pub(in crate::cui::window) fn new() -> Self {
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

impl CuiComponent for UserInput {
    fn render(&self, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect, _: RenderOpts) {
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

    fn update(&mut self) -> anyhow::Result<()> {
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
