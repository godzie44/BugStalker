#![allow(dead_code)]

use crate::debugger::address::RelocatedAddress;
use nix::libc;
use nix::unistd::Pid;
use object::elf::DT_DEBUG;
use std::collections::HashMap;

#[derive(Debug)]
pub struct LinkMap {
    pub addr: RelocatedAddress,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RendezvousError {
    #[error(".dynamic section not found")]
    DynamicSectNotFound,
    #[error("read from remote process: {0}")]
    PtraceRead(#[from] nix::Error),
    #[error("rendezvous not found")]
    NotFound,
}

/// Rendezvous structure maintained by dynamic linker.
/// This structure maintains a list of shared library descriptors.
pub struct Rendezvous {
    pid: Pid,
    inner: ffi::r_debug,
}

impl Rendezvous {
    pub fn new(
        proc_pid: Pid,
        mapping_offset: usize,
        sections: &HashMap<String, u64>,
    ) -> Result<Self, RendezvousError> {
        let dyn_sect_addr = sections
            .get(".dynamic")
            .cloned()
            .ok_or(RendezvousError::DynamicSectNotFound)? as usize;

        let dyn_sect_addr = dyn_sect_addr + mapping_offset;
        let mut addr = dyn_sect_addr;

        let mut val = ffi::read_val::<usize>(proc_pid, &mut addr)?;

        while val != 0 {
            if val == DT_DEBUG as usize {
                let mut rend_addr = ffi::read_val::<usize>(proc_pid, &mut addr)?;
                let rendezvous = ffi::read_val::<ffi::r_debug>(proc_pid, &mut rend_addr)?;
                return Ok(Self {
                    pid: proc_pid,
                    inner: rendezvous,
                });
            }

            val = ffi::read_val::<usize>(proc_pid, &mut addr)?;
        }

        Err(RendezvousError::NotFound)
    }

    pub fn link_map_main(&self) -> RelocatedAddress {
        RelocatedAddress::from(self.inner.link_map as usize)
    }

    pub fn link_maps(&self) -> Result<Vec<LinkMap>, RendezvousError> {
        let mut result = vec![];
        let mut next_link_map_addr = usize::from(self.link_map_main()) as *const libc::c_void;

        while !next_link_map_addr.is_null() {
            let lm = ffi::read_val::<ffi::link_map>(self.pid, &mut (next_link_map_addr as usize))?;
            let name = ffi::read_string(self.pid, lm.l_name as usize)?;

            result.push(LinkMap {
                addr: RelocatedAddress::from(next_link_map_addr as usize),
                name,
            });

            next_link_map_addr = lm.l_next;
        }

        Ok(result)
    }

    /// Return an address of a function internal to the run-time linker,
    /// that will always be called when the linker begins to map in a
    /// library or unmap it, and again when the mapping change is complete.
    pub fn r_brk(&self) -> RelocatedAddress {
        RelocatedAddress::from(self.inner.r_brk)
    }
}

mod ffi {
    #![allow(non_camel_case_types)]

    use nix::libc;
    use nix::sys::uio;
    use nix::sys::uio::RemoteIoVec;
    use nix::unistd::Pid;
    use std::io::IoSliceMut;
    use std::mem;
    use std::str::from_utf8;

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub(super) struct r_debug {
        /// Version number for this protocol.
        pub(super) r_version: i32,
        /// Head of the chain of loaded objects.
        pub(super) link_map: *const libc::c_void,
        /// This is the address of a function internal to the run-time linker,
        /// that will always be called when the linker begins to map in a
        /// library or unmap it, and again when the mapping change is complete.
        /// The debugger can set a breakpoint at this address if it wants to
        /// notice shared object mapping changes.
        pub(super) r_brk: usize,
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub(super) struct link_map {
        /// Difference between the address in the ELF file and the address in memory
        pub(super) l_addr: *mut libc::c_void,
        /// Absolute pathname where object was found
        pub(super) l_name: *const libc::c_char,
        /// Dynamic section of the shared object
        pub(super) l_ld: *mut libc::c_void,

        pub(super) l_next: *mut libc::c_void,
        pub(super) l_prev: *mut libc::c_void,
    }

    pub(super) fn read_val<T: Copy>(pid: Pid, addr: &mut usize) -> nix::Result<T> {
        let size = mem::size_of::<T>();
        let mut buff = vec![0; size];
        let local_iov = IoSliceMut::new(buff.as_mut_slice());
        let remote_iov = RemoteIoVec {
            base: *addr,
            len: size,
        };
        let local_iov_slice = &mut [local_iov];

        let _reads = uio::process_vm_readv(pid, local_iov_slice.as_mut_slice(), &[remote_iov])?;

        let ptr = local_iov_slice[0].as_ptr();

        let val_ptr: *const T = ptr.cast::<T>();
        let val = unsafe { *val_ptr };

        *addr += size;

        Ok(val)
    }

    pub(super) fn read_string(pid: Pid, mut addr: usize) -> nix::Result<String> {
        let mut buff = vec![];
        let mut word = read_val::<usize>(pid, &mut addr)?;

        loop {
            for b in word.to_ne_bytes() {
                if b as char == '\0' {
                    return Ok(from_utf8(&buff).unwrap().to_string());
                }
                buff.push(b);
            }
            word = read_val::<usize>(pid, &mut addr)?;
        }
    }
}
