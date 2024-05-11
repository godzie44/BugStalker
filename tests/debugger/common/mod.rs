use bugstalker::debugger::address::RelocatedAddress;
use bugstalker::debugger::register::debug::BreakCondition;
use bugstalker::debugger::variable::VariableIR;
use bugstalker::debugger::{EventHook, FunctionDie, PlaceDescriptor};
use bugstalker::version::Version;
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
    pub old_value: Arc<RefCell<Option<VariableIR>>>,
    pub new_value: Arc<RefCell<Option<VariableIR>>>,
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
        _: Option<&FunctionDie>,
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
        old_value: Option<&VariableIR>,
        new_value: Option<&VariableIR>,
        _: bool,
    ) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        self.info.old_value.replace(old_value.cloned());
        self.info.new_value.replace(new_value.cloned());
        Ok(())
    }

    fn on_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        _: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        self.info.addr.set(Some(pc));
        let file = &self.info.file;
        file.set(place.as_ref().map(|p| p.file.to_str().unwrap().to_string()));
        self.info.line.set(place.map(|p| p.line_number));
        Ok(())
    }
    fn on_signal(&self, _: Signal) {}
    fn on_exit(&self, _code: i32) {}
    fn on_process_install(&self, _pid: Pid, _: Option<&object::File>) {}
}

#[macro_export]
macro_rules! assert_no_proc {
    ($pid:expr) => {
        let sys = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::everything()
                .without_cpu()
                .without_memory(),
        );
        assert!(
            sysinfo::System::process(&sys, sysinfo::Pid::from_u32($pid.as_raw() as u32)).is_none()
        )
    };
}

pub fn rust_version(file: &str) -> Option<Version> {
    let file = fs::File::open(file).unwrap();
    let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
    let object = object::File::parse(&*mmap).unwrap();
    let sect = object
        .section_by_name(".comment")
        .expect(".comment section not found");

    let data = sect.data().unwrap();
    let string_data = std::str::from_utf8(data).unwrap();

    Version::rustc_parse(string_data)
}
