use crate::debugger::Error;
use crate::debugger::Error::TypeNotFound;
use crate::debugger::debugee::Debugee;
use crate::debugger::debugee::dwarf::unit::{DieRef, Node};
use crate::debugger::debugee::dwarf::{
    AsAllocatedData, ContextualDieRef, DebugInformation, EndianArcSlice,
};
use gimli::{Attribute, DebugInfoOffset, UnitOffset};

/// This DIE does not actually exist in debug information.
/// It may be used to represent variables that are
/// declared by user, for example, using pointer cast operator.
#[derive(Clone, Copy)]
pub struct VirtualVariableDie {
    type_ref: DieRef,
}

impl VirtualVariableDie {
    pub(super) const ANY_NODE: &'static Node = &Node::new_leaf(None);

    /// Create blank virtual variable DIE.
    pub fn workpiece() -> Self {
        Self {
            type_ref: DieRef::Unit(UnitOffset(0)),
        }
    }

    /// Initialize virtual variable with a concrete type.
    /// Return reference to virtual DIE.
    pub fn init_with_known_type<'this, 'dbg>(
        &'this mut self,
        debug_info: &'dbg DebugInformation,
        unit_offset: DebugInfoOffset,
        die_offset: UnitOffset,
    ) -> Result<ContextualDieRef<'this, 'dbg, Self>, Error> {
        let unit = debug_info
            .find_unit(DebugInfoOffset(unit_offset.0 + die_offset.0))
            .ok_or(TypeNotFound)?;

        self.type_ref = DieRef::Unit(die_offset);
        Ok(ContextualDieRef {
            debug_info,
            unit_idx: unit.idx(),
            node: VirtualVariableDie::ANY_NODE,
            die: self,
        })
    }

    /// Initialize virtual variable with a concrete type.
    /// Return reference to virtual DIE.
    pub fn init_with_type<'this, 'dbg>(
        &'this mut self,
        debugee: &'dbg Debugee,
        type_name: &str,
    ) -> Result<ContextualDieRef<'this, 'dbg, Self>, Error> {
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

impl AsAllocatedData for VirtualVariableDie {
    fn name(&self) -> Option<&str> {
        None
    }

    fn type_ref(&self) -> Option<DieRef> {
        Some(self.type_ref)
    }

    fn location(&self) -> Option<&Attribute<EndianArcSlice>> {
        None
    }
}
