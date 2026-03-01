use crate::dap::transport::DapTransport;
use crate::dap::yadap::tracer::FileTracer;
use anyhow::anyhow;
use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Small helper for DAP framing.
pub struct DapIo {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    tracer: Option<FileTracer>,
    trace: bool,
}

impl DapIo {
    pub fn new(stream: TcpStream, tracer: Option<FileTracer>, trace: bool) -> anyhow::Result<Self> {
        stream.set_nodelay(true)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            tracer,
            trace,
        })
    }

    pub fn read_message(&mut self) -> anyhow::Result<Value> {
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
        if self.trace
            && let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(&msg)
        {
            tracer.line(&format!("<- {line}"));
        }
        Ok(msg)
    }

    pub fn write_message<T: Serialize>(&mut self, v: &T) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(v)?;
        if self.trace
            && let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(v)
        {
            tracer.line(&format!("-> {line}"));
        }
        write!(self.stream, "Content-Length: {}\r\n\r\n", payload.len())?;
        self.stream.write_all(&payload)?;
        self.stream.flush()?;
        Ok(())
    }
}

impl DapTransport for DapIo {
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
        if self.trace
            && let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(&msg)
        {
            tracer.line(&format!("<- {line}"));
        }
        Ok(msg)
    }

    fn write_message(&mut self, message: &Value) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(message)?;
        if self.trace
            && let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(message)
        {
            tracer.line(&format!("-> {line}"));
        }
        write!(self.stream, "Content-Length: {}\r\n\r\n", payload.len())?;
        self.stream.write_all(&payload)?;
        self.stream.flush()?;
        Ok(())
    }
}
