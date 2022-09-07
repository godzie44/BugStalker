use nix::libc::{c_void, uintptr_t};
use nix::sys;
use nix::unistd::Pid;
use std::cell::Cell;

pub struct Breakpoint {
    addr: uintptr_t,
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
    pub fn new(addr: uintptr_t, pid: Pid) -> Self {
        Self {
            addr,
            pid,
            enabled: Default::default(),
            saved_data: Default::default(),
        }
    }

    pub fn enable(&self) -> nix::Result<()> {
        let data = sys::ptrace::read(self.pid, self.addr as *mut c_void)?;
        self.saved_data.set((data & 0xff) as u8);
        let int3 = 0xCC as u64;
        let data_with_pb = (data & !0xff) as u64 | int3;
        unsafe {
            sys::ptrace::write(
                self.pid,
                self.addr as *mut c_void,
                data_with_pb as *mut c_void,
            )?;
        }
        self.enabled.set(true);

        Ok(())
    }

    pub fn disable(&self) -> nix::Result<()> {
        let data = sys::ptrace::read(self.pid, self.addr as *mut c_void)? as u64;
        let restored: u64 = (data & !0xff) | self.saved_data.get() as u64;
        unsafe {
            sys::ptrace::write(self.pid, self.addr as *mut c_void, restored as *mut c_void)?;
        }
        self.enabled.set(false);

        Ok(())
    }
}
