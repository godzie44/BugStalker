//! DAP transport abstraction layer.
//! Supports both stdio (for embedded mode) and TCP (for server mode).

use crate::dap::tracer::FileTracer;
use anyhow::anyhow;
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Stdin, Stdout, Write};
use std::net::TcpStream;

/// Trait for DAP message transport (stdio or TCP).
pub trait DapTransport: Send {
    /// Read a single DAP message (with Content-Length framing).
    fn read_message(&mut self) -> anyhow::Result<Value>;

    /// Write a single DAP message (with Content-Length framing).
    fn write_message(&mut self, message: &Value) -> anyhow::Result<()>;
}

pub struct Transport<W: Send, R: Send> {
    writer: W,
    reader: BufReader<R>,
    tracer: Option<FileTracer>,
}

impl<W: Send, R: Send> Transport<W, R> {
    fn trace<T: Serialize>(&self, prefix: &'static str, data: &T) {
        if let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(data)
        {
            tracer.line(&format!("{prefix} {line}"));
        }
    }
}

impl<W: Write + Send, R: Read + Send> DapTransport for Transport<W, R> {
    fn read_message(&mut self) -> anyhow::Result<Value> {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let read_n = self.reader.read_line(&mut line)?;
            if read_n == 0 {
                return Err(anyhow!("DAP connection closed"));
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                content_length = Some(v.trim().parse()?);
            }
        }

        let len = content_length.ok_or_else(|| anyhow!("Missing Content-Length header"))?;
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf)?;
        let msg: Value = serde_json::from_slice(&buf)?;
        self.trace("<-", &msg);
        Ok(msg)
    }

    fn write_message(&mut self, message: &Value) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(message)?;
        self.trace("->", &message);
        write!(self.writer, "Content-Length: {}\r\n\r\n", payload.len())?;
        self.writer.write_all(&payload)?;
        self.writer.flush()?;
        Ok(())
    }
}

/// TCP-based DAP transport (for server mode).
pub fn new_tcp_transport(
    stream: TcpStream,
    tracer: Option<FileTracer>,
) -> anyhow::Result<Transport<TcpStream, TcpStream>> {
    stream.set_nodelay(true)?;
    let reader = BufReader::new(stream.try_clone()?);
    Ok(Transport {
        writer: stream,
        reader,
        tracer,
    })
}

/// Stdio-based DAP transport (for embedded mode in bs).
pub fn new_stdio_transport(tracer: Option<FileTracer>) -> Transport<Stdout, Stdin> {
    Transport {
        reader: BufReader::new(std::io::stdin()),
        writer: std::io::stdout(),
        tracer,
    }
}
