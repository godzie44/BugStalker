//! DAP stdio transport tests
//! Tests for DAP in stdio mode (--dap-local)

use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::os::unix::io::AsRawFd;
use libc::{fcntl, F_GETFL, F_SETFL, O_NONBLOCK};
use std::time::{Duration, Instant};
use std::thread;

/// Start bs in stdio DAP mode and return a connection to it
fn start_bs_stdio_dap(debugee: &str) -> anyhow::Result<StdioDAP> {
    let mut child = Command::new(
        std::env::var("CARGO_BIN_EXE_bs")
            .unwrap_or_else(|_| "./target/debug/bs".to_string()),
    )
    .arg("--dap-local")
    .arg(debugee)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()?;

    let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    // Set stdout non-blocking so reads can be polled with timeouts
    let fd = stdout.as_raw_fd();
    unsafe {
        let flags = fcntl(fd, F_GETFL);
        if flags >= 0 {
            let _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK);
        }
    }
    Ok(StdioDAP {
        child,
        stdin: Box::new(stdin),
        reader: BufReader::new(stdout),
    })
}

pub struct StdioDAP {
    child: Child,
    stdin: Box<dyn Write + Send>,
    reader: BufReader<std::process::ChildStdout>,
}

impl StdioDAP {
    fn read_message(&mut self) -> anyhow::Result<serde_json::Value> {
        const MESSAGE_TIMEOUT: Duration = Duration::from_secs(15);
        let deadline = Instant::now() + MESSAGE_TIMEOUT;

        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let read_n = loop {
                match self.reader.read_line(&mut line) {
                    Ok(n) => break n,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        if Instant::now() > deadline {
                            return Err(anyhow::anyhow!("Timed out waiting for DAP header"));
                        }
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(err) => return Err(err.into()),
                }
            };

            if read_n == 0 {
                return Err(anyhow::anyhow!("connection closed"));
            }
            let line_trimmed = line.trim_end_matches(['\r', '\n']);
            if line_trimmed.is_empty() {
                if content_length.is_some() {
                    break;
                } else {
                    continue;
                }
            }

            let lower = line_trimmed.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = Some(v.trim().parse()?);
            }
        }

        let len = content_length.ok_or_else(|| anyhow::anyhow!("missing content-length"))?;
        let mut buf = vec![0u8; len];

        // read body with deadline
        let mut offset = 0usize;
        while offset < buf.len() {
            match self.reader.read(&mut buf[offset..]) {
                Ok(0) => return Err(anyhow::anyhow!("DAP connection closed")),
                Ok(n) => {
                    offset += n;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if Instant::now() > deadline {
                        return Err(anyhow::anyhow!("Timed out waiting for DAP body"));
                    }
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(err) => return Err(err.into()),
            }
        }

        let v = serde_json::from_slice(&buf)?;
        Ok(v)
    }

    fn write_message(&mut self, msg: &serde_json::Value) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(msg)?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", payload.len())?;
        self.stdin.write_all(&payload)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn send_request(
        &mut self,
        seq: i64,
        command: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.write_message(&json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments
        }))
    }
}

#[test]
fn test_stdio_dap_initialize() -> anyhow::Result<()> {
    let hello_world = std::env::var("CARGO_BIN_EXE_hello_world")
        .unwrap_or_else(|_| "./examples/target/debug/hello_world".to_string());

    let mut dap = start_bs_stdio_dap(&hello_world)?;

    // Send initialize request
    dap.send_request(1, "initialize", json!({
        "clientID": "test",
        "clientName": "test-client",
        "adapterID": "bs-dap",
        "pathFormat": "path",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "supportsVariableType": true,
        "supportsVariablePaging": true,
        "supportsRunInTerminalRequest": false,
        "supportsMemoryReferences": true,
    }))?;

    // Receive initialize response
    let response = dap.read_message()?;
    assert_eq!(response["type"], "response");
    assert_eq!(response["command"], "initialize");
    assert!(response["success"].as_bool().unwrap_or(false));

    // Receive initialized event
    let event = dap.read_message()?;
    assert_eq!(event["type"], "event");
    assert_eq!(event["event"], "initialized");

    Ok(())
}

#[test]
fn test_stdio_dap_launch() -> anyhow::Result<()> {
    let hello_world = std::env::var("CARGO_BIN_EXE_hello_world")
        .unwrap_or_else(|_| "./examples/target/debug/hello_world".to_string());

    let mut dap = start_bs_stdio_dap(&hello_world)?;

    // Initialize
    dap.send_request(1, "initialize", json!({
        "clientID": "test",
        "clientName": "test-client",
        "adapterID": "bs-dap",
        "pathFormat": "path",
        "linesStartAt1": true,
        "columnsStartAt1": true,
    }))?;

    let _init_response = dap.read_message()?;
    let _init_event = dap.read_message()?;

    // Send launch request
    dap.send_request(2, "launch", json!({
        "request": "launch",
        "program": hello_world,
        "stopOnEntry": true,
    }))?;

    // Receive launch response (may get output events first)
    let response = loop {
        let msg = dap.read_message()?;
        if msg["type"] == "response" && msg["command"] == "launch" {
            break msg;
        }
        // Skip any events (like output events) and keep reading
    };
    assert!(response["success"].as_bool().unwrap_or(false));

    Ok(())
}

#[test]
fn test_stdio_dap_capabilities() -> anyhow::Result<()> {
    let hello_world = std::env::var("CARGO_BIN_EXE_hello_world")
        .unwrap_or_else(|_| "./examples/target/debug/hello_world".to_string());

    let mut dap = start_bs_stdio_dap(&hello_world)?;

    // Initialize
    dap.send_request(1, "initialize", json!({
        "clientID": "test",
        "clientName": "test-client",
        "adapterID": "bs-dap",
    }))?;

    let init_response = dap.read_message()?;
    let _event = dap.read_message()?;

    // Check capabilities
    let body = &init_response["body"];
    assert!(body["supportsConfigurationDoneRequest"].as_bool().unwrap_or(false));
    assert!(body["supportsSetVariable"].as_bool().unwrap_or(false));
    assert!(body["supportsBreakpointLocationsRequest"].as_bool().unwrap_or(false));

    Ok(())
}
