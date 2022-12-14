use crate::cui::file_view::FileView;
use crate::cui::{context, AppState};
use crate::debugger::{EventHook, Place};
use nix::libc::c_int;
use std::rc::Rc;
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};

pub struct CuiHook {
    file_view: Rc<FileView>,
}

impl CuiHook {
    pub fn new(file_view: Rc<FileView>) -> Self {
        Self { file_view }
    }
}

impl EventHook for CuiHook {
    fn on_trap(&self, _: usize, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let (code, pos) = self.file_view.render_source(place).unwrap();
            let ctx = context::Context::current();
            ctx.set_render_file_name(place.file.to_string());
            ctx.set_render_text(Text::from(code));
            ctx.set_render_text_pos(pos);
            ctx.change_state(AppState::DebugeeBreak);
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
        context::Context::current().set_alert(alert_text.into());
    }
}
