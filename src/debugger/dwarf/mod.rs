pub mod eval;
pub mod parse;
pub mod r#type;

use crate::debugger::dwarf::parse::{ContextualDieRef, FunctionDie};
use fallible_iterator::FallibleIterator;
use gimli::{Dwarf, RunTimeEndian};
use object::{Object, ObjectSection, ObjectSymbol, ObjectSymbolTable, SymbolKind};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Deref;
use std::rc::Rc;

pub type EndianRcSlice = gimli::EndianRcSlice<gimli::RunTimeEndian>;

pub struct DebugeeContext<R: gimli::Reader = EndianRcSlice> {
    _inner: Dwarf<R>,
    units: Vec<parse::Unit>,
    symbol_table: Option<SymbolTab>,
}

impl DebugeeContext {
    pub fn new<'a: 'b, 'b, OBJ: Object<'a, 'b>>(obj_file: &'a OBJ) -> anyhow::Result<Self> {
        let endian = if obj_file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        fn load_section<'a: 'b, 'b, OBJ, Endian>(
            id: gimli::SectionId,
            file: &'a OBJ,
            endian: Endian,
        ) -> anyhow::Result<gimli::EndianRcSlice<Endian>>
        where
            OBJ: object::Object<'a, 'b>,
            Endian: gimli::Endianity,
        {
            let data = file
                .section_by_name(id.name())
                .and_then(|section| section.uncompressed_data().ok())
                .unwrap_or(Cow::Borrowed(&[]));
            Ok(gimli::EndianRcSlice::new(Rc::from(&*data), endian))
        }

        let dwarf = gimli::Dwarf::load(|id| load_section(id, obj_file, endian))?;
        let symbol_table = SymbolTab::new(obj_file);

        let units = dwarf
            .units()
            .map(|header| parse::Unit::from_unit(&dwarf, dwarf.unit(header)?))
            .collect::<Vec<_>>()?;

        Ok(Self {
            units,
            symbol_table,
            _inner: dwarf,
        })
    }

    fn find_unit_by_pc(&self, pc: u64) -> Option<&parse::Unit> {
        self.units.iter().find(
            |&unit| match unit.ranges.binary_search_by_key(&pc, |r| r.begin) {
                Ok(_) => true,
                Err(pos) => {
                    let found = unit.ranges[..pos]
                        .iter()
                        .rev()
                        .any(|range| range.begin <= pc && pc <= range.end);
                    found
                }
            },
        )
    }

    pub fn find_place_from_pc(&self, pc: usize) -> Option<parse::Place> {
        let pc = pc as u64;
        let unit = self.find_unit_by_pc(pc)?;
        unit.find_place_by_pc(pc)
    }

    pub fn find_function_by_pc(&self, pc: usize) -> Option<ContextualDieRef<FunctionDie>> {
        let pc = pc as u64;
        let unit = self.find_unit_by_pc(pc)?;
        unit.find_function_by_pc(pc)
    }

    pub fn find_function_by_name(&self, fn_name: &str) -> Option<ContextualDieRef<FunctionDie>> {
        self.units
            .iter()
            .find_map(|unit| unit.find_function_by_name(fn_name))
    }

    pub fn find_stmt_line(&self, file: &str, line: u64) -> Option<parse::Place<'_>> {
        self.units
            .iter()
            .find_map(|unit| unit.find_stmt_line(file, line))
    }

    pub fn find_symbol(&self, name: &str) -> Option<&Symbol> {
        self.symbol_table.as_ref().and_then(|table| table.get(name))
    }
}

#[derive(Debug)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub addr: u64,
}

#[derive(Debug)]
struct SymbolTab(HashMap<String, Symbol>);

impl Deref for SymbolTab {
    type Target = HashMap<String, Symbol>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SymbolTab {
    pub fn new<'data: 'file, 'file, OBJ: Object<'data, 'file>>(
        object_file: &'data OBJ,
    ) -> Option<Self> {
        object_file.symbol_table().as_ref().map(|sym_table| {
            SymbolTab(
                sym_table
                    .symbols()
                    .map(|symbol| {
                        let name: String = symbol.name().unwrap_or_default().into();
                        (
                            name,
                            Symbol {
                                kind: symbol.kind(),
                                addr: symbol.address(),
                            },
                        )
                    })
                    .collect::<HashMap<_, _>>(),
            )
        })
    }
}
