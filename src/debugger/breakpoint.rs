use crate::debugger::address::{Address, RelocatedAddress};
use crate::debugger::debugee::dwarf::unit::PlaceDescriptorOwned;
use crate::debugger::debugee::dwarf::DebugInformation;
use crate::debugger::debugee::Debugee;
use crate::debugger::error::Error;
use crate::debugger::error::Error::{NoDebugInformation, NoSuitablePlace, PlaceNotFound};
use crate::debugger::{Debugger, PlaceDescriptor};
use nix::libc::c_void;
use nix::sys;
use nix::unistd::Pid;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

enum BrkptsToAddRequest {
    Init(Vec<Breakpoint>),
    Uninit(Vec<UninitBreakpoint>),
}

impl Debugger {
    /// Create and enable breakpoint at debugee address space
    ///
    /// # Arguments
    ///
    /// * `addr`: address where debugee must be stopped
    ///
    /// # Errors
    ///
    /// Return [`SetupError::PlaceNotFound`] if no place found for address,
    /// return [`BreakpointError::DebugInformation`] if errors occurs while fetching debug information.
    pub fn set_breakpoint_at_addr(
        &mut self,
        addr: RelocatedAddress,
    ) -> Result<BreakpointView, Error> {
        if self.debugee.is_in_progress() {
            let dwarf = self
                .debugee
                .debug_info(addr)
                .map_err(|_| NoDebugInformation("current place"))?;
            let global_addr = addr.into_global(&self.debugee)?;

            let place = dwarf
                .find_place_from_pc(global_addr)?
                .map(|p| p.to_owned())
                .ok_or(PlaceNotFound(global_addr))?;

            return self.breakpoints.add_and_enable(Breakpoint::new(
                dwarf.pathname(),
                addr,
                self.process.pid(),
                Some(place),
            ));
        }

        Ok(self.breakpoints.add_uninit(UninitBreakpoint::new(
            None::<PathBuf>,
            Address::Relocated(addr),
            self.process.pid(),
            None,
        )))
    }

    /// Disable and remove a breakpoint by it address.
    ///
    /// # Arguments
    ///
    /// * `addr`: breakpoint address
    pub fn remove_breakpoint(&mut self, addr: Address) -> Result<Option<BreakpointView>, Error> {
        self.breakpoints.remove_by_addr(addr)
    }

    /// Disable and remove a breakpoint by it number.
    ///
    /// # Arguments
    ///
    /// * `number`: breakpoint number
    pub fn remove_breakpoint_by_number(
        &mut self,
        number: u32,
    ) -> Result<Option<BreakpointView>, Error> {
        self.breakpoints.remove_by_num(number)
    }

    fn create_breakpoint_at_places(
        &self,
        places: Vec<(&DebugInformation, Vec<PlaceDescriptorOwned>)>,
    ) -> Result<BrkptsToAddRequest, Error> {
        let brkpts_to_add = if self.debugee.is_in_progress() {
            let mut to_add = Vec::new();
            for (dwarf, places) in places {
                for place in places {
                    let addr = place.address.relocate_to_segment(&self.debugee, dwarf)?;
                    to_add.push(Breakpoint::new(
                        dwarf.pathname(),
                        addr,
                        self.process.pid(),
                        Some(place),
                    ));
                }
            }
            BrkptsToAddRequest::Init(to_add)
        } else {
            let mut to_add = Vec::new();
            for (dwarf, places) in places {
                for place in places {
                    to_add.push(UninitBreakpoint::new(
                        Some(dwarf.pathname()),
                        Address::Global(place.address),
                        self.process.pid(),
                        Some(place),
                    ));
                }
            }
            BrkptsToAddRequest::Uninit(to_add)
        };
        Ok(brkpts_to_add)
    }

    fn add_breakpoints(
        &mut self,
        brkpts_to_add: BrkptsToAddRequest,
    ) -> Result<Vec<BreakpointView>, Error> {
        let result: Vec<_> = match brkpts_to_add {
            BrkptsToAddRequest::Init(init_brkpts) => {
                let mut result_addrs = Vec::with_capacity(init_brkpts.len());
                for brkpt in init_brkpts {
                    let addr = brkpt.addr;
                    self.breakpoints.add_and_enable(brkpt)?;
                    result_addrs.push(addr);
                }
                result_addrs
                    .iter()
                    .map(|addr| {
                        BreakpointView::from(
                            self.breakpoints
                                .get_enabled(*addr)
                                .expect("breakpoint must exists"),
                        )
                    })
                    .collect()
            }
            BrkptsToAddRequest::Uninit(uninit_brkpts) => {
                let mut result_addrs = Vec::with_capacity(uninit_brkpts.len());
                for brkpt in uninit_brkpts {
                    let addr = self.breakpoints.add_uninit(brkpt).addr;
                    result_addrs.push(addr);
                }
                result_addrs
                    .iter()
                    .map(|addr| {
                        BreakpointView::from(
                            self.breakpoints
                                .get_disabled(*addr)
                                .expect("breakpoint must exists"),
                        )
                    })
                    .collect()
            }
        };

        Ok(result)
    }

    fn addresses_for_breakpoints_at_places(
        &self,
        places: &[(&DebugInformation, Vec<PlaceDescriptorOwned>)],
    ) -> Result<impl Iterator<Item = Address>, Error> {
        let mut init_addresses_to_remove: Vec<Address> = vec![];
        if self.debugee.is_in_progress() {
            for (dwarf, places) in places.iter() {
                for place in places {
                    let addr = place.address.relocate_to_segment(&self.debugee, dwarf)?;
                    init_addresses_to_remove.push(Address::Relocated(addr));
                }
            }
        };

        let uninit_addresses_to_remove: Vec<_> = places
            .iter()
            .flat_map(|(_, places)| places.iter().map(|place| Address::Global(place.address)))
            .collect();

        Ok(init_addresses_to_remove
            .into_iter()
            .chain(uninit_addresses_to_remove))
    }

    pub fn remove_breakpoints_at_addresses(
        &mut self,
        addresses: impl Iterator<Item = Address>,
    ) -> Result<Vec<BreakpointView>, Error> {
        let mut result = vec![];
        for to_rem in addresses {
            if let Some(view) = self.breakpoints.remove_by_addr(to_rem)? {
                result.push(view)
            }
        }
        Ok(result)
    }

    fn search_functions(
        &self,
        tpl: &str,
    ) -> Result<Vec<(&DebugInformation, Vec<PlaceDescriptorOwned>)>, Error> {
        let dwarfs = self.debugee.debug_info_all();

        dwarfs
            .iter()
            .filter(|dwarf| {
                let fn_name = tpl.split("::").last().expect("at least one exists");
                dwarf.has_debug_info() && dwarf.in_pub_names(fn_name) != Some(false)
            })
            .map(|&dwarf| {
                let places = dwarf.search_places_for_fn_tpl(tpl)?;
                Ok((dwarf, places))
            })
            .collect()
    }

    /// Create and enable breakpoint at debugee address space on the following function start.
    ///
    /// # Arguments
    ///
    /// * `template`: template for searchin functions where debugee must be stopped
    ///
    /// # Errors
    ///
    /// Return [`SetupError::PlaceNotFound`] if function not found,
    /// return [`BreakpointError::DebugInformation`] if errors occurs while fetching debug information.
    pub fn set_breakpoint_at_fn(&mut self, template: &str) -> Result<Vec<BreakpointView>, Error> {
        let places = self.search_functions(template)?;
        if places.iter().all(|(_, places)| places.is_empty()) {
            return Err(NoSuitablePlace);
        }

        let brkpts = self.create_breakpoint_at_places(places)?;
        self.add_breakpoints(brkpts)
    }

    /// Disable and remove breakpoint from function start.
    ///
    /// # Arguments
    ///
    /// * `template`: template for searchin functions where breakpoints must be deleted
    pub fn remove_breakpoint_at_fn(
        &mut self,
        template: &str,
    ) -> Result<Vec<BreakpointView>, Error> {
        let places = self.search_functions(template)?;
        let addresses = self.addresses_for_breakpoints_at_places(&places)?;
        self.remove_breakpoints_at_addresses(addresses)
    }

    fn search_lines(
        &self,
        fine_tpl: &str,
        line: u64,
    ) -> Result<Vec<(&DebugInformation, Vec<PlaceDescriptorOwned>)>, Error> {
        let dwarfs = self.debugee.debug_info_all();

        dwarfs
            .iter()
            .filter(|dwarf| dwarf.has_debug_info())
            .map(|&dwarf| {
                let places: Vec<PlaceDescriptor> = dwarf.find_closest_place(fine_tpl, line)?;

                let mut places_by_file: HashMap<&Path, PlaceDescriptor> = HashMap::new();
                // return one place for each unique file
                places.into_iter().for_each(|p| {
                    // if place already exists for `p.file` - replace it if line distance is closer then existed
                    let mb_already_existed_place = places_by_file.get(p.file);
                    if let Some(exited_place) = mb_already_existed_place {
                        if exited_place.line_number - line > p.line_number - line {
                            places_by_file.insert(p.file, p);
                        }
                    } else {
                        places_by_file.insert(p.file, p);
                    }
                });

                Ok((
                    dwarf,
                    places_by_file
                        .into_values()
                        .map(|place| place.to_owned())
                        .collect(),
                ))
            })
            .collect()
    }

    /// Create and enable breakpoint at the following file and line number.
    ///
    /// # Arguments
    ///
    /// * `fine_name`: file name (ex: "main.rs")
    /// * `line`: line number
    ///
    /// # Errors
    ///
    /// Return [`SetupError::PlaceNotFound`] if line or file not exists,
    /// return [`BreakpointError::DebugInformation`] if errors occurs while fetching debug information.
    pub fn set_breakpoint_at_line(
        &mut self,
        fine_path_tpl: &str,
        line: u64,
    ) -> Result<Vec<BreakpointView>, Error> {
        let places = self.search_lines(fine_path_tpl, line)?;
        if places.iter().all(|(_, places)| places.is_empty()) {
            return Err(NoSuitablePlace);
        }

        let brkpts = self.create_breakpoint_at_places(places)?;
        self.add_breakpoints(brkpts)
    }

    /// Disable and remove breakpoint at the following file and line number.
    ///
    /// # Arguments
    ///
    /// * `fine_name`: file name (ex: "main.rs")
    /// * `line`: line number
    pub fn remove_breakpoint_at_line(
        &mut self,
        fine_name_tpl: &str,
        line: u64,
    ) -> Result<Vec<BreakpointView>, Error> {
        let places = self.search_lines(fine_name_tpl, line)?;
        let addresses = self.addresses_for_breakpoints_at_places(&places)?;
        self.remove_breakpoints_at_addresses(addresses)
    }

    /// Return list of breakpoints.
    pub fn breakpoints_snapshot(&self) -> Vec<BreakpointView> {
        self.breakpoints.snapshot()
    }

    /// Add new deferred breakpoint by address in debugee address space.
    pub fn add_deferred_at_addr(&mut self, addr: RelocatedAddress) {
        self.breakpoints
            .deferred_breakpoints
            .push(DeferredBreakpoint::at_address(addr));
    }

    /// Add new deferred breakpoint by function name.
    pub fn add_deferred_at_function(&mut self, function: &str) {
        self.breakpoints
            .deferred_breakpoints
            .push(DeferredBreakpoint::at_function(function));
    }

    /// Add new deferred breakpoint by file and line.
    pub fn add_deferred_at_line(&mut self, file: &str, line: u64) {
        self.breakpoints
            .deferred_breakpoints
            .push(DeferredBreakpoint::at_line(file, line));
    }

    /// Refresh deferred breakpoints. Trying to set breakpoint, if success - remove
    /// breakpoint from deferred list.
    pub fn refresh_deferred(&mut self) -> Vec<Error> {
        let mut errors = vec![];

        let mut deferred_brkpts = mem::take(&mut self.breakpoints.deferred_breakpoints);
        deferred_brkpts.retain(|brkpt| {
            let mb_error = match &brkpt {
                DeferredBreakpoint::Address(addr) => self.set_breakpoint_at_addr(*addr).err(),
                DeferredBreakpoint::Line(file, line) => {
                    self.set_breakpoint_at_line(file, *line).err()
                }
                DeferredBreakpoint::Function(function) => self.set_breakpoint_at_fn(function).err(),
            };

            match mb_error {
                None => false,
                Some(NoSuitablePlace) => true,
                Some(err) => {
                    errors.push(err);
                    true
                }
            }
        });
        self.breakpoints.deferred_breakpoints = deferred_brkpts;

        errors
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum BrkptType {
    /// Breakpoint to program entry point
    EntryPoint,
    /// User defined breakpoint
    UserDefined,
    /// Auxiliary breakpoints, using, for example, in step-over implementation
    Temporary,
    /// Breakpoint at linker internal function that will always be called when the linker
    /// begins to map in a library or unmap it, and again when the mapping change is complete.
    LinkerMapFn,
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
    place: Option<PlaceDescriptorOwned>,
    saved_data: Cell<u8>,
    enabled: Cell<bool>,
    r#type: BrkptType,
    debug_info_file: PathBuf,
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
        place: Option<PlaceDescriptorOwned>,
        r#type: BrkptType,
        debug_info_file: PathBuf,
    ) -> Self {
        Self {
            addr,
            number,
            pid,
            place,
            enabled: Default::default(),
            saved_data: Default::default(),
            r#type,
            debug_info_file,
        }
    }

    pub fn new(
        debug_info_file: impl Into<PathBuf>,
        addr: RelocatedAddress,
        pid: Pid,
        place: Option<PlaceDescriptorOwned>,
    ) -> Self {
        Self::new_inner(
            addr,
            pid,
            GLOBAL_BP_COUNTER.fetch_add(1, Ordering::Relaxed),
            place,
            BrkptType::UserDefined,
            debug_info_file.into(),
        )
    }

    pub fn new_entry_point(
        debug_info_file: impl Into<PathBuf>,
        addr: RelocatedAddress,
        pid: Pid,
    ) -> Self {
        Self::new_inner(
            addr,
            pid,
            0,
            None,
            BrkptType::EntryPoint,
            debug_info_file.into(),
        )
    }

    pub fn new_temporary(
        debug_info_file: impl Into<PathBuf>,
        addr: RelocatedAddress,
        pid: Pid,
    ) -> Self {
        Self::new_inner(
            addr,
            pid,
            0,
            None,
            BrkptType::Temporary,
            debug_info_file.into(),
        )
    }

    pub fn new_linker_map(addr: RelocatedAddress, pid: Pid) -> Self {
        Self::new_inner(
            addr,
            pid,
            0,
            None,
            BrkptType::LinkerMapFn,
            PathBuf::default(),
        )
    }

    pub fn number(&self) -> u32 {
        self.number
    }

    /// Return breakpoint place information.
    ///
    /// # Panics
    /// Panic if a breakpoint not a user defined.
    /// It is the caller responsibility to check that the type is [`BrkptType::UserDefined`].
    pub fn place(&self) -> Option<&PlaceDescriptorOwned> {
        match self.r#type {
            BrkptType::UserDefined => self.place.as_ref(),
            BrkptType::EntryPoint | BrkptType::Temporary | BrkptType::LinkerMapFn => {
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

    pub fn enable(&self) -> Result<(), Error> {
        let addr = self.addr.as_usize() as *mut c_void;
        let data = sys::ptrace::read(self.pid, addr).map_err(Error::Ptrace)?;
        self.saved_data.set((data & 0xff) as u8);
        let data_with_pb = (data & !0xff) as u64 | Self::INT3;
        unsafe {
            sys::ptrace::write(self.pid, addr, data_with_pb as *mut c_void)
                .map_err(Error::Ptrace)?;
        }
        self.enabled.set(true);

        Ok(())
    }

    pub fn disable(&self) -> Result<(), Error> {
        let addr = self.addr.as_usize() as *mut c_void;
        let data = sys::ptrace::read(self.pid, addr).map_err(Error::Ptrace)? as u64;
        let restored: u64 = (data & !0xff) | self.saved_data.get() as u64;
        unsafe {
            sys::ptrace::write(self.pid, addr, restored as *mut c_void).map_err(Error::Ptrace)?;
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
    place: Option<PlaceDescriptorOwned>,
    r#type: BrkptType,
    debug_info_file: Option<PathBuf>,
}

impl UninitBreakpoint {
    fn new_inner(
        addr: Address,
        pid: Pid,
        number: u32,
        place: Option<PlaceDescriptorOwned>,
        r#type: BrkptType,
        debug_info_file: Option<PathBuf>,
    ) -> Self {
        Self {
            addr,
            pid,
            number,
            place,
            r#type,
            debug_info_file,
        }
    }

    pub fn new(
        debug_info_file: Option<impl Into<PathBuf>>,
        addr: Address,
        pid: Pid,
        place: Option<PlaceDescriptorOwned>,
    ) -> Self {
        Self::new_inner(
            addr,
            pid,
            GLOBAL_BP_COUNTER.fetch_add(1, Ordering::Relaxed),
            place,
            BrkptType::UserDefined,
            debug_info_file.map(|path| path.into()),
        )
    }

    pub fn new_entry_point(
        debug_info_file: Option<impl Into<PathBuf>>,
        addr: Address,
        pid: Pid,
    ) -> Self {
        Self::new_inner(
            addr,
            pid,
            0,
            None,
            BrkptType::EntryPoint,
            debug_info_file.map(|path| path.into()),
        )
    }

    /// Return a breakpoint created from template.
    ///
    /// # Panics
    ///
    /// Method will panic if calling with unloaded debugee.
    pub fn try_into_brkpt(self, debugee: &Debugee) -> Result<Breakpoint, Error> {
        debug_assert!(
            self.r#type == BrkptType::EntryPoint || self.r#type == BrkptType::UserDefined
        );

        let (global_addr, rel_addr) = match self.addr {
            Address::Relocated(addr) => (addr.into_global(debugee)?, Some(addr)),
            Address::Global(addr) => (addr, None),
        };

        let dwarf = match self.debug_info_file {
            None if self.r#type == BrkptType::EntryPoint => debugee.program_debug_info().ok(),
            None => rel_addr.and_then(|addr| debugee.debug_info(addr).ok()),
            Some(path) => debugee.debug_info_from_file(&path).ok(),
        }
        .ok_or(NoDebugInformation("breakpoint"))?;

        let place = if self.r#type == BrkptType::UserDefined {
            if self.place.is_some() {
                self.place
            } else {
                Some(
                    dwarf
                        .find_place_from_pc(global_addr)?
                        .ok_or(PlaceNotFound(global_addr))?
                        .to_owned(),
                )
            }
        } else {
            None
        };

        Ok(Breakpoint::new_inner(
            global_addr.relocate_to_segment(debugee, dwarf)?,
            self.pid,
            self.number,
            place,
            self.r#type,
            dwarf.pathname().into(),
        ))
    }
}

pub struct BreakpointView<'a> {
    pub addr: Address,
    pub number: u32,
    pub place: Option<Cow<'a, PlaceDescriptorOwned>>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct BreakpointViewOwned {
    pub addr: Address,
    pub number: u32,
    pub place: Option<PlaceDescriptorOwned>,
}

impl<'a> BreakpointView<'a> {
    pub fn to_owned(&self) -> BreakpointViewOwned {
        BreakpointViewOwned {
            addr: self.addr,
            number: self.number,
            place: self.place.clone().map(|p| p.into_owned()),
        }
    }
}

/// User breakpoint deferred until shared library with target place will be loaded.
pub enum DeferredBreakpoint {
    Address(RelocatedAddress),
    Line(String, u64),
    Function(String),
}

impl DeferredBreakpoint {
    pub fn at_address(addr: RelocatedAddress) -> DeferredBreakpoint {
        DeferredBreakpoint::Address(addr)
    }

    pub fn at_line(file: &str, line: u64) -> DeferredBreakpoint {
        DeferredBreakpoint::Line(file.to_string(), line)
    }

    pub fn at_function(function: &str) -> DeferredBreakpoint {
        DeferredBreakpoint::Function(function.to_string())
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
    /// List of deferred breakpoints, refresh all time when shared library loading.
    deferred_breakpoints: Vec<DeferredBreakpoint>,
}

impl BreakpointRegistry {
    /// Add new breakpoint to registry and enable it.
    pub fn add_and_enable(&mut self, brkpt: Breakpoint) -> Result<BreakpointView, Error> {
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

    pub fn get_disabled(&self, addr: Address) -> Option<&UninitBreakpoint> {
        self.disabled_breakpoints.get(&addr)
    }

    /// Add uninit breakpoint, this means that breakpoint will be created later.
    pub fn add_uninit(&mut self, brkpt: UninitBreakpoint) -> BreakpointView {
        let addr = brkpt.addr;
        self.disabled_breakpoints.insert(addr, brkpt);
        (&self.disabled_breakpoints[&addr]).into()
    }

    /// Remove breakpoint or uninit breakpoint from registry.
    pub fn remove_by_addr(
        &mut self,
        addr: Address,
    ) -> Result<Option<BreakpointView<'static>>, Error> {
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

    /// Remove enabled breakpoint from registry by it number.
    pub fn remove_by_num(&mut self, number: u32) -> Result<Option<BreakpointView<'static>>, Error> {
        if let Some(addr) = self.disabled_breakpoints.iter().find_map(|(addr, brkpt)| {
            if brkpt.number == number {
                return Some(addr);
            }
            None
        }) {
            return self.remove_by_addr(*addr);
        }

        if let Some(addr) = self.breakpoints.iter().find_map(|(addr, brkpt)| {
            if brkpt.number == number {
                return Some(addr);
            }
            None
        }) {
            return self.remove_by_addr(Address::Relocated(*addr));
        }

        Ok(None)
    }

    /// Enable currently disabled breakpoints.
    pub fn enable_all_breakpoints(&mut self, debugee: &Debugee) -> Vec<Error> {
        let mut errors = vec![];
        let mut disabled_breakpoints = mem::take(&mut self.disabled_breakpoints);
        for (_, uninit_brkpt) in disabled_breakpoints.drain() {
            let brkpt = match uninit_brkpt.try_into_brkpt(debugee) {
                Ok(b) => b,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };

            if let Err(e) = self.add_and_enable(brkpt) {
                errors.push(e);
            }
        }
        errors
    }

    /// Enable entry point breakpoint if it disabled.
    pub fn enable_entry_breakpoint(&mut self, debugee: &Debugee) -> Result<(), Error> {
        let Some((&key, _)) = self
            .disabled_breakpoints
            .iter()
            .find(|(_, brkpt)| brkpt.r#type == BrkptType::EntryPoint)
        else {
            return Ok(());
        };

        let uninit_entry_point_brkpt = self.disabled_breakpoints.remove(&key).unwrap();

        let brkpt = uninit_entry_point_brkpt.try_into_brkpt(debugee)?;
        self.add_and_enable(brkpt)?;

        Ok(())
    }

    /// Disable currently enabled breakpoints.
    pub fn disable_all_breakpoints(&mut self, debugee: &Debugee) -> Result<Vec<Error>, Error> {
        let mut errors = vec![];
        let mut breakpoints = std::mem::take(&mut self.breakpoints);
        for (_, brkpt) in breakpoints.drain() {
            if let Err(e) = brkpt.disable() {
                errors.push(e);
            }

            let addr = Address::Global(brkpt.addr.into_global(debugee)?);
            match brkpt.r#type {
                BrkptType::EntryPoint => {
                    self.add_uninit(UninitBreakpoint::new_entry_point(
                        Some(brkpt.debug_info_file),
                        addr,
                        brkpt.pid,
                    ));
                }
                BrkptType::UserDefined => {
                    self.add_uninit(UninitBreakpoint::new(
                        Some(brkpt.debug_info_file),
                        addr,
                        brkpt.pid,
                        brkpt.place,
                    ));
                }
                BrkptType::Temporary | BrkptType::LinkerMapFn => {}
            }
        }
        Ok(errors)
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
        let active_bps = self
            .breakpoints
            .values()
            .filter(|&bp| bp.r#type() == BrkptType::UserDefined)
            .map(BreakpointView::from);
        let disabled_brkpts = self
            .disabled_breakpoints
            .values()
            .filter(|&bp| bp.r#type == BrkptType::UserDefined)
            .map(BreakpointView::from);

        let mut snap = active_bps.chain(disabled_brkpts).collect::<Vec<_>>();
        snap.sort_by(|a, b| a.number.cmp(&b.number));

        snap
    }
}
