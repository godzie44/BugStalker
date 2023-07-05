use bugstalker::debugger::address::RelocatedAddress;
use bugstalker::debugger::{EventHook, Place};
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use std::cell::Cell;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct DebugeeRunInfo {
    pub addr: Arc<Cell<Option<RelocatedAddress>>>,
    pub line: Arc<Cell<Option<u64>>>,
    pub file: Arc<Cell<Option<String>>>,
}

#[derive(Default)]
pub struct TestHooks {
    info: DebugeeRunInfo,
}

impl TestHooks {
    pub fn new(info: DebugeeRunInfo) -> Self {
        Self { info }
    }
}

impl EventHook for TestHooks {
    fn on_breakpoint(&self, pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }
    fn on_step(&self, pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }
    fn on_signal(&self, _: Signal) {}
    fn on_exit(&self, _code: i32) {}
    fn on_process_install(&self, _pid: Pid) {}
}

#[macro_export]
macro_rules! assert_no_proc {
    ($pid:expr) => {
        use sysinfo::{PidExt, SystemExt};

        let sys = sysinfo::System::new_all();
        assert!(sys
            .process(sysinfo::Pid::from_u32($pid.as_raw() as u32))
            .is_none())
    };
}
