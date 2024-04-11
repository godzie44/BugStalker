//! Debugger application entry point.

use bugstalker::debugger::rust;
use bugstalker::log::LOGGER_SWITCHER;
use bugstalker::ui;
use bugstalker::ui::config::{Theme, UIConfig};
use bugstalker::ui::supervisor::{DebugeeSource, Interface};
use clap::error::ErrorKind;
use clap::{arg, CommandFactory, Parser};
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

    /// Attach to running process PID
    #[clap(long, short)]
    pid: Option<i32>,

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
            Err(err) => print_fatal_and_exit(kind, format!("{message}: {err}")),
        }
    }
}

impl From<&Args> for UIConfig {
    fn from(args: &Args) -> Self {
        Self {
            theme: Theme::from_str(&args.theme)
                .unwrap_or_exit(ErrorKind::InvalidValue, "Not an available theme"),
        }
    }
}

fn main() {
    let logger = env_logger::Logger::from_default_env();
    let filter = logger.filter();
    LOGGER_SWITCHER.switch(logger, filter);

    let args = Args::parse();
    ui::config::set(UIConfig::from(&args));

    rust::Environment::init(args.std_lib_path.map(PathBuf::from));

    let debugee_src = if let Some(ref debugee) = args.debugee {
        DebugeeSource::File {
            path: debugee,
            args: &args.args,
        }
    } else if let Some(pid) = args.pid {
        DebugeeSource::Process { pid }
    } else {
        print_fatal_and_exit(ErrorKind::ArgumentConflict, "Please provide a debugee name or use a \"-p\" option for attach to already running process");
    };

    let interface = if args.tui {
        Interface::TUI
    } else {
        Interface::Default
    };

    ui::supervisor::Supervisor::run(debugee_src, interface, &args.oracle)
        .unwrap_or_exit(ErrorKind::InvalidSubcommand, "Application error")
}
