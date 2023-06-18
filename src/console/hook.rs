use crate::console::print::ExternalPrinter;
use crate::console::view::FileView;
use crate::debugger::address::RelocatedAddress;
use crate::debugger::EventHook;
use crate::debugger::Place;
use nix::sys::signal::Signal;
use nix::unistd::Pid;

pub struct TerminalHook {
    file_view: FileView,
    on_install_proc: Box<dyn Fn(Pid)>,
    printer: ExternalPrinter,
}

impl TerminalHook {
    pub fn new(printer: ExternalPrinter, on_install_proc: impl Fn(Pid) + 'static) -> Self {
        Self {
            file_view: FileView::new(),
            on_install_proc: Box::new(on_install_proc),
            printer,
        }
    }
}

impl EventHook for TerminalHook {
    fn on_breakpoint(&self, pc: RelocatedAddress, mb_place: Option<Place>) -> anyhow::Result<()> {
        self.printer
            .print(format!("Hit breakpoint at address {pc}"));
        if let Some(place) = mb_place {
            self.printer
                .print(format!("{}:{}", place.file.display(), place.line_number));
            self.printer.print(self.file_view.render_source(&place, 1)?);
        } else {
            self.printer.print("undefined place");
        }
        Ok(())
    }

    fn on_step(&self, _: RelocatedAddress, mb_place: Option<Place>) -> anyhow::Result<()> {
        if let Some(place) = mb_place {
            self.printer
                .print(format!("{}:{}", place.file.display(), place.line_number));
            self.printer.print(self.file_view.render_source(&place, 1)?);
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
