//! Debugger application entry point.

use bugstalker::debugger::process::Child;
use bugstalker::debugger::{rust, Debugger, NopHook};
use bugstalker::ui::console::AppBuilder;
use bugstalker::ui::tui;
use clap::error::ErrorKind;
use clap::{arg, CommandFactory, Parser};
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

    /// Arguments are passed to debugee
    #[arg(raw(true))]
    args: Vec<String>,
}

fn main() {
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

    let debugger = Debugger::new(process, NopHook {}).expect("prepare application fail");

    if args.tui {
        let mut app_builder = tui::AppBuilder::new(stdout_reader.into(), stderr_reader.into());
        if process_is_external {
            app_builder = app_builder.app_already_run();
        }
        let app = app_builder.build(debugger);
        app.run().expect("run application fail");
    } else {
        let mut app_builder = AppBuilder::new(stdout_reader.into(), stderr_reader.into());
        if process_is_external {
            app_builder = app_builder.app_already_run();
        }
        let app = app_builder.build(debugger).expect("build application fail");
        app.run().expect("run application fail");
    }
}
