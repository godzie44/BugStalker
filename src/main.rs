use bugstalker::console::AppBuilder;
use bugstalker::cui;
use bugstalker::debugger::rust;
use clap::{arg, Parser};
use nix::sys;
use nix::sys::personality::Persona;
use nix::sys::ptrace::Options;
use nix::sys::signal::SIGSTOP;
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{fork, ForkResult, Pid};
use std::os::unix::prelude::CommandExt;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Debugger interface type
    #[arg(long, default_value_t = String::from("console"))]
    ui: String,

    debugee: String,

    /// Path to rust stdlib
    #[clap(short, long)]
    std_lib_path: Option<String>,
}

fn main() {
    let args = Args::parse();
    let debugee = &args.debugee;

    rust::Environment::init(args.std_lib_path.map(PathBuf::from));

    let (stdout_reader, stdout_writer) = os_pipe::pipe().unwrap();
    let (stderr_reader, stderr_writer) = os_pipe::pipe().unwrap();

    let mut debugee_cmd = std::process::Command::new(debugee);
    if args.ui.as_str() == "cui" {
        debugee_cmd.stdout(stdout_writer);
        debugee_cmd.stderr(stderr_writer);
    }

    unsafe {
        debugee_cmd.pre_exec(move || {
            sys::personality::set(Persona::ADDR_NO_RANDOMIZE)?;
            Ok(())
        });
    }

    match unsafe { fork().expect("fork() error") } {
        ForkResult::Parent { child: pid } => {
            waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WSTOPPED)).unwrap();
            sys::ptrace::seize(
                pid,
                Options::PTRACE_O_TRACECLONE
                    .union(Options::PTRACE_O_TRACEEXEC)
                    .union(Options::PTRACE_O_TRACEEXIT),
            )
            .unwrap();

            println!("Child pid {:?}", pid);

            match args.ui.as_str() {
                "cui" => {
                    let app = cui::AppBuilder::new(stdout_reader, stderr_reader)
                        .build(debugee, pid)
                        .expect("prepare application fail");

                    app.run().expect("run application fail");
                }
                _ => {
                    let app = AppBuilder::new()
                        .build(debugee, pid)
                        .expect("prepare application fail");
                    app.run().expect("run application fail");
                }
            }
        }
        ForkResult::Child => {
            sys::signal::raise(SIGSTOP).unwrap();
            debugee_cmd.exec();
        }
    }
}
