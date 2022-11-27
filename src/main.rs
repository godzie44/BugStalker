use anyhow::bail;
use bugstalker::console::AppBuilder;
use bugstalker::debugger;
use clap::Parser;
use nix::errno::errno;
use nix::libc::{c_char, execl};
use nix::sys;
use nix::sys::personality::Persona;
use nix::unistd::fork;
use nix::unistd::ForkResult::{Child, Parent};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value_t = String::from("console"))]
    ui: String,

    debugee: String,
}

fn main() {
    let args = Args::parse();
    let debugee = &args.debugee;

    let pid = unsafe { fork() };

    match pid.expect("Fork Failed: Unable to create child process!") {
        Child => {
            execute_debugee(debugee).expect("execute debugee fail");
        }
        Parent { child } => {
            println!("Child pid {:?}", pid);

            match args.ui.as_str() {
                "cui" => {}
                _ => {
                    let app = AppBuilder::new().build(debugee, child);
                    app.run().expect("run application fail");
                }
            }
        }
    }
}

fn execute_debugee(path: &str) -> anyhow::Result<()> {
    sys::personality::set(Persona::ADDR_NO_RANDOMIZE)?;

    sys::ptrace::traceme()?;

    unsafe {
        let path = path.as_ptr() as *const c_char;
        if execl(path, std::ptr::null()) < 0 {
            bail!("cannot execute process: {}", errno());
        }
    }

    Ok(())
}
