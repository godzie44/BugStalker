use super::debugger::command::Continue;
use crate::console::hook::TerminalHook;
use crate::console::variable::render_variable_ir;
use crate::console::view::FileView;
use crate::debugger::command::{
    Arguments, Backtrace, Break, Frame, Run, StepI, StepInto, StepOut, StepOver, Symbol, Trace,
    Variables,
};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::{command, Debugger};
use command::{Memory, Register};
use nix::unistd::Pid;
use rustyline::Editor;
use std::sync::mpsc;
use std::thread;

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

enum ControlAction {
    Cmd(String),
    Terminate,
}

pub struct TerminalApplication {
    debugger: Debugger,
}

impl TerminalApplication {
    pub fn run(mut self) -> anyhow::Result<()> {
        env_logger::init();

        let (control_tx, control_rx) = mpsc::channel::<ControlAction>();

        {
            let control_tx = control_tx.clone();
            thread::spawn(move || {
                let mut rl = Editor::<()>::new().expect("create editor");
                if rl.load_history("history.txt").is_err() {
                    println!("No previous history.");
                }

                loop {
                    let readline = rl.readline(">> ");
                    match readline {
                        Ok(input) => {
                            if input == "q" || input == "quit" {
                                control_tx.send(ControlAction::Terminate).unwrap();
                            } else {
                                rl.add_history_entry(&input);
                                control_tx.send(ControlAction::Cmd(input)).unwrap();
                            }
                        }
                        Err(err) => {
                            println!("error: {:?}", err);
                            control_tx.send(ControlAction::Terminate).unwrap();
                            break;
                        }
                    }
                }
            });
        }

        {
            ctrlc::set_handler(move || control_tx.send(ControlAction::Terminate).unwrap())?;
        }

        for action in control_rx {
            match action {
                ControlAction::Cmd(command) => {
                    println!("> {}", command);
                    if let Err(e) = self.handle_cmd(&command) {
                        println!("error: {:?}", e);
                    }
                }
                ControlAction::Terminate => {
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
                        "thread {} - {}",
                        thread.thread.pid,
                        thread.pc.unwrap_or(0_usize.into())
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
            "args" => Arguments::new(&self.debugger)?
                .run()?
                .into_iter()
                .for_each(|arg| {
                    println!("{} = {}", arg.name(), render_variable_ir(&arg, 0),);
                }),
            "frame" => {
                let frame = Frame::new(&self.debugger).run()?;
                println!("current frame: {}", frame.base_addr);
                println!(
                    "return address: {}",
                    frame
                        .return_addr
                        .map_or(String::from("unknown"), |addr| format!("{}", addr))
                );
            }
            "symbol" => {
                let cmd = Symbol::new(&self.debugger, args)?;
                let symbol = cmd.run()?;
                println!("{:?} {:#016X}", symbol.kind, symbol.addr);
            }
            "help" => todo!(),
            _ => eprintln!("unknown command"),
        }

        Ok(())
    }
}
