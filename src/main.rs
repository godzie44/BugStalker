//! Debugger application entry point.

use bugstalker::debugger::process::Child;
use bugstalker::debugger::{rust, Debugger, DoNothingHook};
use bugstalker::ui::console::AppBuilder;
use bugstalker::ui::tui;
use clap::{arg, Parser};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Debugger interface type
    #[arg(long, default_value_t = String::from("console"))]
    ui: String,

    /// Executable file (debugee)
    debugee: String,

    /// Path to rust stdlib
    #[clap(short, long)]
    std_lib_path: Option<String>,

    /// Arguments are passed to debugee
    #[arg(raw(true))]
    args: Vec<String>,
}

fn main() {
    let args = Args::parse();
    let debugee = &args.debugee;

    rust::Environment::init(args.std_lib_path.map(PathBuf::from));

    let (stdout_reader, stdout_writer) = os_pipe::pipe().unwrap();
    let (stderr_reader, stderr_writer) = os_pipe::pipe().unwrap();

    let proc_tpl = Child::new(debugee, args.args, stdout_writer, stderr_writer);
    let process = proc_tpl
        .install()
        .expect("initial process instantiation fail");

    match args.ui.as_str() {
        "tui" => {
            let debugger =
                Debugger::new(process, DoNothingHook {}).expect("prepare application fail");
            let app =
                tui::AppBuilder::new(stdout_reader.into(), stderr_reader.into()).build(debugger);
            app.run().expect("run application fail");
        }
        _ => {
            let app = AppBuilder::new(stdout_reader.into(), stderr_reader.into())
                .build_from_process(process)
                .expect("build application fail");
            app.run().expect("run application fail");
        }
    }
}
