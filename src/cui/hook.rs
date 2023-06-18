use crate::cui::{context, AppState};
use crate::debugger::address::RelocatedAddress;
use crate::debugger::{EventHook, Place};
use nix::sys::signal::Signal;
use nix::unistd::Pid;
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
    fn on_breakpoint(&self, _: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let ctx = context::Context::current();
            ctx.set_trap_file_name(place.file.to_path_buf().to_string_lossy().to_string());
            ctx.set_trap_text_pos(place.line_number);
            ctx.change_state(AppState::DebugeeBreak);
        }
        Ok(())
    }

    fn on_step(&self, _: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()> {
        if let Some(ref place) = place {
            let ctx = context::Context::current();
            ctx.set_trap_file_name(place.file.to_path_buf().to_string_lossy().to_string());
            ctx.set_trap_text_pos(place.line_number);
            ctx.change_state(AppState::DebugeeBreak);
        }
        Ok(())
    }

    fn on_signal(&self, signal: Signal) {
        let alert_text = vec![Spans::from(vec![
            Span::raw("Application receive signal: "),
            Span::styled(format!("{signal}"), Style::default().fg(Color::Red)),
        ])];
        context::Context::current().set_alert(alert_text.into());
    }

    fn on_exit(&self, _code: i32) {
        context::Context::current().change_state(AppState::Finish)
    }

    fn on_process_install(&self, _pid: Pid) {}
}
