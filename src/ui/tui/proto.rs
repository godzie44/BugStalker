use crate::debugger::Debugger;
use std::sync::mpsc::{channel, Receiver, Sender};

type DebuggerSyncTask = dyn FnOnce(&mut Debugger) -> Box<dyn std::any::Any + Send + 'static> + Send;
type DebuggerAsyncTask = dyn FnOnce(&mut Debugger) -> anyhow::Result<()> + Send;

pub enum Request {
    Exit,
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
    requests: Sender<Request>,
    responses: Receiver<Box<dyn std::any::Any + Send + 'static>>,
    async_responses: Receiver<anyhow::Error>,
}

unsafe impl Sync for ClientExchanger {}

impl ClientExchanger {
    pub fn request_sync<T, F>(&self, f: F) -> T
    where
        T: Send + 'static,
        F: FnOnce(&mut Debugger) -> T + Send + 'static,
    {
        let f = Box::new(
            |dbg: &mut Debugger| -> Box<dyn std::any::Any + Send + 'static> {
                let t = f(dbg);
                Box::new(t)
            },
        );
        _ = self.requests.send(Request::DebuggerSyncTask(f));
        let result = self.responses.recv().unwrap();
        *result.downcast::<T>().unwrap()
    }

    pub fn request_async<F>(&self, f: F)
    where
        F: FnOnce(&mut Debugger) -> anyhow::Result<()> + Send + 'static,
    {
        let f = Box::new(|dbg: &mut Debugger| f(dbg));
        _ = self.requests.send(Request::DebuggerAsyncTask(f));
    }

    pub fn send_exit(&self) {
        _ = self.requests.send(Request::Exit);
    }

    pub fn send_switch_ui(&self) {
        _ = self.requests.send(Request::SwitchUi);
    }

    pub fn poll_async_resp(&self) -> Option<anyhow::Error> {
        self.async_responses.try_recv().ok()
    }
}

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
            requests: req_tx,
            responses: resp_rx,
            async_responses: async_resp_rx,
        },
    )
}
