use std::io::{self, BufReader, BufWriter, Stdin, Stdout};
use std::sync::{Arc, Mutex};

use dap::errors::ServerError;
use dap::events::Event;
use dap::requests::{Command, LaunchRequestArguments, Request};
use dap::responses::{Response, ResponseBody, ResponseMessage};
use dap::server::{Server, ServerOutput};

pub struct DapServer {
    server: Server<Stdin, Stdout>,
    is_config_done: bool,
    buffered_launch_request: Option<(i64, LaunchRequestArguments)>,
}

impl DapServer {
    pub fn new() -> DapServer {
        let input = BufReader::new(io::stdin());
        let output = BufWriter::new(io::stdout());

        let server = Server::new(input, output);

        DapServer {
            server,
            is_config_done: false,
            buffered_launch_request: None,
        }
    }

    pub fn output(&self) -> Arc<Mutex<ServerOutput<Stdout>>> {
        self.server.output.clone()
    }

    pub fn poll_request(&mut self) -> Result<Option<Request>, ServerError> {
        // Vscode sends breakpoint configuration concurrently with the launch request for some reason. This
        // can be a problem if we get the breakpoints after launching, because if we set the breakpoints
        // while the program is running it might be too late to break.
        // This method buffers the launch request and makes sure that we have received a ConfigurationDone
        // from the client (ensuring that all breakpoints have been received) before emitting the buffered
        // launch request when next polled.

        if self.is_config_done {
            if let Some((seq, args)) = self.buffered_launch_request.take() {
                let req = Request {
                    seq,
                    command: Command::Launch(args),
                };

                log::debug!("{}: {:?}", req.seq, req.command);

                return Ok(Some(req));
            }
        }

        let Some(req) = self.server.poll_request()? else {
            return Ok(None);
        };

        if !self.is_config_done {
            if let Command::Launch(args) = req.command {
                self.buffered_launch_request = Some((req.seq, args));
                return self.poll_request();
            } else if let Command::ConfigurationDone = req.command {
                self.is_config_done = true;
            }
        }

        log::debug!("{}: {:?}", req.seq, req.command);

        Ok(Some(req))
    }

    pub fn respond_success(&mut self, seq: i64, body: ResponseBody) -> Result<(), ServerError> {
        self.server.respond(Response {
            request_seq: seq,
            success: true,
            message: None,
            body: Some(body), // to love
            error: None,
        })
    }

    pub fn respond_error(&mut self, seq: i64, error: impl Into<String>) -> Result<(), ServerError> {
        self.server.respond(Response {
            request_seq: seq,
            success: false,
            message: Some(ResponseMessage::Error(error.into())),
            body: None,
            error: None,
        })
    }

    pub fn respond_cancel(&mut self, seq: i64) -> Result<(), ServerError> {
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
