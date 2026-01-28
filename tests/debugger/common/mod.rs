use bugstalker::debugger::address::RelocatedAddress;
use bugstalker::debugger::register::debug::BreakCondition;
use bugstalker::debugger::variable::value::Value;
use bugstalker::debugger::{EventHook, FunctionInfo, PlaceDescriptor};
use bugstalker::version::RustVersion;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use std::cell::{Cell, RefCell};
use std::fs;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct TestInfo {
    pub addr: Arc<Cell<Option<RelocatedAddress>>>,
    pub line: Arc<Cell<Option<u64>>>,
    pub file: Arc<Cell<Option<String>>>,
    pub wp_dqe_string: Arc<RefCell<Option<String>>>,
    pub old_value: Arc<RefCell<Option<Value>>>,
    pub new_value: Arc<RefCell<Option<Value>>>,
}

#[derive(Default)]
pub struct TestHooks {
    info: TestInfo,
}

impl TestHooks {
    pub fn new(info: TestInfo) -> Self {
        Self { info }
    }
}

impl EventHook for TestHooks {
    fn on_breakpoint(
        &self,
        pc: RelocatedAddress,
        _: u32,
        place: Option<PlaceDescriptor>,
        _: Option<&FunctionInfo>,
        _: Option<u32>,
    ) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }

    fn on_watchpoint(
        &self,
        pc: RelocatedAddress,
        _: u32,
        place: Option<PlaceDescriptor>,
        _: BreakCondition,
        dqe_string: Option<&str>,
        old_value: Option<&Value>,
        new_value: Option<&Value>,
        _: bool,
    ) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        self.info
            .wp_dqe_string
            .replace(dqe_string.map(ToString::to_string));
        self.info.old_value.replace(old_value.cloned());
        self.info.new_value.replace(new_value.cloned());
        Ok(())
    }

    fn on_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        _: Option<&FunctionInfo>,
        _: Option<u32>,
    ) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }

    fn on_async_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionInfo>,
        _: u64,
        _: bool,
    ) -> anyhow::Result<()> {
        self.on_step(pc, place, function, None)
    }

    fn on_signal(&self, _: Signal) {}
    fn on_exit(&self, _code: i32) {}
    fn on_process_install(&self, _pid: Pid, _: Option<&object::File>) {}
}

#[macro_export]
macro_rules! assert_no_proc {
    ($pid:expr) => {
        // Give the system a bit of time for process cleanup
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        let sys = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::everything()
                .without_cpu()
                .without_memory(),
        );
        assert!(
            sysinfo::System::process(&sys, sysinfo::Pid::from_u32($pid.as_raw() as u32)).is_none(),
            "Process {} should have been terminated but still exists",
            $pid
        )
    };
}

pub fn rust_version(file: &str) -> Option<RustVersion> {
    let file = fs::File::open(file).unwrap();
    let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
    let object = object::File::parse(&*mmap).unwrap();
    let sect = object
        .section_by_name(".comment")
        .expect(".comment section not found");

    let data = sect.data().unwrap();
    let string_data = std::str::from_utf8(data).unwrap();

    RustVersion::parse(string_data)
}

/// Wait for a debugger event with retries (useful for multithreaded tests)
pub fn wait_for_stop_line(info: &TestInfo, expected_line: u64, max_retries: u32) -> u64 {
    for attempt in 0..max_retries {
        if let Some(line) = info.line.take() {
            return line;
        }
        if attempt < max_retries - 1 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    panic!("Timeout waiting for breakpoint at line {}, retries exhausted", expected_line);
}
