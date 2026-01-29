//! DAP transport abstraction layer.
//! Supports both stdio (for embedded mode) and TCP (for server mode).

use anyhow::anyhow;
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

/// Stdio-based DAP transport (for embedded mode in bs).
pub struct StdioTransport {
    reader: BufReader<Stdin>,
    writer: Stdout,
}

impl StdioTransport {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            reader: BufReader::new(std::io::stdin()),
            writer: std::io::stdout(),
        })
    }
}

impl DapTransport for StdioTransport {
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
        Ok(msg)
    }

    fn write_message(&mut self, message: &Value) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(message)?;
        writeln!(self.writer, "Content-Length: {}\r", payload.len())?;
        writeln!(self.writer, "\r")?;
        self.writer.write_all(&payload)?;
        self.writer.flush()?;
        Ok(())
    }
}

/// TCP-based DAP transport (for server mode).
pub struct TcpTransport {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl TcpTransport {
    pub fn new(stream: TcpStream) -> anyhow::Result<Self> {
        stream.set_nodelay(true)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { stream, reader })
    }
}

impl DapTransport for TcpTransport {
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
        Ok(msg)
    }

    fn write_message(&mut self, message: &Value) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(message)?;
        write!(self.stream, "Content-Length: {}\r\n\r\n", payload.len())?;
        self.stream.write_all(&payload)?;
        self.stream.flush()?;
        Ok(())
    }
}
