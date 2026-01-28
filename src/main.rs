//! Debugger application entry point.

use bugstalker::debugger::rust;
use bugstalker::dap::yadap;
use bugstalker::dap::transport::DapTransport;
use bugstalker::log::LOGGER_SWITCHER;
use bugstalker::ui;
use bugstalker::ui::config::{Theme, UIConfig};
use bugstalker::ui::supervisor::{DebugeeSource, Interface};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use std::fmt::Display;
use std::path::PathBuf;
use std::process::exit;
use std::str::FromStr;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Start with terminal ui
    #[clap(long)]
    #[arg(default_value_t = false)]
    tui: bool,

    /// Enable Debug Adapter Protocol in stdio mode (embedded in terminal/IDE)
    #[clap(long)]
    #[arg(default_value_t = false)]
    dap_local: bool,

    /// Enable Debug Adapter Protocol in TCP server mode
    #[clap(long)]
    dap_remote: Option<String>,

    /// DAP: exit after first debug session
    #[clap(long)]
    dap_oneshot: bool,

    /// DAP: log file for adapter diagnostics
    #[clap(long)]
    dap_log_file: Option<PathBuf>,

    /// DAP: trace protocol traffic (requires --dap-log-file)
    #[clap(long)]
    dap_trace: bool,

    /// Attach to running process PID
    #[clap(long, short)]
    pid: Option<i32>,

    #[clap(long)]
    cwd: Option<PathBuf>,

    /// Executable file (debugee)
    debugee: Option<String>,

    /// Path to rust stdlib
    #[clap(short, long)]
    std_lib_path: Option<String>,

    /// Discover a specific oracle (maybe more than one)
    #[clap(short, long)]
    oracle: Vec<String>,

    /// Arguments are passed to debugee
    #[arg(raw(true))]
    args: Vec<String>,

    /// Theme used for visualize code and variables.
    /// Available themes: none, inspired_github, solarized_dark, solarized_light, base16_eighties_dark
    /// base16_mocha_dark, base16_ocean_dark, base16_ocean_light
    #[clap(short, long)]
    #[arg(default_value = "solarized_dark")]
    theme: String,

    /// Path to TUI keymap file [default: ~/.config/bs/keymap.toml]
    #[clap(long, env)]
    keymap_file: Option<String>,

    // Retain command history between sessions.
    #[clap(long, env)]
    #[arg(default_value_t = false)]
    save_history: bool,
}

fn print_fatal_and_exit(kind: ErrorKind, message: impl Display) -> ! {
    let mut cmd = Args::command();
    _ = cmd.error(kind, message).print();
    exit(1);
}

trait FatalResult<T> {
    fn unwrap_or_exit(self, kind: ErrorKind, message: impl Display) -> T;
}

impl<T, E: Display> FatalResult<T> for Result<T, E> {
    fn unwrap_or_exit(self, kind: ErrorKind, message: impl Display) -> T {
        match self {
            Ok(ok) => ok,
            Err(err) => print_fatal_and_exit(kind, format!("{message}: {err:#}")),
        }
    }
}

impl From<&Args> for UIConfig {
    fn from(args: &Args) -> Self {
        Self {
            theme: Theme::from_str(&args.theme)
                .unwrap_or_exit(ErrorKind::InvalidValue, "Not an available theme"),
            tui_keymap: ui::tui::config::KeyMap::from_file(args.keymap_file.as_deref())
                .unwrap_or_default(),
            save_history: args.save_history,
        }
    }
}

fn main() {
    let logger = env_logger::Logger::from_default_env();
    let filter = logger.filter();
    LOGGER_SWITCHER.switch(logger, filter);

    let args = Args::parse();
    ui::config::set(UIConfig::from(&args));

    rust::Environment::init(args.std_lib_path.as_ref().map(|p| PathBuf::from(p)));

    let debugee_src = || {
        if let Some(ref debugee) = args.debugee {
            DebugeeSource::File {
                path: debugee,
                args: &args.args,
                cwd: args.cwd.as_deref(),
            }
        } else if let Some(pid) = args.pid {
            DebugeeSource::Process { pid }
        } else {
            print_fatal_and_exit(
                ErrorKind::ArgumentConflict,
                "Please provide a debugee name or use a \"-p\" option for attach to already running process",
            );
        }
    };

    // Determine interface mode
    let interface = if args.dap_local {
        // Stdio DAP mode
        Interface::DAP
    } else if let Some(listen_addr) = &args.dap_remote {
        // TCP DAP server mode
        run_dap_tcp_server(&args, listen_addr)
            .unwrap_or_exit(ErrorKind::Io, "DAP TCP server error");
        return;
    } else if args.tui {
        Interface::TUI {
            source: debugee_src(),
        }
    } else {
        Interface::Default {
            source: debugee_src(),
        }
    };

    ui::supervisor::Supervisor::run(interface, &args.oracle)
        .unwrap_or_exit(ErrorKind::InvalidSubcommand, "Application error")
}

fn run_dap_tcp_server(args: &Args, listen_addr: &str) -> anyhow::Result<()> {
    use log::warn;
    use std::net::{SocketAddr, TcpListener};
    
    let addr: SocketAddr = listen_addr.parse()
        .map_err(|_| anyhow::anyhow!("Invalid listen address: {}", listen_addr))?;
    let listener = TcpListener::bind(addr)
        .map_err(|e| anyhow::anyhow!("Failed to bind {}: {}", addr, e))?;
    
    log::info!(target: "dap", "DAP TCP server listening on {addr}");

    let tracer = match &args.dap_log_file {
        Some(path) => Some(yadap::tracer::FileTracer::new(path)?),
        None => None,
    };
    if args.dap_trace && tracer.is_none() {
        warn!(target: "dap", "--dap-trace requires --dap-log-file; tracing disabled");
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
        log::info!(target: "dap", "DAP client connected: {peer}");
        if let Some(t) = &tracer {
            t.line(&format!("client connected: {peer}"));
        }

        let io = match yadap::io::DapIo::new(stream, tracer.clone(), args.dap_trace) {
            Ok(v) => v,
            Err(err) => {
                warn!(target: "dap", "failed to init DAP I/O: {err:#}");
                continue;
            }
        };

        let transport: Box<dyn DapTransport> = Box::new(io);
        let res = yadap::session::DebugSession::new(transport).run(args.oracle.clone());
        if let Err(err) = res {
            warn!(target: "dap", "session ended with error: {err:#}");
            if let Some(t) = &tracer {
                t.line(&format!("session error: {err:#}"));
            }
        } else if let Some(t) = &tracer {
            t.line("session finished OK");
        }

        if args.dap_oneshot {
            break;
        }
    }
    Ok(())
}

