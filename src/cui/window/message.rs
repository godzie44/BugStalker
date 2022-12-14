use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub(super) enum ActionMessage {
    ActivateComponent { activate: &'static str },
    FocusOnComponent { focus_on: &'static str },
    ActivateUserInput { sender: &'static str },
    HandleUserInput { input: String },
    CancelUserInput {},
}

thread_local! {
    pub(super) static EXCHANGER: Exchanger = Exchanger::default()
}

#[derive(Clone, Default)]
pub(super) struct Exchanger {
    pub(super) buff: Rc<RefCell<HashMap<&'static str, Vec<ActionMessage>>>>,
}

impl Exchanger {
    pub(super) fn current() -> Exchanger {
        EXCHANGER.with(|q| q.clone())
    }

    pub(super) fn pop(&self, recipient: &'static str) -> Vec<ActionMessage> {
        (*self.buff)
            .borrow_mut()
            .remove(recipient)
            .unwrap_or_default()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.buff.borrow().is_empty()
    }
}

#[macro_export]
macro_rules! fire {
    ($action: expr => $recipient: expr) => {
        $crate::cui::window::message::EXCHANGER.with(|q| {
            let mut hm = (*q.buff).borrow_mut();
            let v = hm.entry($recipient).or_insert(vec![]);
            v.push($action);
        })
    };
}
