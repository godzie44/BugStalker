use crate::console::view::FileView;
use crate::debugger::Place;
use crate::debugger::{EventHook, RelocatedAddress};
use nix::libc::c_int;

pub(super) struct TerminalHook {
    file_view: FileView,
}

impl TerminalHook {
    pub(super) fn new(file_view: FileView) -> Self {
        Self { file_view }
    }
}

impl EventHook for TerminalHook {
    fn on_trap(&self, pc: RelocatedAddress, mb_place: Option<Place>) -> anyhow::Result<()> {
        println!("Hit breakpoint at address {:#016X}", pc.0);
        if let Some(place) = mb_place {
            println!("{}:{}", place.file.display(), place.line_number);
            println!("{}", self.file_view.render_source(&place, 1)?);
        }
        Ok(())
    }

    fn on_signal(&self, signo: c_int, code: c_int) {
        println!("Receive signal {signo}, reason: {code}")
    }

    fn on_exit(&self, code: i32) {
        println!("Program exit with code: {code}");
    }
}
