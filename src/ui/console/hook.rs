use crate::debugger::address::RelocatedAddress;
use crate::debugger::register::debug::BreakCondition;
use crate::debugger::variable::VariableIR;
use crate::debugger::PlaceDescriptor;
use crate::debugger::{EventHook, FunctionDie};
use crate::ui::console::file::FileView;
use crate::ui::console::print::style::{AddressView, FilePathView, FunctionNameView, KeywordView};
use crate::ui::console::print::ExternalPrinter;
use crate::ui::console::variable::render_variable;
use crate::version;
use log::warn;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use std::cell::RefCell;
use std::ops::Add;
use std::rc::Rc;

#[derive(Default)]
struct Context {
    prev_func: Option<FunctionDie>,
}

pub struct TerminalHook {
    file_view: Rc<FileView>,
    on_install_proc: Box<dyn Fn(Pid)>,
    printer: ExternalPrinter,
    context: RefCell<Context>,
}

impl TerminalHook {
    pub fn new(
        printer: ExternalPrinter,
        fv: Rc<FileView>,
        on_install_proc: impl Fn(Pid) + 'static,
    ) -> Self {
        Self {
            file_view: fv,
            on_install_proc: Box::new(on_install_proc),
            printer,
            context: RefCell::new(Context::default()),
        }
    }
}

impl EventHook for TerminalHook {
    fn on_breakpoint(
        &self,
        pc: RelocatedAddress,
        num: u32,
        mb_place: Option<PlaceDescriptor>,
        mb_func: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        let msg = format!("Hit breakpoint {num} at {}:", AddressView::from(pc));
        if let Some(place) = mb_place {
            self.printer.println(format!(
                "{msg} {}:{}",
                FilePathView::from(place.file.to_string_lossy()),
                place.line_number
            ));
            self.printer.print(self.file_view.render_source(&place, 0)?);
        } else {
            self.printer.println(format!("{msg} undefined place"));
        }

        self.context.borrow_mut().prev_func = mb_func.cloned();

        Ok(())
    }

    fn on_watchpoint(
        &self,
        pc: RelocatedAddress,
        num: u32,
        mb_place: Option<PlaceDescriptor>,
        cond: BreakCondition,
        old: Option<&VariableIR>,
        new: Option<&VariableIR>,
        end_of_scope: bool,
    ) -> anyhow::Result<()> {
        let msg = if end_of_scope {
            format!(
                "Watchpoint {num} end of scope (and it will be removed)\n{}:",
                AddressView::from(pc)
            )
        } else {
            format!(
                "Hit watchpoint {num} ({cond}) at {}:",
                AddressView::from(pc)
            )
        };

        if let Some(place) = mb_place {
            self.printer.println(format!(
                "{msg} {}:{}",
                FilePathView::from(place.file.to_string_lossy()),
                place.line_number
            ))
        } else {
            self.printer.println(format!("{msg} undefined place"));
        };

        if cond == BreakCondition::DataReadsWrites && old == new {
            if let Some(old) = old {
                let val = render_variable(old)?;
                self.printer.println(format!("value: {val}"));
            }
        } else {
            if let Some(old) = old {
                let old = render_variable(old)?;
                self.printer.println(format!("old value: {old}"));
            }
            if let Some(new) = new {
                let new = render_variable(new)?;
                self.printer.println(format!("new value: {new}"));
            }
        }

        Ok(())
    }

    fn on_step(
        &self,
        _: RelocatedAddress,
        mb_place: Option<PlaceDescriptor>,
        mb_func: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        if let Some(place) = mb_place {
            if self.context.borrow().prev_func.as_ref() != mb_func {
                self.context.borrow_mut().prev_func = mb_func.cloned();

                let func_name = mb_func.map(|f| {
                    f.namespace
                        .join("::")
                        .add("::")
                        .add(f.base_attributes.name.as_deref().unwrap_or_default())
                });

                self.printer.println(format!(
                    "{} at {}:{}",
                    FunctionNameView::from(func_name),
                    FilePathView::from(place.file.to_string_lossy()),
                    place.line_number,
                ));
            }
            self.printer.print(self.file_view.render_source(&place, 0)?);
        } else {
            self.printer.println("undefined place, go to next");
        }

        Ok(())
    }

    fn on_signal(&self, signal: Signal) {
        self.printer.println(format!(
            "Signal {} received, debugee stopped",
            KeywordView::from(signal)
        ));
    }

    fn on_exit(&self, code: i32) {
        self.printer.println(format!(
            "Program exit with code: {}",
            KeywordView::from(code)
        ));
    }

    fn on_process_install(&self, pid: Pid, object: Option<&object::File>) {
        if let Some(obj) = object {
            if !version::probe_file(obj) {
                let supported_versions = version::supported_versions_to_string();
                warn!(target: "debugger", "Found unsupported rust version, some of program data may not be displayed correctly. \
                List of supported rustc versions: {supported_versions}.");
            }
        }
        (self.on_install_proc)(pid)
    }
}
