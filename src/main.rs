use anyhow::bail;
use bugstalker::debugger;
use nix::errno::errno;
use nix::libc::{c_char, execl};
use nix::sys;
use nix::sys::personality::Persona;
use nix::unistd::fork;
use nix::unistd::ForkResult::{Child, Parent};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let debugee = &args[1];

    let pid = unsafe { fork() };

    match pid.expect("Fork Failed: Unable to create child process!") {
        Child => {
            execute_debugee(debugee).expect("execute debugee fail");
        }
        Parent { child } => {
            println!("Child pid {:?}", pid);
            let dbg = debugger::Debugger::new(debugee, child);
            dbg.run().expect("run debugger fail");
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
