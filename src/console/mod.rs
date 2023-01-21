use super::debugger::command::Continue;
use crate::console::hook::TerminalHook;
use crate::console::variable::render_variable_ir;
use crate::console::view::FileView;
use crate::debugger::command::{
    Backtrace, Break, Frame, Quit, Run, StepI, StepInto, StepOut, StepOver, Symbol, Trace,
    Variables,
};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::{command, Debugger, RelocatedAddress};
use command::{Memory, Register};
use nix::unistd::Pid;
use rustyline::error::ReadlineError;
use rustyline::Editor;

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

    pub fn build(
        self,
        program: impl Into<String>,
        pid: Pid,
    ) -> anyhow::Result<TerminalApplication> {
        let hook = TerminalHook::new(self.file_view);
        let debugger = Debugger::new(program, pid, hook)?;
        Ok(TerminalApplication { debugger })
    }
}

pub struct TerminalApplication {
    debugger: Debugger,
}

impl TerminalApplication {
    pub fn run(mut self) -> anyhow::Result<()> {
        env_logger::init();

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
                    if let Err(e) = self.handle_cmd(&input) {
                        println!("Error: {:?}", e);
                        break;
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

    fn handle_cmd(&mut self, cmd: &str) -> anyhow::Result<()> {
        let args = cmd.split(' ').collect::<Vec<_>>();
        let command = args[0];

        match command.to_lowercase().as_str() {
            "r" | "run" => Run::new(&mut self.debugger).run()?,
            "c" | "continue" => Continue::new(&mut self.debugger).run()?,
            "b" | "break" => Break::new(&mut self.debugger, args)?.run()?,
            "reg" | "register" => {
                let cmd = Register::new(&self.debugger, args)?;
                let response = cmd.run()?;
                response.iter().for_each(|register| {
                    println!("{:10} {:#016X}", register.register_name, register.value);
                });
            }
            "mem" | "memory" => {
                let read = Memory::new(&self.debugger, args)?.run()?;
                println!("read at address: {:#016X}", read);
            }
            "bt" | "backtrace" => {
                let bt = Backtrace::new(&self.debugger).run()?;
                bt.iter().for_each(|part| match part.place.as_ref() {
                    Some(place) => {
                        println!(
                            "{:#016X} - {} ({:#016X}) + {:#X}",
                            part.ip, place.func_name, place.start_ip, place.offset,
                        );
                    }
                    None => {
                        println!("{:#016X} - ????", part.ip)
                    }
                })
            }
            "trace" => {
                let bt = Trace::new(&self.debugger).run()?;
                bt.iter().for_each(|thread| {
                    println!(
                        "thread {} - {:#016X}",
                        thread.thread.pid,
                        thread.pc.unwrap_or(RelocatedAddress(0)).0
                    );
                    if let Some(ref bt) = thread.bt {
                        bt.iter().for_each(|part| match part.place.as_ref() {
                            Some(place) => {
                                println!(
                                    "{:#016X} - {} ({:#016X}) + {:#X}",
                                    part.ip, place.func_name, place.start_ip, place.offset,
                                );
                            }
                            None => {
                                println!("{:#016X} - ????", part.ip)
                            }
                        })
                    }
                });
            }
            "stepi" => StepI::new(&self.debugger).run()?,
            "step" | "stepinto" => StepInto::new(&self.debugger).run()?,
            "next" | "stepover" => StepOver::new(&mut self.debugger).run()?,
            "finish" | "stepout" => StepOut::new(&mut self.debugger).run()?,
            "vars" => Variables::new(&self.debugger, args)?
                .run()?
                .into_iter()
                .for_each(|var| {
                    println!("{} = {}", var.name(), render_variable_ir(&var, 0),);
                }),
            "frame" => {
                let frame = Frame::new(&self.debugger).run()?;
                println!("current frame: {:#016X}", frame.base_addr);
                println!(
                    "return address: {}",
                    frame
                        .return_addr
                        .map_or(String::from("unknown"), |addr| format!("{:#016x}", addr.0))
                );
            }
            "symbol" => {
                let cmd = Symbol::new(&self.debugger, args)?;
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
