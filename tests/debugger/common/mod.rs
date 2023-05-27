use bugstalker::debugger::address::RelocatedAddress;
use bugstalker::debugger::{EventHook, Place};
use nix::sys::signal::Signal;
use std::cell::Cell;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct DebugeeRunInfo {
    pub line: Arc<Cell<Option<u64>>>,
    pub file: Arc<Cell<Option<String>>>,
}

#[derive(Default)]
pub struct TestHooks {
    info: DebugeeRunInfo,
}

impl TestHooks {
    pub fn new(info: DebugeeRunInfo) -> Self {
        Self { info }
    }
}

impl EventHook for TestHooks {
    fn on_breakpoint(&self, _pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()> {
        self.info
            .file
            .set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }
    fn on_step(&self, _pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()> {
        self.info
            .file
            .set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }
    fn on_signal(&self, _: Signal) {}
    fn on_exit(&self, _code: i32) {}
}

#[macro_export]
macro_rules! debugger_env {
    ($prog:expr, $args: expr, $child:ident, $code: expr) => {
        use bugstalker::debugger::{rust, Debugger};
        use nix::sys;
        use nix::sys::personality::Persona;
        use nix::sys::ptrace::Options;
        use nix::sys::signal::SIGSTOP;
        use nix::sys::wait::{waitpid, WaitPidFlag};
        use nix::unistd::{fork, ForkResult, Pid};
        use std::fs::File;
        use std::os::unix::process::CommandExt;

        let debugee = $prog;
        rust::Environment::init(None);
        let null_f = File::open("/dev/null").unwrap();
        let mut debugee_cmd = std::process::Command::new(debugee);
        let args: Vec<&str> = Vec::from($args);
        debugee_cmd.args(args);
        debugee_cmd.stdout(null_f);

        unsafe {
            debugee_cmd.pre_exec(move || {
                sys::personality::set(Persona::ADDR_NO_RANDOMIZE)?;
                Ok(())
            });
        }

        match unsafe { fork().unwrap() } {
            ForkResult::Child => {
                sys::signal::raise(SIGSTOP).unwrap();
                debugee_cmd.exec();
            }
            ForkResult::Parent { $child } => {
                waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WSTOPPED)).unwrap();
                sys::ptrace::seize(
                    $child,
                    Options::PTRACE_O_TRACECLONE
                        .union(Options::PTRACE_O_TRACEEXEC)
                        .union(Options::PTRACE_O_TRACEEXIT),
                )
                .unwrap();

                $code
            }
        }
    };
}

#[macro_export]
macro_rules! assert_no_proc {
    ($pid:expr) => {
        use sysinfo::{PidExt, SystemExt};

        let sys = sysinfo::System::new_all();
        assert!(sys
            .process(sysinfo::Pid::from_u32($pid.as_raw() as u32))
            .is_none())
    };
}
