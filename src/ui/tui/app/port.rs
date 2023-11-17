use crate::debugger::address::RelocatedAddress;
use crate::debugger::{EventHook, FunctionDie, PlaceDescriptor};
use crate::ui::tui::output::OutputLine;
use crate::ui::tui::proto::ClientExchanger;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use std::sync::{Arc, Mutex};
use tuirealm::listener::{ListenerResult, Poll};
use tuirealm::Event;

#[derive(Eq, Clone, PartialOrd)]
pub enum UserEvent {
    GotOutput(Vec<OutputLine>, usize),
    Breakpoint {
        pc: RelocatedAddress,
        num: u32,
        file: Option<String>,
        line: Option<u64>,
        function: Option<String>,
    },
    Step {
        pc: RelocatedAddress,
        file: Option<String>,
        line: Option<u64>,
        function: Option<String>,
    },
    Signal(Signal),
    Exit(i32),
    AsyncErrorResponse(String),
}

impl PartialEq for UserEvent {
    fn eq(&self, other: &Self) -> bool {
        match self {
            UserEvent::GotOutput(_, _) => matches!(other, UserEvent::GotOutput(_, _)),
            UserEvent::Breakpoint { .. } => matches!(other, UserEvent::Breakpoint { .. }),
            UserEvent::Step { .. } => {
                matches!(other, UserEvent::Step { .. })
            }
            UserEvent::Signal(_) => {
                matches!(other, UserEvent::Signal(_))
            }
            UserEvent::Exit(_) => {
                matches!(other, UserEvent::Exit(_))
            }
            UserEvent::AsyncErrorResponse(_) => {
                matches!(other, UserEvent::AsyncErrorResponse(_))
            }
        }
    }
}

pub struct OutputPort {
    output_buf: Arc<Mutex<Vec<OutputLine>>>,
    read_line_count: usize,
}

impl OutputPort {
    pub fn new(out_buf: Arc<Mutex<Vec<OutputLine>>>) -> Self {
        Self {
            output_buf: out_buf,
            read_line_count: 0,
        }
    }
}

impl Poll<UserEvent> for OutputPort {
    fn poll(&mut self) -> ListenerResult<Option<Event<UserEvent>>> {
        let lock = self.output_buf.lock().unwrap();
        if lock.len() != self.read_line_count {
            let event = UserEvent::GotOutput(lock.clone(), lock.len() - self.read_line_count);
            self.read_line_count = lock.len();
            return Ok(Some(Event::User(event)));
        }
        Ok(None)
    }
}

pub type DebuggerEventQueue = Arc<Mutex<Vec<UserEvent>>>;

pub struct TuiHook {
    event_queue: DebuggerEventQueue,
}

impl TuiHook {
    pub fn new(event_queue: DebuggerEventQueue) -> Self {
        Self { event_queue }
    }
}

impl EventHook for TuiHook {
    fn on_breakpoint(
        &self,
        pc: RelocatedAddress,
        num: u32,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        self.event_queue
            .lock()
            .unwrap()
            .push(UserEvent::Breakpoint {
                pc,
                num,
                file: place.as_ref().map(|p| p.file.to_string_lossy().to_string()),
                line: place.as_ref().map(|p| p.line_number),
                function: function.and_then(|f| f.base_attributes.name.clone()),
            });
        Ok(())
    }

    fn on_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        self.event_queue.lock().unwrap().push(UserEvent::Step {
            pc,
            file: place.as_ref().map(|p| p.file.to_string_lossy().to_string()),
            line: place.as_ref().map(|p| p.line_number),
            function: function.and_then(|f| f.base_attributes.name.clone()),
        });
        Ok(())
    }

    fn on_signal(&self, signal: Signal) {
        self.event_queue
            .lock()
            .unwrap()
            .push(UserEvent::Signal(signal));
    }

    fn on_exit(&self, code: i32) {
        self.event_queue.lock().unwrap().push(UserEvent::Exit(code));
    }

    fn on_process_install(&self, _pid: Pid) {}
}

pub struct DebuggerEventsPort {
    event_queue: DebuggerEventQueue,
}

impl DebuggerEventsPort {
    pub fn new(event_queue: DebuggerEventQueue) -> Self {
        Self { event_queue }
    }
}

impl Poll<UserEvent> for DebuggerEventsPort {
    fn poll(&mut self) -> ListenerResult<Option<Event<UserEvent>>> {
        if let Some(event) = self.event_queue.lock().unwrap().pop() {
            return Ok(Some(Event::User(event)));
        }
        Ok(None)
    }
}

pub struct AsyncResponsesPort {
    exchanger: Arc<ClientExchanger>,
}

impl AsyncResponsesPort {
    pub fn new(exchanger: Arc<ClientExchanger>) -> Self {
        Self { exchanger }
    }
}

impl Poll<UserEvent> for AsyncResponsesPort {
    fn poll(&mut self) -> ListenerResult<Option<Event<UserEvent>>> {
        Ok(self
            .exchanger
            .poll_async_resp()
            .map(|err| Event::User(UserEvent::AsyncErrorResponse(format!("{:#}", err)))))
    }
}
