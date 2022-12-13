use crate::cui::file_view::FileView;
use crate::cui::{AppContext, AppState};
use crate::debugger::{EventHook, Place};
use nix::libc::c_int;
use std::rc::Rc;
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};

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
    fn on_trap(&self, _: usize, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let (code, pos) = self.file_view.render_source(place).unwrap();
            *self.app_ctx.data.debugee_file_name.borrow_mut() = place.file.to_string();
            *self.app_ctx.data.debugee_text.borrow_mut() = Text::from(code);
            self.app_ctx.data.debugee_text_pos.set(pos);
            self.app_ctx.change_state(AppState::DebugeeBreak);
        }
        Ok(())
    }

    fn on_signal(&self, signo: c_int, code: c_int) {
        let alert_text = vec![
            Spans::from(vec![
                Span::raw("Application receive signal: "),
                Span::styled(format!("{signo}"), Style::default().fg(Color::Red)),
            ]),
            Spans::from(vec![Span::raw(format!("Reason: {code}"))]),
        ];
        self.app_ctx.data.set_alert(alert_text.into());
    }
}
