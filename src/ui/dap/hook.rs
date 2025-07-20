use std::io::Stdout;
use std::sync::{Arc, Mutex};

use dap::events::{Event, ExitedEventBody, StoppedEventBody};
use dap::server::ServerOutput;
use dap::types::StoppedEventReason;

use crate::debugger::EventHook;

pub struct DapHook {
    output: Arc<Mutex<ServerOutput<Stdout>>>,
}

impl DapHook {
    pub fn new(output: Arc<Mutex<ServerOutput<Stdout>>>) -> DapHook {
        DapHook { output }
    }
}

impl EventHook for DapHook {
    fn on_breakpoint(
        &self,
        _pc: crate::debugger::address::RelocatedAddress,
        num: u32,
        _place: Option<crate::debugger::PlaceDescriptor>,
        _function: Option<&crate::debugger::FunctionDie>,
    ) -> anyhow::Result<()> {
        let mut output = self.output.lock().unwrap();

        output.send_event(Event::Stopped(StoppedEventBody {
            reason: StoppedEventReason::Breakpoint,
            description: None,
            thread_id: Some(1),
            preserve_focus_hint: None,
            text: None,
            all_threads_stopped: None,
            hit_breakpoint_ids: Some(vec![num.into()]),
        }))?;

        Ok(())
    }

    fn on_watchpoint(
        &self,
        _pc: crate::debugger::address::RelocatedAddress,
        _num: u32,
        _place: Option<crate::debugger::PlaceDescriptor>,
        _condition: crate::debugger::register::debug::BreakCondition,
        _dqe_string: Option<&str>,
        _old_value: Option<&crate::debugger::variable::value::Value>,
        _new_value: Option<&crate::debugger::variable::value::Value>,
        _end_of_scope: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_step(
        &self,
        _pc: crate::debugger::address::RelocatedAddress,
        _place: Option<crate::debugger::PlaceDescriptor>,
        _function: Option<&crate::debugger::FunctionDie>,
    ) -> anyhow::Result<()> {
        let mut output = self.output.lock().unwrap();

        output.send_event(Event::Stopped(StoppedEventBody {
            reason: StoppedEventReason::Step,
            description: None,
            thread_id: Some(1),
            preserve_focus_hint: None,
            text: None,
            all_threads_stopped: None,
            hit_breakpoint_ids: None,
        }))?;

        Ok(())
    }

    fn on_async_step(
        &self,
        _pc: crate::debugger::address::RelocatedAddress,
        _place: Option<crate::debugger::PlaceDescriptor>,
        _function: Option<&crate::debugger::FunctionDie>,
        _task_id: u64,
        _task_completed: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_signal(&self, _signal: nix::sys::signal::Signal) {}

    fn on_exit(&self, code: i32) {
        let mut output = self.output.lock().unwrap();

        _ = output.send_event(Event::Terminated(None));

        _ = output.send_event(Event::Exited(ExitedEventBody {
            exit_code: code.into(),
        }));
    }

    fn on_process_install(&self, _pid: thread_db::Pid, _object: Option<&object::File>) {}
}
