use super::debugger::command::Continue;
use crate::console::hook::TerminalHook;
use crate::console::variable::render_variable;
use crate::console::view::FileView;
use crate::debugger::command::{
    Backtrace, Break, Frame, Quit, StepI, StepInto, StepOut, StepOver, Symbol, Variables,
};
use crate::debugger::{command, Debugger};
use command::{Memory, Register};
use nix::unistd::Pid;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::borrow::Cow;

pub mod hook;
mod variable;
pub mod view;

pub struct AppBuilder {
    file_view: FileView,
}

impl AppBuilder {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            file_view: FileView::new(),
        }
    }

    pub fn build(self, program: impl Into<String>, pid: Pid) -> TerminalApplication {
        let hook = TerminalHook::new(self.file_view);
        let debugger = Debugger::new(program, pid, hook);
        TerminalApplication { debugger }
    }
}

pub struct TerminalApplication {
    debugger: Debugger<TerminalHook>,
}

impl TerminalApplication {
    pub fn run(&self) -> anyhow::Result<()> {
        self.debugger.on_debugee_start()?;

        let mut rl = Editor::<()>::new()?;
        if rl.load_history("history.txt").is_err() {
            println!("No previous history.");
        }

        loop {
            let readline = rl.readline(">> ");
            match readline {
                Ok(input) => {
                    rl.add_history_entry(input.as_str());
                    println!("> {}", input);
                    if let Err(e) = self.handle_cmd(&input, &self.debugger) {
                        println!("Error: {:?}", e);
                    }
                    rl.add_history_entry(input.as_str());
                }
                Err(ReadlineError::Interrupted) => break,
                Err(ReadlineError::Eof) => break,
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }

        Ok(())
    }

    fn handle_cmd(&self, cmd: &str, debugger: &Debugger<TerminalHook>) -> anyhow::Result<()> {
        let args = cmd.split(' ').collect::<Vec<_>>();
        let command = args[0];

        match command.to_lowercase().as_str() {
            "c" | "continue" => Continue::new(debugger).run()?,
            "b" | "break" => Break::new(debugger, args)?.run()?,
            "r" | "register" => {
                let cmd = Register::new(debugger, args)?;
                let response = cmd.run()?;
                response.iter().for_each(|register| {
                    println!("{:10} {:#016X}", register.register_name, register.value);
                });
            }
            "mem" | "memory" => {
                let read = Memory::new(debugger, args)?.run()?;
                println!("read at address: {:#016X}", read);
            }
            "bt" | "trace" => {
                let bt = Backtrace::new(debugger).run()?;

                bt.iter().for_each(|part| match part.place.as_ref() {
                    Some(place) => {
                        println!(
                            "{:#016X} - {} ({:#016X}) + {:#X} {}",
                            part.ip,
                            place.func_name,
                            place.start_ip,
                            place.offset,
                            place.signal_frame,
                        );
                    }
                    None => {
                        println!("{:#016X} - ????", part.ip)
                    }
                })
            }
            "stepi" => StepI::new(debugger).run()?,
            "step" | "stepinto" => StepInto::new(debugger).run()?,
            "next" | "stepover" => StepOver::new(debugger).run()?,
            "finish" | "stepout" => StepOut::new(debugger).run()?,
            "vars" => {
                Variables::new(debugger).run()?.iter().for_each(|var| {
                    println!(
                        "{} = {}",
                        var.name.as_ref().unwrap_or(&Cow::Borrowed("unknown")),
                        render_variable(&var.render(debugger.pid), 0),
                    );
                });
            }
            "frame" => {
                let frame = Frame::new(debugger).run()?;
                println!("current frame: {:#016X}", frame.base_addr);
                println!(
                    "return address: {}",
                    frame
                        .return_addr
                        .map_or(String::from("unknown"), |addr| format!("{:#016x}", addr))
                );
            }
            "symbol" => {
                let cmd = Symbol::new(debugger, args)?;
                let symbol = cmd.run()?;
                println!("{:?} {:#016X}", symbol.kind, symbol.addr);
            }

            "help" => todo!(),
            "q" | "quit" => Quit::new().run(),

            _ => eprintln!("unknown command"),
        }

        Ok(())
    }
}
