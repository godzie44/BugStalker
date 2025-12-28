use super::super::command::r#async::AsyncCommandResult;
use super::super::command::trigger::TriggerEvent;
use super::trigger::TriggerRegistry;
use crate::debugger;
use crate::debugger::Debugger;
use crate::ui::command;
use crate::ui::command::backtrace::Handler as BacktraceHandler;
use crate::ui::command::r#break::ExecutionResult;
use crate::ui::command::r#break::Handler as BreakpointHandler;
use crate::ui::command::r#continue::Handler as ContinueHandler;
use crate::ui::command::frame::ExecutionResult as FrameResult;
use crate::ui::command::frame::Handler as FrameHandler;
use crate::ui::command::memory::Handler as MemoryHandler;
use crate::ui::command::print::Handler as PrintHandler;
use crate::ui::command::register::Handler as RegisterHandler;
use crate::ui::command::run::Handler as RunHandler;
use crate::ui::command::sharedlib::Handler as SharedlibHandler;
use crate::ui::command::source_code::{DisAsmHandler, FunctionLineRangeHandler};
use crate::ui::command::symbol::Handler as SymbolHandler;
use crate::ui::command::thread::ExecutionResult as ThreadResult;
use crate::ui::command::watch::ExecutionResult as WatchpointExecutionResult;
use crate::ui::command::watch::Handler as WatchpointHandler;
use crate::ui::command::{Command, run};
use crate::ui::command::{
    CommandError, r#break, source_code, step_instruction, step_into, step_out, step_over,
};
use crate::ui::generic::r#async::print_backtrace;
use crate::ui::generic::r#async::print_backtrace_full;
use crate::ui::generic::r#async::print_task_ex;
use crate::ui::generic::file::FileView;
use crate::ui::generic::help::*;
use crate::ui::generic::print::ExternalPrinter;
use crate::ui::generic::print::style::{
    AddressView, AsmInstructionView, AsmOperandsView, ErrorView, FilePathView, FunctionNameView,
    KeywordView,
};
use crate::ui::generic::variable::render_variable;
use crate::ui::short::Abbreviator;
use r#break::Command as BreakpointCommand;
use command::trigger::Command as UserCommandTarget;
use crossterm::style::Stylize;
use debugger::Error;
use itertools::Itertools;

pub trait YesQuestion {
    fn yes(&self, question: &str) -> Result<bool, CommandError>;
}

pub trait Completer {
    fn update_completer_variables(&self, debugger: &Debugger) -> anyhow::Result<()>;
}

pub trait ProgramTaker {
    fn take_user_command_list(
        &self,
        help: &str,
    ) -> Result<super::trigger::UserProgram, CommandError>;
}

pub struct CommandHandler<'a, Y: YesQuestion, C: Completer, U: ProgramTaker> {
    pub yes_handler: Y,
    pub complete_handler: C,
    pub prog_taker: U,
    pub trigger_reg: &'a TriggerRegistry,
    pub debugger: &'a mut Debugger,
    pub printer: &'a ExternalPrinter,
    pub file_view: &'a FileView,
    pub helper: &'a Helper,
}

impl<Y: YesQuestion, C: Completer, U: ProgramTaker> CommandHandler<'_, Y, C, U> {
    pub fn handle_command(&mut self, cmd: Command) -> Result<(), CommandError> {
        match cmd {
            Command::Print(print_var_command) => PrintHandler::new(self.debugger)
                .handle(print_var_command)?
                .into_iter()
                .for_each(|var| {
                    let string_to_render = match var {
                        command::print::ReadVariableResult::PreRender(qr, val) => {
                            render_variable(&qr, Some(&val))
                        }
                        command::print::ReadVariableResult::Raw(qr) => render_variable(&qr, None),
                    }
                    .unwrap_or(super::print::style::UNKNOWN_PLACEHOLDER.to_string());
                    self.printer.println(string_to_render)
                }),
            Command::PrintBacktrace(cmd) => {
                let bt = BacktraceHandler::new(self.debugger).handle(cmd)?;
                bt.into_iter().for_each(|thread| {
                    let ip = thread
                        .bt
                        .as_ref()
                        .and_then(|bt| bt.first().map(|f| f.ip.to_string()));

                    self.printer.println(format!(
                        "thread #{}, {} - {}",
                        thread.thread.number,
                        thread.thread.pid,
                        AddressView::from(ip),
                    ));

                    let abbreviator = Abbreviator::new("/", "/..", 30);

                    if let Some(bt) = thread.bt {
                        for (frame_num, frame) in bt.into_iter().enumerate() {
                            let fn_name = frame.func_name.clone().unwrap_or_default();

                            let user_bt_end = fn_name == "main"
                                //|| fn_name.contains("::main")
                                || fn_name.contains("::thread_start");

                            let place = frame.place;

                            let file_and_line = place
                                .map(|p| {
                                    format!(
                                        "at {}:{}",
                                        FilePathView::from(
                                            abbreviator.apply(&p.file.to_string_lossy())
                                        ),
                                        p.line_number
                                    )
                                })
                                .unwrap_or_default();

                            let mut frame_info = format!(
                                "#{frame_num} {} - {} {}",
                                AddressView::from(frame.ip),
                                FunctionNameView::from(frame.func_name),
                                file_and_line
                            );
                            if thread.focus_frame == Some(frame_num) {
                                frame_info = frame_info.bold().to_string();
                            }

                            self.printer.println(frame_info);
                            if user_bt_end {
                                break;
                            }
                        }
                    }
                });
            }
            Command::Continue => {
                ContinueHandler::new(self.debugger).handle()?;
                _ = self
                    .complete_handler
                    .update_completer_variables(self.debugger);
            }
            Command::Frame(cmd) => {
                let result = FrameHandler::new(self.debugger).handle(cmd)?;
                match result {
                    FrameResult::FrameInfo(frame) => {
                        self.printer.println(format!(
                            "frame #{} ({})",
                            frame.num,
                            FunctionNameView::from(frame.frame.func_name),
                        ));
                        let cfa = AddressView::from(frame.cfa);
                        self.printer.println(format!("cfa: {cfa}"));
                        let ret_addr = AddressView::from(frame.return_addr);
                        self.printer.println(format!("return address: {ret_addr}"));
                    }
                    FrameResult::BroughtIntoFocus(num) => {
                        self.printer.println(format!("switch to #{num}"));
                    }
                }
            }
            Command::Run => match RunHandler::new(self.debugger).handle(run::Command::Start) {
                Err(CommandError::Handle(Error::AlreadyRun)) => {
                    if self.yes_handler.yes("Restart a program?")? {
                        RunHandler::new(self.debugger).handle(run::Command::Restart)?
                    }
                }
                Err(e) => return Err(e),
                _ => {
                    _ = self
                        .complete_handler
                        .update_completer_variables(self.debugger);
                }
            },
            Command::StepInstruction => {
                step_instruction::Handler::new(self.debugger).handle()?;
                _ = self
                    .complete_handler
                    .update_completer_variables(self.debugger);
            }
            Command::StepInto => {
                step_into::Handler::new(self.debugger).handle()?;
                _ = self
                    .complete_handler
                    .update_completer_variables(self.debugger);
            }
            Command::StepOut => {
                step_out::Handler::new(self.debugger).handle()?;
                _ = self
                    .complete_handler
                    .update_completer_variables(self.debugger);
            }
            Command::StepOver => {
                step_over::Handler::new(self.debugger).handle()?;
                _ = self
                    .complete_handler
                    .update_completer_variables(self.debugger);
            }
            Command::Breakpoint(mut brkpt_cmd) => {
                let print_bp = |action: &str, bp: &debugger::BreakpointView| match &bp.place {
                    None => {
                        self.printer.println(format!(
                            "{action} {} at {}",
                            bp.number,
                            AddressView::from(bp.addr),
                        ));
                    }
                    Some(place) => {
                        self.printer.println(format!(
                            "{action} {} at {}: {}:{} ",
                            bp.number,
                            AddressView::from(place.address),
                            FilePathView::from(place.file.to_string_lossy()),
                            place.line_number,
                        ));
                    }
                };

                loop {
                    match BreakpointHandler::new(self.debugger).handle(&brkpt_cmd) {
                        Ok(r#break::ExecutionResult::New(brkpts)) => {
                            brkpts.iter().for_each(|brkpt| {
                                print_bp("New breakpoint", brkpt);
                                self.trigger_reg.set_previous_brkpt(brkpt.number);
                            });
                        }
                        Ok(r#break::ExecutionResult::Removed(brkpts)) => {
                            brkpts.iter().for_each(|brkpt| {
                                print_bp("Removed breakpoint", brkpt);
                                self.trigger_reg
                                    .remove(TriggerEvent::Breakpoint(brkpt.number));
                            });
                        }
                        Ok(r#break::ExecutionResult::Dump(brkpts)) => brkpts
                            .iter()
                            .for_each(|brkpt| print_bp("- Breakpoint", brkpt)),
                        Err(Error::NoSuitablePlace) => {
                            if self
                                .yes_handler
                                .yes("Add deferred breakpoint for future shared library load?")?
                            {
                                brkpt_cmd = BreakpointCommand::AddDeferred(
                                    brkpt_cmd
                                        .identity()
                                        .expect("unreachable: deferred breakpoint must based on exists breakpoint"),
                                );
                                continue;
                            }
                        }
                        Ok(ExecutionResult::AddDeferred) => {
                            self.printer.println("Add deferred endpoint")
                        }
                        Err(e) => return Err(e.into()),
                    }
                    break;
                }
            }
            Command::Watchpoint(cmd) => {
                let print_wp = |prefix: &str, wp: debugger::WatchpointView| {
                    let source_expr = wp
                        .source_dqe
                        .map(|dqe| format!(", expression: {dqe}"))
                        .unwrap_or_default();
                    let (addr, cond) = (
                        AddressView::from(wp.address),
                        KeywordView::from(wp.condition),
                    );
                    self.printer.println(format!(
                        "{prefix} {} at {}, condition: {}, watch size: {}{source_expr}",
                        wp.number, addr, cond, wp.size
                    ))
                };

                let mut handler = WatchpointHandler::new(self.debugger);
                let res = handler.handle(cmd)?;
                match res {
                    WatchpointExecutionResult::New(wp) => {
                        self.trigger_reg.set_previous_wp(wp.number);
                        print_wp("New watchpoint", wp);
                    }
                    WatchpointExecutionResult::Removed(Some(wp)) => {
                        self.trigger_reg.remove(TriggerEvent::Watchpoint(wp.number));
                        print_wp("Removed watchpoint", wp);
                    }
                    WatchpointExecutionResult::Removed(_) => {
                        self.printer.println("No watchpoint found")
                    }
                    WatchpointExecutionResult::Dump(wps) => {
                        self.printer
                            .println(format!("{}/4 active watchpoints:", wps.len()));
                        for wp in wps {
                            print_wp("- Watchpoint", wp)
                        }
                    }
                }
            }
            Command::Memory(mem_cmd) => {
                let read = MemoryHandler::new(self.debugger).handle(mem_cmd)?;
                for b in read {
                    self.printer.print(format!("0x{b:X} "));
                }
                self.printer.println("");
            }
            Command::Register(reg_cmd) => {
                let response = RegisterHandler::new(self.debugger).handle(&reg_cmd)?;
                response.iter().for_each(|register| {
                    self.printer.println(format!(
                        "{:10} {:#016X}",
                        register.register_name, register.value
                    ));
                });
            }
            Command::Help { reason, command } => {
                if let Some(reason) = reason {
                    self.printer.println(reason);
                }
                self.printer
                    .println(self.helper.help_for_command(command.as_deref()));
            }
            Command::SkipInput => {}
            Command::PrintSymbol(symbol) => {
                let symbols = SymbolHandler::new(self.debugger).handle(&symbol)?;
                for symbol in symbols {
                    self.printer.println(format!(
                        "{} - {:?} {}",
                        symbol.name,
                        symbol.kind,
                        AddressView::from(symbol.addr)
                    ));
                }
            }
            Command::Thread(cmd) => {
                let result = command::thread::Handler::new(self.debugger).handle(cmd)?;
                match result {
                    ThreadResult::List(mut list) => {
                        list.sort_by(|t1, t2| t1.thread.number.cmp(&t2.thread.number));
                        for thread in list {
                            let current_frame = thread.bt.and_then(|mut bt| bt.drain(..).next());
                            let ip = current_frame.as_ref().map(|f| f.ip.to_string());
                            let func = current_frame.and_then(|f| f.func_name);

                            let view = format!(
                                "#{} thread id: {}, {} in {}",
                                thread.thread.number,
                                thread.thread.pid,
                                AddressView::from(ip),
                                FunctionNameView::from(func),
                            );

                            if thread.in_focus {
                                self.printer.println(format!("{}", view.bold()))
                            } else {
                                self.printer.println(view)
                            }
                        }
                    }
                    ThreadResult::BroughtIntoFocus(thread) => self
                        .printer
                        .println(format!("Thread #{} brought into focus", thread.number)),
                }
            }
            Command::SharedLib => {
                let handler = SharedlibHandler::new(self.debugger);
                for lib in handler.handle() {
                    let mb_range = lib
                        .range
                        .map(|range| format!("{} - {}", range.from, range.to));

                    self.printer.println(format!(
                        "{}  {}  {}",
                        AddressView::from(mb_range),
                        if !lib.has_debug_info { "*" } else { " " },
                        FilePathView::from(lib.path.to_string_lossy())
                    ))
                }
            }
            Command::SourceCode(inner_cmd) => match inner_cmd {
                source_code::Command::Range(bounds) => {
                    let handler = FunctionLineRangeHandler::new(self.debugger);
                    let range = handler.handle()?;

                    self.printer.println(format!(
                        "{} at {}:{}",
                        FunctionNameView::from(range.name),
                        FilePathView::from(range.stop_place.file.to_string_lossy()),
                        range.stop_place.line_number,
                    ));

                    self.printer.print(
                        self.file_view
                            .render_source(&range.stop_place, bounds)
                            .map_err(CommandError::FileRender)?,
                    );
                }
                source_code::Command::Function => {
                    let handler = FunctionLineRangeHandler::new(self.debugger);
                    let range = handler.handle()?;

                    self.printer.println(format!(
                        "{} at {}:{}",
                        FunctionNameView::from(range.name),
                        FilePathView::from(range.stop_place.file.to_string_lossy()),
                        range.stop_place.line_number,
                    ));

                    self.printer.print(
                        self.file_view
                            .render_source_range(range.file, range.start_line, range.end_line)
                            .map_err(CommandError::FileRender)?,
                    );
                }
                source_code::Command::Asm => {
                    let handler = DisAsmHandler::new(self.debugger);
                    let assembly = handler.handle()?;
                    self.printer.println(format!(
                        "Assembler code for function {}",
                        FunctionNameView::from(assembly.name)
                    ));
                    for ins in assembly.instructions {
                        let instruction_view = format!(
                            "{} {} {}",
                            AddressView::from(ins.address),
                            AsmInstructionView::from(ins.mnemonic),
                            AsmOperandsView::from(ins.operands),
                        );

                        if ins.address == assembly.addr_in_focus {
                            self.printer.println(format!("{}", instruction_view.bold()));
                        } else {
                            self.printer.println(instruction_view);
                        }
                    }
                }
            },
            Command::Async(cmd) => {
                let mut handler = command::r#async::Handler::new(self.debugger);
                let result: command::r#async::AsyncCommandResult = handler.handle(&cmd)?;

                match result {
                    AsyncCommandResult::ShortBacktrace(bt) => {
                        print_backtrace(&bt, self.printer);
                    }
                    AsyncCommandResult::FullBacktrace(bt) => {
                        print_backtrace_full(&bt, self.printer);
                    }
                    AsyncCommandResult::CurrentTask(bt, regex) => {
                        print_task_ex(&bt, self.printer, regex);
                    }
                    AsyncCommandResult::StepOver => {
                        _ = self
                            .complete_handler
                            .update_completer_variables(self.debugger);
                    }
                    AsyncCommandResult::StepOut => {
                        _ = self
                            .complete_handler
                            .update_completer_variables(self.debugger);
                    }
                }
            }
            Command::Trigger(cmd) => {
                let event = match cmd {
                    UserCommandTarget::AttachToPreviouslyCreated => {
                        let Some(event) = self.trigger_reg.get_previous_event() else {
                            self.printer.println(ErrorView::from(
                                "No previously added watchpoints or breakpoints exist",
                            ));
                            return Ok(());
                        };
                        event
                    }
                    UserCommandTarget::AttachToDefined(event) => event,
                    UserCommandTarget::Info => {
                        self.printer.println(format!("{:<30}  Program", "Event"));

                        self.trigger_reg.for_each_trigger(|event, program| {
                            self.printer.println(format!(
                                "{:<30}  {}",
                                event.to_string(),
                                KeywordView::from(program.iter().map(|(_, str)| str).join(", ")),
                            ));
                        });
                        return Ok(());
                    }
                };

                let help = match event {
                    TriggerEvent::Breakpoint(num) => format!("Print the commands to be executed on breakpoint {num}"),
                    TriggerEvent::Watchpoint(num) => format!("Print the commands to be executed on watchpoint {num}"),
                    TriggerEvent::Any => "Print the commands to be executed on each breakpoint or watchpoint, and print 'end' for end".to_string(),
                };

                let commands = self.prog_taker.take_user_command_list(&help)?;
                self.trigger_reg.add(event, commands);
            }
            Command::Call(call) => {
                let mut handler = command::call::Handler::new(self.debugger);
                handler.handle(call)?;
            }
            Command::Oracle(name, subcmd) => match self.debugger.get_oracle(&name) {
                None => self
                    .printer
                    .println(ErrorView::from("Oracle not found or not ready")),
                Some(oracle) => oracle.print(self.printer, subcmd.as_deref()),
            },
        }

        Ok(())
    }
}
