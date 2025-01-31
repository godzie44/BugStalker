use crate::ui::command::{self, trigger::TriggerEvent};
use std::cell::{Cell, RefCell};

pub type UserProgram = Vec<(command::Command, String)>;

#[derive(Default)]
pub struct TriggerRegistry {
    previous_brkpt_or_wp: Cell<Option<TriggerEvent>>,
    list: RefCell<indexmap::IndexMap<TriggerEvent, UserProgram>>,
    active_event: RefCell<Option<TriggerEvent>>,
}

impl TriggerRegistry {
    pub fn set_previous_brkpt(&self, num: u32) {
        self.previous_brkpt_or_wp
            .set(Some(TriggerEvent::Breakpoint(num)));
    }

    pub fn set_previous_wp(&self, num: u32) {
        self.previous_brkpt_or_wp
            .set(Some(TriggerEvent::Watchpoint(num)));
    }

    pub fn get_previous_event(&self) -> Option<TriggerEvent> {
        self.previous_brkpt_or_wp.get()
    }

    pub fn add(&self, event: TriggerEvent, program: UserProgram) {
        if program.is_empty() {
            self.remove(event);
        } else {
            self.list.borrow_mut().insert(event, program);
        }
    }

    pub fn remove(&self, event: TriggerEvent) {
        self.list.borrow_mut().shift_remove(&event);
    }

    pub fn fire_event(&self, event: TriggerEvent) {
        if self.list.borrow().get(&event).is_some() {
            *self.active_event.borrow_mut() = Some(event);
        } else {
            *self.active_event.borrow_mut() = Some(TriggerEvent::Any);
        }
    }

    pub fn take_program(&self) -> Option<UserProgram> {
        let trigger = self.active_event.borrow_mut().take()?;
        self.list.borrow().get(&trigger).cloned()
    }

    pub fn for_each_trigger(&self, f: impl Fn(&TriggerEvent, &UserProgram)) {
        self.list.borrow().iter().for_each(|(k, v)| f(k, v));
    }
}
