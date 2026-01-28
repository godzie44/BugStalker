use std::path::Path;

use crate::debugger::DebuggerBuilder;
use crate::debugger::process::{Child, Installed};
use crate::dap::transport::StdioTransport;
use crate::dap::yadap;
use crate::oracle::builtin;
use crate::ui::console::TerminalApplication;
use crate::ui::tui::TuiApplication;
use crate::ui::{console, tui};
use anyhow::Context;
use log::{info, warn};
use nix::unistd::Pid;

/// Interface type.
pub enum Interface<'a> {
    TUI { source: DebugeeSource<'a> },
    Default { source: DebugeeSource<'a> },
    DAP,
}

/// Source from which debugee is created or attached.
pub enum DebugeeSource<'a> {
    /// Create debugee from executable file with arguments.
    File {
        path: &'a str,
        args: &'a [String],
        cwd: Option<&'a Path>,
    },
    /// Create debugee from an already running process by its pid.
    Process { pid: i32 },
}

impl DebugeeSource<'_> {
    pub fn create_child(
        self,
        stdout_writer: os_pipe::PipeWriter,
        stderr_writer: os_pipe::PipeWriter,
    ) -> anyhow::Result<Child<Installed>> {
        match self {
            DebugeeSource::File { path, args, cwd } => {
                let path = if !Path::new(path).exists() {
                    which::which(path)?.to_string_lossy().to_string()
                } else {
                    path.to_string()
                };
                let proc_tpl = Child::new(path, args, cwd, stdout_writer, stderr_writer);
                proc_tpl.install().context("Initial process instantiation")
            }
            DebugeeSource::Process { pid } => {
                Child::from_external(Pid::from_raw(pid), stdout_writer, stderr_writer)
                    .context("Attach external process")
            }
        }
    }
}

/// Possible applications.
#[allow(clippy::large_enum_variant)]
pub enum Application {
    TUI(TuiApplication),
    Terminal(TerminalApplication),
    DAP,
}

impl Application {
    pub fn run(self) -> anyhow::Result<ControlFlow> {
        match self {
            Application::TUI(tui_app) => tui_app.run(),
            Application::Terminal(term_app) => term_app.run(),
            Application::DAP => {
                let transport = StdioTransport::new()?;
                let session = yadap::session::DebugSession::new(Box::new(transport));
                session.run(vec![])?;
                Ok(ControlFlow::Exit)
            }
        }
    }
}

/// Result of application execution. Application may request exit at the end of execution, or may
/// request a switch to another application.
#[allow(clippy::large_enum_variant)]
pub enum ControlFlow {
    Exit,
    Switch(Application),
}

/// Supervisor control application execution process.
/// Makes it possible to switch between applications in runtime
pub struct Supervisor;

impl Supervisor {
    /// Create and run initial application, make application switch if needed.
    ///
    /// # Arguments
    ///
    /// * `src`: debugee source
    /// * `ui`: determines what application will be created
    /// * `oracles`: list of oracle names
    pub fn run(ui: Interface, oracles: &[String]) -> anyhow::Result<()> {
        let (stdout_reader, stdout_writer) = os_pipe::pipe()?;
        let (stderr_reader, stderr_writer) = os_pipe::pipe()?;

        let process = |src: DebugeeSource| src.create_child(stdout_writer, stderr_writer);

        let oracles = oracles
            .iter()
            .filter_map(|ora_name| {
                if let Some(oracle) = builtin::make_builtin(ora_name) {
                    info!(target: "debugger", "oracle `{ora_name}` discovered");
                    Some(oracle)
                } else {
                    warn!(target: "debugger", "oracle `{ora_name}` not found");
                    None
                }
            })
            .collect();

        let mut app = match ui {
            Interface::TUI { source } => {
                let app_builder = tui::AppBuilder::new(stdout_reader.into(), stderr_reader.into());
                let app = app_builder
                    .build(
                        DebuggerBuilder::new().with_oracles(oracles),
                        process(source)?,
                    )
                    .context("Build debugger")?;
                Application::TUI(app)
            }
            Interface::Default { source } => {
                let app_builder =
                    console::AppBuilder::new(stdout_reader.into(), stderr_reader.into());
                let app = app_builder
                    .build(
                        DebuggerBuilder::new().with_oracles(oracles),
                        process(source)?,
                    )
                    .context("Build debugger")?;
                Application::Terminal(app)
            }
            Interface::DAP => Application::DAP,
        };

        loop {
            let ctl = app.run()?;

            match ctl {
                ControlFlow::Exit => {
                    return Ok(());
                }
                ControlFlow::Switch(next_app) => {
                    app = next_app;
                }
            }
        }
    }
}
