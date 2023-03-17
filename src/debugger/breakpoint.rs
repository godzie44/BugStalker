use crate::debugger::address::PCValue;
use nix::libc::c_void;
use nix::sys;
use nix::unistd::Pid;
use std::cell::Cell;

impl PCValue {
    fn as_ptr(&self) -> *mut c_void {
        match self {
            PCValue::Relocated(addr) => usize::from(*addr) as *mut c_void,
            PCValue::Global(_) => {
                panic!("only address with offset allowed")
            }
        }
    }
}

pub struct Breakpoint {
    pub addr: PCValue,
    pid: Pid,
    saved_data: Cell<u8>,
    enabled: Cell<bool>,
}

impl Breakpoint {
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.get()
    }
}

impl Breakpoint {
    pub fn new(addr: PCValue, pid: Pid) -> Self {
        Self {
            addr,
            pid,
            enabled: Default::default(),
            saved_data: Default::default(),
        }
    }

    pub fn enable(&self) -> nix::Result<()> {
        let data = sys::ptrace::read(self.pid, self.addr.as_ptr())?;
        self.saved_data.set((data & 0xff) as u8);
        let int3 = 0xCC_u64;
        let data_with_pb = (data & !0xff) as u64 | int3;
        unsafe {
            sys::ptrace::write(self.pid, self.addr.as_ptr(), data_with_pb as *mut c_void)?;
        }
        self.enabled.set(true);

        Ok(())
    }

    pub fn disable(&self) -> nix::Result<()> {
        let data = sys::ptrace::read(self.pid, self.addr.as_ptr())? as u64;
        let restored: u64 = (data & !0xff) | self.saved_data.get() as u64;
        unsafe {
            sys::ptrace::write(self.pid, self.addr.as_ptr(), restored as *mut c_void)?;
        }
        self.enabled.set(false);

        Ok(())
    }
}
