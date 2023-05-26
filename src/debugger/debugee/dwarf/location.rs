use crate::debugger::address::GlobalAddress;
use crate::debugger::debugee::dwarf::unit::Unit;
use crate::debugger::debugee::dwarf::{DebugeeContext, EndianArcSlice};
use crate::weak_error;
use fallible_iterator::FallibleIterator;
use gimli::{Attribute, AttributeValue, Expression};

pub(super) struct Location<'a>(pub(super) &'a Attribute<EndianArcSlice>);

impl<'a> Location<'a> {
    /// Converts location attribute to a dwarf expression.
    /// Expect location attribute one of:
    /// - DW_FORM_exprloc
    /// - DW_FORM_block*
    /// - DW_FORM_loclistx
    /// - W_FORM_sec_offset
    /// - DW_FORM_loclistx
    /// Return `None` otherwise.
    pub(super) fn try_as_expression(
        &self,
        dwarf_ctx: &DebugeeContext<EndianArcSlice>,
        unit: &Unit,
        pc: GlobalAddress,
    ) -> Option<Expression<EndianArcSlice>> {
        if let Some(expr) = self.0.exprloc_value() {
            return Some(expr);
        }

        let offset = match self.0.value() {
            AttributeValue::LocationListsRef(offset) => offset,
            AttributeValue::DebugLocListsIndex(index) => weak_error!(dwarf_ctx
                .locations()
                .get_offset(unit.encoding(), unit.loclists_base(), index))?,
            _ => return None,
        };

        let mut iter = weak_error!(dwarf_ctx.locations().locations(
            offset,
            unit.encoding(),
            unit.low_pc(),
            dwarf_ctx.debug_addr(),
            unit.addr_base(),
        ))?;

        let pc = u64::from(pc);
        let entry = iter
            .find(|list_entry| Ok(list_entry.range.begin <= pc && list_entry.range.end >= pc))
            .ok()?;

        entry.map(|e| e.data)
    }
}
