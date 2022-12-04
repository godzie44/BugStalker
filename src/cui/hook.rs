use crate::cui::file_view::FileView;
use crate::cui::SharedRenderData;
use crate::debugger::{EventHook, Place};
use std::rc::Rc;
use tui::text::Text;

pub struct CuiHook {
    render_data: SharedRenderData,
    file_view: Rc<FileView>,
}

impl CuiHook {
    pub fn new(render_data: SharedRenderData, file_view: Rc<FileView>) -> Self {
        Self {
            render_data,
            file_view,
        }
    }
}

impl EventHook for CuiHook {
    fn on_sigtrap(&self, _: usize, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let code = self.file_view.render_source(place, 5).unwrap();
            *self.render_data.main_text.borrow_mut() = Text::from(code);
        }
        Ok(())
    }
}
