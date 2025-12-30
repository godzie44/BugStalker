//! YADAP - BugStalker Debug Adapter Protocol (DAP) adapter.
//!
//! This binary exposes a minimal Debug Adapter Protocol server over TCP.
//! Intended as a building block for IDE integrations (VSCode, etc.).

mod yadap;

use anyhow::Context;
use clap::Parser;
use log::{info, warn};
use std::net::{SocketAddr, TcpListener};

use yadap::args::Args;
use yadap::io::DapIo;
use yadap::session::DebugSession;
use yadap::tracer::FileTracer;

fn main() -> anyhow::Result<()> {
    let logger = env_logger::Logger::from_default_env();
    let filter = logger.filter();
    bugstalker::log::LOGGER_SWITCHER.switch(logger, filter);

    let args = Args::parse();

    // Ensure Rust environment is initialised for non-CLI frontend.
    // This avoids panics in src/debugger/rust/mod.rs when core tries to access it.
    bugstalker::debugger::rust::Environment::init(None);

    let addr: SocketAddr = args.listen.parse().context("Invalid listen address")?;
    let listener = TcpListener::bind(addr).with_context(|| format!("bind {addr}"))?;
    info!(target: "dap", "yadap listening on {addr}");

    let tracer = match &args.log_file {
        Some(path) => Some(FileTracer::new(path)?),
        None => None,
    };
    if args.trace_dap && tracer.is_none() {
        warn!(target: "dap", "--trace-dap requires --log-file; tracing disabled");
    }

    // Server mode: accept multiple clients sequentially. One client == one debug session.
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(v) => v,
            Err(err) => {
                warn!(target: "dap", "accept failed: {err:#}");
                continue;
            }
        };
        info!(target: "dap", "DAP client connected: {peer}");
        if let Some(t) = &tracer {
            t.line(&format!("client connected: {peer}"));
        }

        let io = match DapIo::new(stream, tracer.clone(), args.trace_dap) {
            Ok(v) => v,
            Err(err) => {
                warn!(target: "dap", "failed to init DAP I/O: {err:#}");
                continue;
            }
        };

        let res = DebugSession::new(io).run(args.oracle.clone());
        if let Err(err) = res {
            warn!(target: "dap", "session ended with error: {err:#}");
            if let Some(t) = &tracer {
                t.line(&format!("session error: {err:#}"));
            }
        } else if let Some(t) = &tracer {
            t.line("session finished OK");
        }

        if args.oneshot {
            break;
        }
    }
    Ok(())
}
