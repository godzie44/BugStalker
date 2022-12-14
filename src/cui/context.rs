use crate::cui::AppState;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};

pub struct RenderData {
    debugee_file_name: String,
    debugee_text: Text<'static>,
    debugee_text_pos: u64,
    alert: Option<Text<'static>>,
}

impl Default for RenderData {
    fn default() -> Self {
        Self {
            debugee_file_name: String::new(),
            debugee_text: vec![
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::raw("Welcome")]),
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::raw("to")]),
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::styled(
                    "BUG STALKER",
                    Style::default().fg(Color::LightBlue),
                )]),
                Spans::from(vec![Span::raw("")]),
            ]
            .into(),
            debugee_text_pos: 0,
            alert: None,
        }
    }
}

thread_local! {
    static CURRENT_CONTEXT: RefCell<Context> = RefCell::default();
}

#[derive(Clone)]
pub struct Context {
    data: Rc<RefCell<RenderData>>,
    state: Rc<Cell<AppState>>,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            data: Rc::new(RefCell::default()),
            state: Rc::new(Cell::new(AppState::Initial)),
        }
    }
}

impl Context {
    pub(super) fn current() -> Self {
        CURRENT_CONTEXT.with(|ctx| ctx.borrow().clone())
    }

    pub(super) fn change_state(&self, new: AppState) {
        self.state.set(new);
    }

    pub(super) fn state(&self) -> AppState {
        self.state.get()
    }

    pub(super) fn assert_state(&self, expected_state: AppState) -> bool {
        self.state.get() == expected_state
    }

    pub(super) fn render_file_name(&self) -> String {
        self.data.borrow().debugee_file_name.clone()
    }

    pub(super) fn set_render_file_name(&self, name: String) {
        (*self.data).borrow_mut().debugee_file_name = name
    }

    pub(super) fn render_text(&self) -> Text<'static> {
        self.data.borrow().debugee_text.clone()
    }

    pub(super) fn set_render_text(&self, text: Text<'static>) {
        (*self.data).borrow_mut().debugee_text = text
    }

    pub(super) fn render_text_pos(&self) -> u64 {
        self.data.borrow().debugee_text_pos
    }

    pub(super) fn set_render_text_pos(&self, pos: u64) {
        (*self.data).borrow_mut().debugee_text_pos = pos
    }

    pub(super) fn alert(&self) -> Option<Text<'static>> {
        self.data.borrow().alert.clone()
    }

    pub(super) fn set_alert(&self, alert: Text<'static>) {
        (*self.data).borrow_mut().alert = Some(alert)
    }

    pub(super) fn drop_alert(&self) {
        (*self.data).borrow_mut().alert.take();
    }
}
