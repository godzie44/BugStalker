//! Debugger application entry point.

use bugstalker::debugger::process::Child;
use bugstalker::debugger::{rust, DebuggerBuilder, NopHook};
use bugstalker::log::LOGGER_SWITCHER;
use bugstalker::oracle::builtin;
use bugstalker::ui::{console, tui};
use clap::error::ErrorKind;
use clap::{arg, CommandFactory, Parser};
use log::info;
use nix::unistd::Pid;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
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

    /// Discover a specific oracle (may be more than one)
    #[clap(short, long)]
    oracle: Vec<String>,

    /// Arguments are passed to debugee
    #[arg(raw(true))]
    args: Vec<String>,
}

fn main() {
    let logger = env_logger::Logger::from_default_env();
    let filter = logger.filter();
    LOGGER_SWITCHER.switch(logger, filter);

    let args = Args::parse();

    rust::Environment::init(args.std_lib_path.map(PathBuf::from));
    let (stdout_reader, stdout_writer) = os_pipe::pipe().unwrap();
    let (stderr_reader, stderr_writer) = os_pipe::pipe().unwrap();

    let process = if let Some(ref debugee) = args.debugee {
        let proc_tpl = Child::new(debugee, args.args, stdout_writer, stderr_writer);
        proc_tpl
            .install()
            .expect("initial process instantiation error")
    } else if let Some(pid) = args.pid {
        Child::from_external(Pid::from_raw(pid), stdout_writer, stderr_writer)
            .expect("attach external process error")
    } else {
        let mut cmd = Args::command();
        _ = cmd
            .error(ErrorKind::ArgumentConflict, "Please provide a debugee name or use a \"-p\" option for attach to already running process")
            .print();
        return;
    };

    let mut debugger_builder: DebuggerBuilder<NopHook> = DebuggerBuilder::new();

    for name in args.oracle {
        if let Some(oracle) = builtin::make_builtin(&name) {
            info!(target: "debugger", "oracle `{name}` discovered");
            debugger_builder = debugger_builder.with_oracle(oracle);
        }
    }

    let debugger = debugger_builder
        .build(process)
        .expect("prepare application error");

    if args.tui {
        let app_builder = tui::AppBuilder::new(stdout_reader.into(), stderr_reader.into());
        let app = app_builder.build(debugger);
        app.run().expect("application run error");
    } else {
        let app_builder = console::AppBuilder::new(stdout_reader.into(), stderr_reader.into());
        let app = app_builder
            .build(debugger)
            .expect("application build error");
        app.run().expect("application run error");
    }
}
