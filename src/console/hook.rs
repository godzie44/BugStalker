use crate::console::view::FileView;
use crate::debugger::address::RelocatedAddress;
use crate::debugger::EventHook;
use crate::debugger::Place;
use nix::sys::signal::Signal;

pub(super) struct TerminalHook {
    file_view: FileView,
}

impl TerminalHook {
    pub(super) fn new(file_view: FileView) -> Self {
        Self { file_view }
    }
}

impl EventHook for TerminalHook {
    fn on_breakpoint(&self, pc: RelocatedAddress, mb_place: Option<Place>) -> anyhow::Result<()> {
        println!("Hit breakpoint at address {}", pc);
        if let Some(place) = mb_place {
            println!("{}:{}", place.file.display(), place.line_number);
            println!("{}", self.file_view.render_source(&place, 1)?);
        } else {
            println!("undefined place");
        }
        Ok(())
    }

    fn on_step(&self, _: RelocatedAddress, mb_place: Option<Place>) -> anyhow::Result<()> {
        if let Some(place) = mb_place {
            println!("{}:{}", place.file.display(), place.line_number);
            println!("{}", self.file_view.render_source(&place, 1)?);
        } else {
            println!("undefined place, go to next");
        }
        Ok(())
    }

    fn on_signal(&self, signal: Signal) {
        println!("Receive signal {signal}, debugee stopped")
    }

    fn on_exit(&self, code: i32) {
        println!("Program exit with code: {code}");
    }
}
