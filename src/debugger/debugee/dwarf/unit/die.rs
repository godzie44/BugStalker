use fallible_iterator::FallibleIterator;
use gimli::{
    DW_AT_name, DebuggingInformationEntry, DwAt, Dwarf, Range, Reader, Unit, UnitHeader, UnitOffset,
};

use crate::debugger::debugee::dwarf::{
    ContextualDieRef, EndianArcSlice,
    unit::{DieAttributes, DieRange},
};

pub struct Die<'a> {
    dwarf: &'a Dwarf<EndianArcSlice>,
    unit: Unit<EndianArcSlice>,
    offset: UnitOffset,
}

impl<'a> Die<'a> {
    pub fn new(
        dwarf: &'a Dwarf<EndianArcSlice>,
        header: UnitHeader<EndianArcSlice>,
        offset: UnitOffset,
    ) -> Self {
        // TODO construct unit in parse
        let unit = dwarf.unit(header).unwrap();

        Self {
            dwarf,
            unit,
            offset,
        }
    }

    pub fn from_ref<T>(reference: &'a ContextualDieRef<'_, '_, T>) -> Self {
        let header = reference.debug_info.header(reference.unit_idx);
        Self::new(reference.debug_info.dwarf(), header, reference.die_off)
    }

    pub fn base_attr(&self) -> DieAttributes {
        let die = self.unit.entry(self.offset).unwrap();
        let name = self.attr_to_string(&self.unit, &die, DW_AT_name).unwrap();

        let ranges: Box<[Range]> = self
            .dwarf
            .die_ranges(&self.unit, &die)
            .unwrap()
            .collect::<Vec<Range>>()
            .unwrap()
            .into();

        let base_attrs = DieAttributes { name, ranges };
        base_attrs
    }

    fn attr_to_string(
        &self,
        unit: &gimli::Unit<EndianArcSlice, usize>,
        die: &DebuggingInformationEntry<EndianArcSlice, usize>,
        attr: DwAt,
    ) -> gimli::Result<Option<String>> {
        die.attr(attr)?
            .and_then(|attr| self.dwarf.attr_string(unit, attr.value()).ok())
            .map(|l| l.to_string_lossy().map(|s| s.to_string()))
            .transpose()
    }
}
