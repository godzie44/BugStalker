use crate::debugger::error::Error;
use crate::debugger::error::Error::{Ptrace, Waitpid};
use nix::sys;
use nix::sys::personality::Persona;
use nix::sys::ptrace::Options;
use nix::sys::signal::SIGSTOP;
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{fork, ForkResult, Pid};
use os_pipe::PipeWriter;
use std::marker::PhantomData;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Process state.
pub trait State {}

/// Process running and attached with `ptrace` system call.
pub struct Installed;

impl State for Installed {}

/// Process prepare for instantiation by a `fork` call.
pub struct Template;

impl State for Template {}

/// Process attached to tracer with ptrace.
pub struct Child<S: State> {
    pub program: String,
    stdout: PipeWriter,
    stderr: PipeWriter,
    args: Vec<String>,
    pid: Option<Pid>,
    _p: PhantomData<S>,
}

impl Child<Template> {
    /// Create new process, but dont start it.
    ///
    /// # Arguments
    ///
    /// * `program`: program name
    /// * `stdout`: stdout pipe
    /// * `stderr`: stderr pipe
    /// * `args`: program arguments
    pub fn new<ARGS: IntoIterator<Item = I>, I: Into<String>>(
        program: impl Into<String>,
        args: ARGS,
        stdout: PipeWriter,
        stderr: PipeWriter,
    ) -> Child<Template> {
        Self {
            stdout,
            stderr,
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            pid: None,
            _p: PhantomData::default(),
        }
    }
}

impl Child<Installed> {
    /// Return running process pid.
    pub fn pid(&self) -> Pid {
        self.pid.unwrap()
    }
}

impl<S: State> Child<S> {
    /// Instantiate process by `fork()` system call with caller as a parent process.
    /// After installation child process stopped by `SIGSTOP` signal.
    pub fn install(&self) -> Result<Child<Installed>, Error> {
        let mut debugee_cmd = Command::new(&self.program);
        let debugee_cmd = debugee_cmd
            .args(&self.args)
            .stdout(self.stdout.try_clone()?)
            .stderr(self.stderr.try_clone()?);

        unsafe {
            debugee_cmd.pre_exec(move || {
                sys::personality::set(Persona::ADDR_NO_RANDOMIZE)?;
                Ok(())
            });
        }

        match unsafe { fork().expect("fork() error") } {
            ForkResult::Parent { child: pid } => {
                waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WSTOPPED)).map_err(Waitpid)?;
                sys::ptrace::seize(
                    pid,
                    Options::PTRACE_O_TRACECLONE
                        .union(Options::PTRACE_O_TRACEEXEC)
                        .union(Options::PTRACE_O_TRACEEXIT),
                )
                .map_err(Ptrace)?;

                Ok(Child {
                    stdout: self.stdout.try_clone()?,
                    stderr: self.stderr.try_clone()?,
                    program: self.program.clone(),
                    args: self.args.clone(),
                    pid: Some(pid),
                    _p: PhantomData::default(),
                })
            }
            ForkResult::Child => {
                sys::signal::raise(SIGSTOP).unwrap();
                let err = debugee_cmd.exec();
                panic!("run debugee fail with: {err}");
            }
        }
    }
}
