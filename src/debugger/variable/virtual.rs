use crate::debugger::Error::TypeNotFound;
use crate::debugger::debugee::Debugee;
use crate::debugger::debugee::dwarf::DebugInformation;
use crate::debugger::debugee::dwarf::unit::DieAddr;
use crate::debugger::debugee::dwarf::unit::die_ref::Variable;
use crate::debugger::{Error, debugee::dwarf::unit::die_ref::FatDieRef};
use gimli::{DebugInfoOffset, UnitOffset};

/// This DIE does not actually exist in debug information.
/// It may be used to represent variables that are
/// declared by user, for example, using pointer cast operator.
#[derive(Clone, Copy)]
pub struct VirtualVariableDie {
    pub type_ref: DieAddr,
}

impl VirtualVariableDie {
    /// Create blank virtual variable DIE.
    pub fn workpiece() -> Self {
        Self {
            type_ref: DieAddr::Unit(UnitOffset(0)),
        }
    }

    /// Initialize virtual variable with a concrete type.
    /// Return reference to virtual DIE.
    pub fn init_with_known_type<'dbg>(
        &mut self,
        debug_info: &'dbg DebugInformation,
        unit_offset: DebugInfoOffset,
        die_offset: UnitOffset,
    ) -> Result<FatDieRef<'dbg, Variable>, Error> {
        let unit = debug_info
            .find_unit(DebugInfoOffset(unit_offset.0 + die_offset.0))
            .ok_or(TypeNotFound)?;

        self.type_ref = DieAddr::Unit(die_offset);
        Ok(FatDieRef::new_virt_var(debug_info, unit.idx(), *self))
    }

    /// Initialize virtual variable with a concrete type.
    /// Return reference to virtual DIE.
    pub fn init_with_type<'dbg>(
        &mut self,
        debugee: &'dbg Debugee,
        type_name: &str,
    ) -> Result<FatDieRef<'dbg, Variable>, Error> {
        let (debug_info, offset_of_unit, offset_of_die) = debugee
            .debug_info_all()
            .iter()
            .find_map(|&debug_info| {
                let (offset_of_unit, offset_of_die) = debug_info.find_type_die_ref(type_name)?;
                Some((debug_info, offset_of_unit, offset_of_die))
            })
            .ok_or(TypeNotFound)?;

        self.init_with_known_type(debug_info, offset_of_unit, offset_of_die)
    }
}
