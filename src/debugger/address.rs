use gimli::Range;
use std::fmt::{Display, Formatter};

/// Represent address in running program.
/// Relocated address is a `GlobalAddress` + user VAS segment offset.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug, Default)]
pub struct RelocatedAddress(usize);

impl RelocatedAddress {
    pub fn into_global(self, offset: usize) -> GlobalAddress {
        GlobalAddress(self.0 - offset)
    }

    pub fn offset(self, offset: isize) -> RelocatedAddress {
        if offset >= 0 {
            self.0 + offset as usize
        } else {
            self.0 - offset.unsigned_abs()
        }
        .into()
    }

    pub fn as_u64(self) -> u64 {
        u64::from(self)
    }

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
        f.write_str(&format!("{:#016X}", self.0))
    }
}

/// Represent address in object files.
/// This address unique per object file but not per process.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug, Default)]
pub struct GlobalAddress(usize);

impl GlobalAddress {
    pub fn relocate(self, offset: usize) -> RelocatedAddress {
        RelocatedAddress(self.0 + offset)
    }

    pub fn in_range(self, range: &Range) -> bool {
        u64::from(self) >= range.begin && u64::from(self) < range.end
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
        f.write_str(&format!("{:#016X}", self.0))
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
