use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::Msg;
use tui_realm_stdlib::Label;
use tuirealm::{Component, Event, MockComponent};

#[derive(MockComponent)]
pub struct Stub {
    component: Label,
}

impl Stub {
    pub fn new(text: &str) -> Self {
        Self {
            component: Label::default().text(text),
        }
    }
}

impl Component<Msg, UserEvent> for Stub {
    fn on(&mut self, _ev: Event<UserEvent>) -> Option<Msg> {
        None
    }
}
