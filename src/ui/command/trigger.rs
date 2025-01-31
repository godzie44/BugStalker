use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum TriggerEvent {
    Breakpoint(u32),
    Watchpoint(u32),
    Any,
}

impl Display for TriggerEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerEvent::Breakpoint(num) => write!(f, "Breakpoint {num}"),
            TriggerEvent::Watchpoint(num) => write!(f, "Watchpoint {num}"),
            TriggerEvent::Any => write!(f, "Any breakpoint or watchpoint"),
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Command {
    AttachToPreviouslyCreated,
    AttachToDefined(TriggerEvent),
    Info,
}
