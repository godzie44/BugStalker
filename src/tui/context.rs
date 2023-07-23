use crate::tui::AppState;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tui::text::Text;

#[derive(Default)]
pub struct TrapData {
    debugee_file_name: Option<String>,
    debugee_text_pos: Option<u64>,
}

thread_local! {
    static CURRENT_CONTEXT: RefCell<Context> = RefCell::default();
}

#[derive(Clone)]
pub struct Context {
    trap: Rc<RefCell<TrapData>>,
    state: Rc<Cell<AppState>>,
    alert: Rc<RefCell<Option<Text<'static>>>>,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            trap: Rc::new(RefCell::default()),
            state: Rc::new(Cell::new(AppState::Initial)),
            alert: Rc::new(RefCell::default()),
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

    pub(super) fn trap_file_name(&self) -> Option<String> {
        self.trap.borrow().debugee_file_name.clone()
    }

    pub(super) fn set_trap_file_name(&self, name: String) {
        (*self.trap).borrow_mut().debugee_file_name = Some(name)
    }

    pub(super) fn take_trap_text_pos(&self) -> Option<u64> {
        (*self.trap).borrow_mut().debugee_text_pos.take()
    }

    pub(super) fn set_trap_text_pos(&self, pos: u64) {
        (*self.trap).borrow_mut().debugee_text_pos = Some(pos)
    }

    pub(super) fn alert(&self) -> Option<Text<'static>> {
        (*self.alert).borrow().clone()
    }

    pub(super) fn set_alert(&self, alert: Text<'static>) {
        *(*self.alert).borrow_mut() = Some(alert)
    }

    pub(super) fn drop_alert(&self) {
        (*self.alert).borrow_mut().take();
    }
}
