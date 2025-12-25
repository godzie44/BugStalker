/// Utilities for communication between the debugger thread and UI threads
use crate::debugger::Debugger;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

type DebuggerSyncTask = dyn FnOnce(&mut Debugger) -> Box<dyn std::any::Any + Send + 'static> + Send;
type DebuggerAsyncTask = dyn FnOnce(&mut Debugger) -> anyhow::Result<()> + Send;

pub enum Request {
    Exit,
    ExitSync,
    SwitchUi,
    DebuggerSyncTask(Box<DebuggerSyncTask>),
    DebuggerAsyncTask(Box<DebuggerAsyncTask>),
}

pub struct ServerExchanger {
    requests: Receiver<Request>,
    responses: Sender<Box<dyn std::any::Any + Send + 'static>>,
    async_responses: Sender<anyhow::Error>,
}

impl ServerExchanger {
    pub fn next_request(&self) -> Option<Request> {
        self.requests.recv().ok()
    }

    pub fn send_response(&self, resp: Box<dyn std::any::Any + Send>) {
        _ = self.responses.send(resp);
    }

    pub fn send_async_response(&self, resp: anyhow::Error) {
        _ = self.async_responses.send(resp);
    }
}

pub struct ClientExchanger {
    messaging_enabled: AtomicBool,
    requests: Sender<Request>,
    responses: Receiver<Box<dyn std::any::Any + Send + 'static>>,
    async_responses: Receiver<anyhow::Error>,
}

unsafe impl Sync for ClientExchanger {}

#[derive(Debug, thiserror::Error)]
#[error("messaging disabled")]
pub struct MessagingDisabled;

impl ClientExchanger {
    #[inline(always)]
    pub fn is_messaging_enabled(&self) -> bool {
        self.messaging_enabled.load(Ordering::Relaxed)
    }

    /// Enable messaging between tracer and tui.
    #[inline(always)]
    pub fn enable_messaging(&self) {
        self.messaging_enabled.store(true, Ordering::Relaxed);
    }

    /// Disable messaging between tracer and tui, all requests will return [`MessagingDisabled`] error.
    #[inline(always)]
    pub fn disable_messaging(&self) {
        self.messaging_enabled.store(false, Ordering::Relaxed);
    }

    /// Send request to the debugger and wait for response.
    /// May return [`MessagingDisabled`] error if messaging is disabled now.
    pub fn request_sync<T, F>(&self, f: F) -> Result<T, MessagingDisabled>
    where
        T: Send + 'static,
        F: FnOnce(&mut Debugger) -> T + Send + 'static,
    {
        if !self.is_messaging_enabled() {
            return Err(MessagingDisabled);
        }

        let f = Box::new(
            |dbg: &mut Debugger| -> Box<dyn std::any::Any + Send + 'static> {
                let t = f(dbg);
                Box::new(t)
            },
        );
        _ = self.requests.send(Request::DebuggerSyncTask(f));
        let result = self.responses.recv().unwrap();
        Ok(*result.downcast::<T>().unwrap())
    }

    /// Send request to the debugger and return.
    /// Useful in situations when need to send command to debugger and no need to lock UI thread.
    /// May return [`MessagingDisabled`] error if messaging is disabled now.
    pub fn request_async<F>(&self, f: F) -> Result<(), MessagingDisabled>
    where
        F: FnOnce(&mut Debugger) -> anyhow::Result<()> + Send + 'static,
    {
        if !self.is_messaging_enabled() {
            return Err(MessagingDisabled);
        }

        let f = Box::new(|dbg: &mut Debugger| f(dbg));
        _ = self.requests.send(Request::DebuggerAsyncTask(f));
        Ok(())
    }

    pub fn send_exit(&self) {
        _ = self.requests.send(Request::Exit);
    }

    pub fn send_exit_sync(&self) {
        _ = self.requests.send(Request::ExitSync);
        _ = self.responses.recv().unwrap();
    }

    pub fn send_switch_ui(&self) {
        _ = self.requests.send(Request::SwitchUi);
    }

    /// Return response of last async debugger request or `None`.
    pub fn poll_async_resp(&self) -> Option<anyhow::Error> {
        self.async_responses.try_recv().ok()
    }
}

/// Create an exchanger pair.
/// Tui use exchanger to communicate with debugger by message passing.
/// Tui and debugger must be in separate threads,
/// because debugger is a tracer and can't be moving between threads.
///
/// [`ServerExchanger`] must be used at tracer (debugger) side and handle
/// incoming requests.
/// [`ClientExchanger`] must be used at tui side, send requests and receive responses.
pub fn exchanger() -> (ServerExchanger, ClientExchanger) {
    let (req_tx, req_rx) = channel::<Request>();
    let (resp_tx, resp_rx) = channel::<Box<dyn std::any::Any + Send + 'static>>();
    let (async_resp_tx, async_resp_rx) = channel::<anyhow::Error>();
    (
        ServerExchanger {
            requests: req_rx,
            responses: resp_tx,
            async_responses: async_resp_tx,
        },
        ClientExchanger {
            messaging_enabled: AtomicBool::new(true),
            requests: req_tx,
            responses: resp_rx,
            async_responses: async_resp_rx,
        },
    )
}
