use crate::cui::window::{Action, CuiComponent};
use crate::cui::{AppContext, AppState};
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
    fn render(&self, _: AppContext, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect) {
        frame.render_widget(self.textarea.widget(), rect);
    }

    fn handle_user_event(&mut self, ctx: AppContext, e: KeyEvent) -> Vec<Action> {
        match e.code {
            KeyCode::Esc => {
                self.clear();
                //todo state history
                ctx.change_state(AppState::DebugeeRun);
                vec![Action::CancelUserInput]
            }
            KeyCode::Enter => {
                let text = self.textarea.lines()[0].to_string();
                self.clear();
                ctx.change_state(AppState::DebugeeRun);
                vec![
                    Action::HandleUserInput(self.input_requested_component, text),
                    Action::CancelUserInput,
                ]
            }
            _ => {
                self.textarea.input(e);
                vec![]
            }
        }
    }

    fn apply_app_action(&mut self, ctx: AppContext, behaviour: &[Action]) {
        for b in behaviour {
            if let Action::ActivateUserInput(component) = b {
                ctx.change_state(AppState::UserInput);
                self.input_requested_component = component;
            }
        }
    }

    fn name(&self) -> &'static str {
        "user-input"
    }
}
