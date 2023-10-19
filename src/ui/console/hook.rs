use crate::debugger::address::RelocatedAddress;
use crate::debugger::PlaceDescriptor;
use crate::debugger::{EventHook, FunctionDie};
use crate::ui::console::print::style::{AddressView, FilePathView, FunctionNameView, KeywordView};
use crate::ui::console::print::ExternalPrinter;
use crate::ui::console::view::FileView;
use crate::ui::{context, AppState};
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
        context::Context::current().change_state(AppState::DebugeeBreak);

        let msg = format!("Hit breakpoint {num} at {}:", AddressView::from(pc));
        if let Some(place) = mb_place {
            self.printer.print(format!(
                "{msg} {}:{}",
                FilePathView::from(place.file.to_string_lossy()),
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
        context::Context::current().change_state(AppState::DebugeeBreak);

        if let Some(place) = mb_place {
            if self.context.borrow().prev_func.as_ref() != mb_func {
                self.context.borrow_mut().prev_func = mb_func.cloned();

                let func_name = mb_func.map(|f| {
                    f.namespace
                        .join("::")
                        .add("::")
                        .add(f.base_attributes.name.as_deref().unwrap_or_default())
                });

                self.printer.print(format!(
                    "{} at {}:{}",
                    FunctionNameView::from(func_name),
                    FilePathView::from(place.file.to_string_lossy()),
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
        self.printer.print(format!(
            "Signal {} received, debugee stopped",
            KeywordView::from(signal)
        ));
    }

    fn on_exit(&self, code: i32) {
        context::Context::current().change_state(AppState::Finish);
        self.printer.print(format!(
            "Program exit with code: {}",
            KeywordView::from(code)
        ));
    }

    fn on_process_install(&self, pid: Pid) {
        (self.on_install_proc)(pid)
    }
}
