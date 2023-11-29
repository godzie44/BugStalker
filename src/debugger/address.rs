use crate::debugger::debugee::dwarf::DebugInformation;
use crate::debugger::debugee::Debugee;
use crate::debugger::error::Error;
use gimli::Range;
use std::fmt::{Display, Formatter};

/// Represent address in running program.
/// Relocated address is a `GlobalAddress` + user VAS segment offset.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug, Default, PartialOrd, Ord)]
pub struct RelocatedAddress(usize);

impl RelocatedAddress {
    #[inline(always)]
    pub fn remove_vas_region_offset(self, offset: usize) -> GlobalAddress {
        GlobalAddress(self.0 - offset)
    }

    pub fn into_global(self, debugee: &Debugee) -> Result<GlobalAddress, Error> {
        Ok(self.remove_vas_region_offset(debugee.mapping_offset_for_pc(self)?))
    }

    #[inline(always)]
    pub fn offset(self, offset: isize) -> RelocatedAddress {
        if offset >= 0 {
            self.0 + offset as usize
        } else {
            self.0 - offset.unsigned_abs()
        }
        .into()
    }

    #[inline(always)]
    pub fn as_u64(self) -> u64 {
        u64::from(self)
    }

    #[inline(always)]
    pub fn as_usize(self) -> usize {
        usize::from(self)
    }
}

impl From<usize> for RelocatedAddress {
    fn from(addr: usize) -> Self {
        RelocatedAddress(addr)
    }
}

impl From<u64> for RelocatedAddress {
    fn from(addr: u64) -> Self {
        RelocatedAddress(addr as usize)
    }
}

impl From<RelocatedAddress> for usize {
    fn from(addr: RelocatedAddress) -> Self {
        addr.0
    }
}

impl From<RelocatedAddress> for u64 {
    fn from(addr: RelocatedAddress) -> Self {
        addr.0 as u64
    }
}

impl Display for RelocatedAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:#016X}", self.0))
    }
}

/// Represent address in object files.
/// This address unique per object file but not per process.
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Debug, Default)]
pub struct GlobalAddress(usize);

impl GlobalAddress {
    fn relocate(self, offset: usize) -> RelocatedAddress {
        RelocatedAddress(self.0 + offset)
    }

    /// Relocate address to VAS segment determined by debug information.
    ///
    /// # Errors
    ///
    /// Return error if no VAS offset for debug information.
    pub fn relocate_to_segment(
        self,
        debugee: &Debugee,
        segment: &DebugInformation,
    ) -> Result<RelocatedAddress, Error> {
        let offset = debugee.mapping_offset_for_file(segment)?;
        Ok(self.relocate(offset))
    }

    /// Relocate address to VAS segment determined by another address.
    ///
    /// # Errors
    ///
    /// Return error if no VAS offset for address.
    pub fn relocate_to_segment_by_pc(
        self,
        debugee: &Debugee,
        pc: RelocatedAddress,
    ) -> Result<RelocatedAddress, Error> {
        let offset = debugee.mapping_offset_for_pc(pc)?;
        Ok(self.relocate(offset))
    }

    pub fn in_range(self, range: &Range) -> bool {
        u64::from(self) >= range.begin && u64::from(self) < range.end
    }

    pub fn in_ranges(self, ranges: &[Range]) -> bool {
        ranges.iter().any(|range| self.in_range(range))
    }
}

impl From<usize> for GlobalAddress {
    fn from(addr: usize) -> Self {
        GlobalAddress(addr)
    }
}

impl From<u64> for GlobalAddress {
    fn from(addr: u64) -> Self {
        GlobalAddress(addr as usize)
    }
}

impl Display for GlobalAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:#016X}", self.0))
    }
}

impl From<GlobalAddress> for usize {
    fn from(addr: GlobalAddress) -> Self {
        addr.0
    }
}

impl From<GlobalAddress> for u64 {
    fn from(addr: GlobalAddress) -> Self {
        addr.0 as u64
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub enum Address {
    Relocated(RelocatedAddress),
    Global(GlobalAddress),
}

impl Display for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::Relocated(addr) => addr.fmt(f),
            Address::Global(addr) => addr.fmt(f),
        }
    }
}
