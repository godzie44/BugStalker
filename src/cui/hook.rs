use crate::cui::file_view::FileView;
use crate::cui::{AppContext, AppState};
use crate::debugger::{EventHook, Place};
use std::rc::Rc;
use tui::text::Text;

pub struct CuiHook {
    app_ctx: AppContext,
    file_view: Rc<FileView>,
}

impl CuiHook {
    pub fn new(app_ctx: AppContext, file_view: Rc<FileView>) -> Self {
        Self { app_ctx, file_view }
    }
}

impl EventHook for CuiHook {
    fn on_sigtrap(&self, _: usize, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let (code, pos) = self.file_view.render_source(place).unwrap();
            *self.app_ctx.data.debugee_file_name.borrow_mut() = place.file.to_string();
            *self.app_ctx.data.debugee_text.borrow_mut() = Text::from(code);
            self.app_ctx.data.debugee_text_pos.set(pos);
            self.app_ctx.change_state(AppState::DebugeeBreak);
        }
        Ok(())
    }
}
