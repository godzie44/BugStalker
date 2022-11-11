use crate::debugger::dwarf::parse::Place;
use crate::debugger::ui::view::FileView;
use crate::debugger::EventHook;
use std::rc::Rc;

pub struct TerminalHook {
    file_view: Rc<FileView>,
}

impl TerminalHook {
    pub fn new(file_view: Rc<FileView>) -> Self {
        Self { file_view }
    }
}

impl EventHook for TerminalHook {
    fn on_sigtrap(&self, pc: usize, mb_place: Option<Place>) -> anyhow::Result<()> {
        println!("Hit breakpoint at address {:#016X}", pc);
        if let Some(place) = mb_place {
            println!("{}:{}", place.file, place.line_number);
            println!("{}", self.file_view.render_source(&place, 1)?);
        }
        Ok(())
    }
}
