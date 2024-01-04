//! Debugger application entry point.

use bugstalker::debugger::process::Child;
use bugstalker::debugger::{rust, Debugger, NopHook};
use bugstalker::log::LOGGER_SWITCHER;
use bugstalker::oracle::{builtin, Oracle};
use bugstalker::ui::{console, tui};
use clap::error::ErrorKind;
use clap::{arg, CommandFactory, Parser};
use log::info;
use nix::unistd::Pid;
use std::path::PathBuf;
use std::rc::Rc;

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

    let process_is_external = process.is_external();

    let oracles: Vec<Rc<dyn Oracle>> = args
        .oracle
        .into_iter()
        .filter_map(|name| {
            let oracle = builtin::create_builtin(&name)?.into();
            info!(target: "debugger", "oracle `{name}` discovered");
            Some(oracle)
        })
        .collect();

    let debugger = Debugger::new(process, NopHook {}, oracles).expect("prepare application fail");

    if args.tui {
        let app_builder = tui::AppBuilder::new(stdout_reader.into(), stderr_reader.into())
            .with_already_run(process_is_external);
        let app = app_builder.build(debugger);
        app.run().expect("run application fail");
    } else {
        let app_builder = console::AppBuilder::new(stdout_reader.into(), stderr_reader.into())
            .with_already_run(process_is_external);
        let app = app_builder.build(debugger).expect("build application fail");
        app.run().expect("run application fail");
    }
}
