use std::io::{self, BufReader, BufWriter, Stdin, Stdout};
use std::sync::{Arc, Mutex};

use dap::errors::ServerError;
use dap::events::Event;
use dap::requests::Request;
use dap::responses::{Response, ResponseBody, ResponseMessage};
use dap::server::{Server, ServerOutput};

pub struct DapServer {
    server: Server<Stdin, Stdout>,
}

impl DapServer {
    pub fn new() -> DapServer {
        let input = BufReader::new(io::stdin());
        let output = BufWriter::new(io::stdout());

        let server = Server::new(input, output);

        DapServer { server }
    }

    pub fn output(&self) -> Arc<Mutex<ServerOutput<Stdout>>> {
        self.server.output.clone()
    }

    pub fn poll_request(&mut self) -> Result<Option<Request>, ServerError> {
        let Some(req) = self.server.poll_request()? else {
            return Ok(None);
        };

        log::debug!("{}: {:?}", req.seq, req.command);

        Ok(Some(req))
    }

    pub fn respond_success(&mut self, seq: i64, body: ResponseBody) -> Result<(), ServerError> {
        log::debug!("success {seq}: {body:?}");

        self.server.respond(Response {
            request_seq: seq,
            success: true,
            message: None,
            body: Some(body), // to love
            error: None,
        })
    }

    pub fn respond_error(&mut self, seq: i64, error: impl Into<String>) -> Result<(), ServerError> {
        let error = error.into();

        log::debug!("error {seq}: {error}");

        self.server.respond(Response {
            request_seq: seq,
            success: false,
            message: Some(ResponseMessage::Error(error)),
            body: None,
            error: None,
        })
    }

    pub fn respond_cancel(&mut self, seq: i64) -> Result<(), ServerError> {
        log::debug!("cancel {seq}");

        self.server.respond(Response {
            request_seq: seq,
            success: false,
            message: Some(ResponseMessage::Cancelled),
            body: None,
            error: None,
        })
    }

    pub fn send_event(&mut self, event: Event) -> Result<(), ServerError> {
        self.server.send_event(event)
    }
}
