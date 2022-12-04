use crate::cui::window::{Action, CuiComponent, RenderContext};
use crossterm::event::{KeyCode, KeyEvent};
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::style::{Color, Style};
use tui::widgets::{Block, Borders};
use tui::Frame;
use tui_textarea::TextArea;

pub(super) struct UserInput {
    textarea: TextArea<'static>,
    input_requested_component: &'static str,
}

impl UserInput {
    pub(super) fn new() -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        textarea.set_style(Style::default().fg(Color::LightGreen));
        textarea.set_block(Block::default().borders(Borders::ALL).title("OK"));

        Self {
            textarea,
            input_requested_component: "",
        }
    }

    fn clear(&mut self) {
        self.textarea.delete_line_by_head();
        self.textarea.delete_line_by_end();
    }
}

impl CuiComponent for UserInput {
    fn render(
        &self,
        _: RenderContext,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
    ) {
        frame.render_widget(self.textarea.widget(), rect);
    }

    fn handle_user_event(&mut self, e: KeyEvent) -> Vec<Action> {
        match e.code {
            KeyCode::Esc => {
                self.clear();
                vec![Action::CancelUserInput]
            }
            KeyCode::Enter => {
                self.clear();
                vec![
                    Action::HandleUserInput(
                        self.input_requested_component,
                        self.textarea.lines()[0].to_string(),
                    ),
                    Action::CancelUserInput,
                ]
            }
            _ => {
                self.textarea.input(e);
                vec![]
            }
        }
    }

    fn apply_app_action(&mut self, behaviour: &[Action]) {
        for b in behaviour {
            if let Action::ActivateUserInput(component) = b {
                self.input_requested_component = component;
            }
        }
    }

    fn name(&self) -> &'static str {
        "user-input"
    }
}
