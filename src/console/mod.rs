use super::debugger::command::Continue;
use crate::console::hook::TerminalHook;
use crate::console::variable::render_variable_ir;
use crate::console::view::FileView;
use crate::debugger::command::{
    Arguments, Backtrace, Break, Command, Frame, Run, StepI, StepInto, StepOut, StepOver, Symbol,
    Trace, Variables,
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
                                break;
                            } else {
                                rl.add_history_entry(&input);
                                control_tx.send(ControlAction::Cmd(input)).unwrap();
                            }
                        }
                        Err(err) => {
                            println!("error: {:#}", err);
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
                        println!("error: {:#}", e);
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
        match Command::parse(cmd)? {
            Command::PrintVariables(print_var_command) => Variables::new(&self.debugger)
                .handle(print_var_command)?
                .into_iter()
                .for_each(|var| {
                    println!("{} = {}", var.name(), render_variable_ir(&var, 0));
                }),
            Command::PrintArguments(print_arg_command) => Arguments::new(&self.debugger)
                .handle(print_arg_command)?
                .into_iter()
                .for_each(|arg| {
                    println!("{} = {}", arg.name(), render_variable_ir(&arg, 0));
                }),
            Command::PrintBacktrace => {
                let bt = Backtrace::new(&self.debugger).handle()?;

                for frame in bt.iter() {
                    match &frame.func_name {
                        None => {
                            println!("{} - ????", frame.ip)
                        }
                        Some(fn_name) => {
                            let user_bt_end = fn_name == "main"
                                || fn_name.contains("::main")
                                || fn_name.contains("::thread_start");

                            let fn_ip = frame.fn_start_ip.unwrap_or_default();
                            println!(
                                "{} - {} ({} + {:#X})",
                                frame.ip,
                                fn_name,
                                fn_ip,
                                frame.ip.as_u64() - fn_ip.as_u64(),
                            );

                            if user_bt_end {
                                break;
                            }
                        }
                    }
                }
            }
            Command::Continue => Continue::new(&mut self.debugger).handle()?,
            Command::PrintFrame => {
                let frame = Frame::new(&self.debugger).handle()?;
                println!("cfa: {}", frame.cfa);
                println!(
                    "return address: {}",
                    frame
                        .return_addr
                        .map_or(String::from("unknown"), |addr| format!("{}", addr))
                );
            }
            Command::Run => Run::new(&mut self.debugger).handle()?,
            Command::StepInstruction => StepI::new(&self.debugger).handle()?,
            Command::StepInto => StepInto::new(&self.debugger).handle()?,
            Command::StepOut => StepOut::new(&mut self.debugger).handle()?,
            Command::StepOver => StepOver::new(&mut self.debugger).handle()?,
            Command::PrintTrace => {
                let bt = Trace::new(&self.debugger).handle()?;
                bt.iter().for_each(|thread| {
                    println!(
                        "thread {} - {}",
                        thread.thread.pid,
                        thread
                            .bt
                            .as_ref()
                            .and_then(|bt| bt.get(0).map(|f| f.ip))
                            .unwrap_or(0_usize.into())
                    );
                    if let Some(ref bt) = thread.bt {
                        for frame in bt.iter() {
                            match &frame.func_name {
                                None => {
                                    println!("{} - ????", frame.ip)
                                }
                                Some(fn_name) => {
                                    let user_bt_end = fn_name == "main"
                                        || fn_name.contains("::main")
                                        || fn_name.contains("::thread_start");

                                    let fn_ip = frame.fn_start_ip.unwrap_or_default();
                                    println!(
                                        "{} - {} ({} + {:#X})",
                                        frame.ip,
                                        fn_name,
                                        fn_ip,
                                        frame.ip.as_u64() - fn_ip.as_u64(),
                                    );

                                    if user_bt_end {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });
            }
            Command::Breakpoint(bp_cmd) => Break::new(&mut self.debugger).handle(bp_cmd)?,
            Command::Memory(mem_cmd) => {
                let read = Memory::new(&self.debugger).handle(mem_cmd)?;
                println!("{:#016X}", read);
            }
            Command::Register(reg_cmd) => {
                let response = Register::new(&self.debugger).handle(&reg_cmd)?;
                response.iter().for_each(|register| {
                    println!("{:10} {:#016X}", register.register_name, register.value);
                });
            }
            Command::Help(reason) => match reason {
                None => {
                    println!("help here (TODO)")
                }
                Some(reason) => {
                    println!("{reason}");
                    println!("help here (TODO)")
                }
            },
            Command::PrintSymbol(symbol) => {
                let symbol = Symbol::new(&self.debugger).handle(&symbol)?;
                println!("{:?} {:#016X}", symbol.kind, symbol.addr);
            }
        }

        Ok(())
    }
}
