use crate::cui::{context, AppState};
use crate::debugger::{EventHook, Place};
use nix::libc::c_int;
use tui::style::{Color, Style};
use tui::text::{Span, Spans};

#[derive(Default)]
pub struct CuiHook {}

impl CuiHook {
    pub fn new() -> Self {
        Self {}
    }
}

impl EventHook for CuiHook {
    fn on_trap(&self, _: usize, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let ctx = context::Context::current();
            ctx.set_trap_file_name(place.file.to_string());
            ctx.set_trap_text_pos(place.line_number);
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
