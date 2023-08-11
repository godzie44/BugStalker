use crate::console::print::ExternalPrinter;
use crate::console::view::FileView;
use crate::debugger::address::RelocatedAddress;
use crate::debugger::PlaceDescriptor;
use crate::debugger::{EventHook, FunctionDie};
use crossterm::style::Stylize;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use std::cell::RefCell;
use std::ops::Add;

#[derive(Default)]
struct Context {
    prev_func: Option<FunctionDie>,
}

pub struct TerminalHook {
    file_view: FileView,
    on_install_proc: Box<dyn Fn(Pid)>,
    printer: ExternalPrinter,
    context: RefCell<Context>,
}

impl TerminalHook {
    pub fn new(printer: ExternalPrinter, on_install_proc: impl Fn(Pid) + 'static) -> Self {
        Self {
            file_view: FileView::new(),
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
        let msg = format!("Hit breakpoint {num} at {}:", format!("{pc}").blue());
        if let Some(place) = mb_place {
            self.printer.print(format!(
                "{msg} {}:{}",
                place.file.as_os_str().to_string_lossy().green(),
                place.line_number
            ));
            self.printer.print(self.file_view.render_source(&place, 0)?);
        } else {
            self.printer.print(format!("{msg} undefined place"));
        }

        self.context.borrow_mut().prev_func = mb_func.cloned();

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

                let func_name = mb_func
                    .map(|f| {
                        f.namespace
                            .join("::")
                            .add("::")
                            .add(f.base_attributes.name.as_deref().unwrap_or_default())
                    })
                    .unwrap_or_default();

                self.printer.print(format!(
                    "{func_name} at {}:{}",
                    place.file.as_os_str().to_string_lossy().green(),
                    place.line_number,
                ));
            }

            self.printer.print(self.file_view.render_source(&place, 0)?);
        } else {
            self.printer.print("undefined place, go to next");
        }

        Ok(())
    }

    fn on_signal(&self, signal: Signal) {
        self.printer
            .print(format!("Receive signal {signal}, debugee stopped"));
    }

    fn on_exit(&self, code: i32) {
        self.printer
            .print(format!("Program exit with code: {code}"));
    }

    fn on_process_install(&self, pid: Pid) {
        (self.on_install_proc)(pid)
    }
}
