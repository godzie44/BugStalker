use super::command::Continue;
use crate::debugger::command::{
    Backtrace, Break, Frame, Quit, StepI, StepInto, StepOut, StepOver, Symbol, Variables,
};
use crate::debugger::console::hook::TerminalHook;
use crate::debugger::console::variable::render_variable_value;
use crate::debugger::console::view::FileView;
use crate::debugger::{command, Debugger};
use command::{Memory, Register};
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::borrow::Cow;
use std::rc::Rc;

pub mod hook;
mod variable;
pub mod view;

pub struct TerminalApplication {
    file_view: Rc<FileView>,
}

impl TerminalApplication {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            file_view: Rc::new(FileView::new()),
        }
    }

    pub fn make_hook(&self) -> TerminalHook {
        TerminalHook::new(self.file_view.clone())
    }

    pub fn run(&self, debugger: Debugger<TerminalHook>) -> anyhow::Result<()> {
        debugger.on_debugee_start()?;

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
                    if let Err(e) = self.handle_cmd(&input, &debugger) {
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
            "stepi" => {
                let cmd = StepI::new(debugger);
                let mb_place = cmd.run()?;
                if let Some(place) = mb_place {
                    println!("{}:{}", place.file, place.line_number);
                    println!("{}", self.file_view.render_source(&place, 1)?);
                }
            }
            "step" | "stepinto" => {
                let cmd = StepInto::new(debugger);
                let mb_place = cmd.run()?;
                if let Some(place) = mb_place {
                    println!("{}:{}", place.file, place.line_number);
                    println!("{}", self.file_view.render_source(&place, 1)?);
                }
            }
            "next" | "stepover" => StepOver::new(debugger).run()?,
            "finish" | "stepout" => StepOut::new(debugger).run()?,
            "vars" => {
                Variables::new(debugger).run()?.iter().for_each(|var| {
                    println!(
                        "{} = {}",
                        var.name.as_ref().unwrap_or(&Cow::Borrowed("unknown")),
                        render_variable_value(var, debugger.pid)
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
