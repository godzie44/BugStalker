use crate::debugger::address::{Address, RelocatedAddress};
use crate::debugger::debugee::dwarf::unit::PlaceOwned;
use crate::debugger::debugee::Debugee;
use anyhow::anyhow;
use nix::libc::c_void;
use nix::sys;
use nix::unistd::Pid;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum BrkptType {
    /// Breakpoint to program entry point
    EntryPoint,
    /// User defined breakpoint
    UserDefined,
    /// Auxiliary breakpoints, using, for example, in step-over implementation
    Temporary,
}

static GLOBAL_BP_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Breakpoint representation.
#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub addr: RelocatedAddress,
    pub pid: Pid,
    /// Breakpoint number, > 0 for user defined breakpoints have a number, 0 for others
    number: u32,
    /// Place information, None if brkpt is temporary or entry point
    place: Option<PlaceOwned>,
    saved_data: Cell<u8>,
    enabled: Cell<bool>,
    r#type: BrkptType,
}

impl Breakpoint {
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.get()
    }
}

impl Breakpoint {
    const INT3: u64 = 0xCC_u64;

    fn new_inner(
        addr: RelocatedAddress,
        pid: Pid,
        number: u32,
        place: Option<PlaceOwned>,
        r#type: BrkptType,
    ) -> Self {
        Self {
            addr,
            number,
            pid,
            place,
            enabled: Default::default(),
            saved_data: Default::default(),
            r#type,
        }
    }

    pub fn new(addr: RelocatedAddress, pid: Pid, place: Option<PlaceOwned>) -> Self {
        Self::new_inner(
            addr,
            pid,
            GLOBAL_BP_COUNTER.fetch_add(1, Ordering::Relaxed),
            place,
            BrkptType::UserDefined,
        )
    }

    pub fn new_entry_point(addr: RelocatedAddress, pid: Pid) -> Self {
        Self::new_inner(addr, pid, 0, None, BrkptType::EntryPoint)
    }

    pub fn new_temporary(addr: RelocatedAddress, pid: Pid) -> Self {
        Self::new_inner(addr, pid, 0, None, BrkptType::Temporary)
    }

    pub fn number(&self) -> u32 {
        self.number
    }

    /// Return breakpoint place information.
    ///
    /// # Panics
    /// Panic if a breakpoint not a user defined.
    /// It is the caller responsibility to check that the type is [`BrkptType::UserDefined`].
    pub fn place(&self) -> Option<&PlaceOwned> {
        match self.r#type {
            BrkptType::UserDefined => self.place.as_ref(),
            BrkptType::EntryPoint | BrkptType::Temporary => {
                panic!("only user defined breakpoint has a place attribute")
            }
        }
    }

    pub fn is_entry_point(&self) -> bool {
        self.r#type == BrkptType::EntryPoint
    }

    pub fn r#type(&self) -> BrkptType {
        self.r#type
    }

    pub fn is_temporary(&self) -> bool {
        matches!(self.r#type, BrkptType::Temporary)
    }

    pub fn enable(&self) -> nix::Result<()> {
        let addr = self.addr.as_usize() as *mut c_void;
        let data = sys::ptrace::read(self.pid, addr)?;
        self.saved_data.set((data & 0xff) as u8);
        let data_with_pb = (data & !0xff) as u64 | Self::INT3;
        unsafe {
            sys::ptrace::write(self.pid, addr, data_with_pb as *mut c_void)?;
        }
        self.enabled.set(true);

        Ok(())
    }

    pub fn disable(&self) -> nix::Result<()> {
        let addr = self.addr.as_usize() as *mut c_void;
        let data = sys::ptrace::read(self.pid, addr)? as u64;
        let restored: u64 = (data & !0xff) | self.saved_data.get() as u64;
        unsafe {
            sys::ptrace::write(self.pid, addr, restored as *mut c_void)?;
        }
        self.enabled.set(false);

        Ok(())
    }
}

/// User defined breakpoint template,
/// may created if debugee program not running and
/// there is no and there is no way to determine the relocated address.
#[derive(Debug, Clone)]
pub struct UninitBreakpoint {
    addr: Address,
    pid: Pid,
    number: u32,
    place: Option<PlaceOwned>,
    r#type: BrkptType,
}

impl UninitBreakpoint {
    fn new_inner(
        addr: Address,
        pid: Pid,
        number: u32,
        place: Option<PlaceOwned>,
        r#type: BrkptType,
    ) -> Self {
        Self {
            addr,
            pid,
            number,
            place,
            r#type,
        }
    }

    pub fn new(addr: Address, pid: Pid, place: Option<PlaceOwned>) -> Self {
        Self::new_inner(
            addr,
            pid,
            GLOBAL_BP_COUNTER.fetch_add(1, Ordering::Relaxed),
            place,
            BrkptType::UserDefined,
        )
    }

    pub fn new_entry_point(addr: Address, pid: Pid) -> Self {
        Self::new_inner(addr, pid, 0, None, BrkptType::EntryPoint)
    }

    /// Return breakpoint create from template.
    ///
    /// # Panics
    ///
    /// Method will panic if calling with unload debugee.
    pub fn try_into_brkpt(self, debugee: &Debugee) -> anyhow::Result<Breakpoint> {
        debug_assert!(
            self.r#type == BrkptType::EntryPoint || self.r#type == BrkptType::UserDefined
        );

        let global_addr = match self.addr {
            Address::Relocated(addr) => addr.into_global(debugee.mapping_offset()),
            Address::Global(addr) => addr,
        };

        let place = if self.r#type == BrkptType::UserDefined {
            if self.place.is_some() {
                self.place
            } else {
                Some(
                    debugee
                        .dwarf()
                        .find_place_from_pc(global_addr)
                        .ok_or(anyhow!("unknown place for address: {}", self.addr))?
                        .to_owned(),
                )
            }
        } else {
            None
        };

        Ok(Breakpoint::new_inner(
            global_addr.relocate(debugee.mapping_offset()),
            self.pid,
            self.number,
            place,
            self.r#type,
        ))
    }
}

pub struct BreakpointView<'a> {
    pub addr: Address,
    pub number: u32,
    pub place: Option<Cow<'a, PlaceOwned>>,
}

impl<'a> From<Breakpoint> for BreakpointView<'a> {
    fn from(brkpt: Breakpoint) -> Self {
        Self {
            addr: Address::Relocated(brkpt.addr),
            number: brkpt.number,
            place: brkpt.place.map(Cow::Owned),
        }
    }
}

impl<'a> From<&'a Breakpoint> for BreakpointView<'a> {
    fn from(brkpt: &'a Breakpoint) -> Self {
        Self {
            addr: Address::Relocated(brkpt.addr),
            number: brkpt.number,
            place: brkpt.place.as_ref().map(Cow::Borrowed),
        }
    }
}

impl<'a> From<UninitBreakpoint> for BreakpointView<'a> {
    fn from(brkpt: UninitBreakpoint) -> Self {
        Self {
            addr: brkpt.addr,
            number: brkpt.number,
            place: brkpt.place.map(Cow::Owned),
        }
    }
}

impl<'a> From<&'a UninitBreakpoint> for BreakpointView<'a> {
    fn from(brkpt: &'a UninitBreakpoint) -> Self {
        Self {
            addr: brkpt.addr,
            number: brkpt.number,
            place: brkpt.place.as_ref().map(Cow::Borrowed),
        }
    }
}

/// Container for application breakpoints.
/// Supports active breakpoints and uninit breakpoints.
#[derive(Default)]
pub struct BreakpointRegistry {
    /// Active breakpoint list.
    breakpoints: HashMap<RelocatedAddress, Breakpoint>,
    /// Non-active breakpoint list.
    disabled_breakpoints: HashMap<Address, UninitBreakpoint>,
}

impl BreakpointRegistry {
    /// Add new breakpoint to registry and enable it.
    pub fn add_and_enable(&mut self, brkpt: Breakpoint) -> anyhow::Result<BreakpointView> {
        if let Some(existed) = self.breakpoints.get(&brkpt.addr) {
            existed.disable()?;
        }
        brkpt.enable()?;

        let addr = brkpt.addr;
        self.breakpoints.insert(addr, brkpt);
        Ok((&self.breakpoints[&addr]).into())
    }

    pub fn get_enabled(&self, addr: RelocatedAddress) -> Option<&Breakpoint> {
        self.breakpoints.get(&addr)
    }

    /// Add uninit breakpoint, this means that breakpoint will be created later.
    pub fn add_uninit(&mut self, brkpt: UninitBreakpoint) -> BreakpointView {
        let addr = brkpt.addr;
        self.disabled_breakpoints.insert(addr, brkpt);
        (&self.disabled_breakpoints[&addr]).into()
    }

    /// Remove breakpoint or uninit breakpoint from registry.
    pub fn remove_by_addr(&mut self, addr: Address) -> anyhow::Result<Option<BreakpointView>> {
        if let Some(brkpt) = self.disabled_breakpoints.remove(&addr) {
            return Ok(Some(brkpt.into()));
        }
        if let Address::Relocated(addr) = addr {
            if let Some(brkpt) = self.breakpoints.remove(&addr) {
                if brkpt.is_enabled() {
                    brkpt.disable()?;
                }
                return Ok(Some(brkpt.into()));
            }
        }
        Ok(None)
    }

    /// Enable currently disabled breakpoints.
    pub fn enable_all_breakpoints(&mut self, debugee: &Debugee) -> anyhow::Result<()> {
        let mut disabled_breakpoints = std::mem::take(&mut self.disabled_breakpoints);
        for (_, uninit_brkpt) in disabled_breakpoints.drain() {
            let brkpt = uninit_brkpt.try_into_brkpt(debugee)?;
            let number = brkpt.number();
            if let Err(e) = self.add_and_enable(brkpt) {
                log::warn!(target: "debugger", "broken breakpoint {}: {:#}", number, e);
            }
        }
        Ok(())
    }

    /// Disable currently enabled breakpoints.
    pub fn disable_all_breakpoints(&mut self, debugee: &Debugee) {
        let mut breakpoints = std::mem::take(&mut self.breakpoints);
        for (_, brkpt) in breakpoints.drain() {
            if let Err(e) = brkpt.disable() {
                log::warn!(target: "debugger", "broken breakpoint {}: {:#}", brkpt.number(), e);
            }

            let addr = Address::Global(brkpt.addr.into_global(debugee.mapping_offset()));
            match brkpt.r#type {
                BrkptType::EntryPoint => {
                    self.add_uninit(UninitBreakpoint::new_entry_point(addr, brkpt.pid));
                }
                BrkptType::UserDefined => {
                    self.add_uninit(UninitBreakpoint::new(addr, brkpt.pid, brkpt.place));
                }
                BrkptType::Temporary => {}
            }
        }
    }

    /// Update pid of all breakpoints.
    pub fn update_pid(&mut self, new_pid: Pid) {
        self.breakpoints
            .iter_mut()
            .for_each(|(_, brkpt)| brkpt.pid = new_pid);
        self.disabled_breakpoints
            .iter_mut()
            .for_each(|(_, brkpt)| brkpt.pid = new_pid);
    }

    /// Return vector of currently enabled breakpoints.
    pub fn active_breakpoints(&self) -> Vec<&Breakpoint> {
        self.breakpoints.values().collect()
    }

    /// Return view for all user defined breakpoints.
    pub fn snapshot(&self) -> Vec<BreakpointView> {
        let active_bps = self.breakpoints.values().filter_map(|bp| {
            (bp.r#type() == BrkptType::UserDefined).then(|| BreakpointView::from(bp))
        });
        let disabled_brkpts = self.disabled_breakpoints.values().filter_map(|bp| {
            (bp.r#type == BrkptType::UserDefined).then(|| BreakpointView::from(bp))
        });

        let mut snap = active_bps.chain(disabled_brkpts).collect::<Vec<_>>();
        snap.sort_by(|a, b| a.number.cmp(&b.number));

        snap
    }
}
