mod parser;

pub use parser::DwarfUnitParser;

use crate::debugger::address::GlobalAddress;
use crate::debugger::debugee::dwarf::eval::ExpressionEvaluator;
use crate::debugger::debugee::dwarf::{EndianArcSlice, NamespaceHierarchy};
use crate::debugger::debugee::Debugee;
use gimli::{
    Attribute, AttributeValue, DebugAddrBase, DebugInfoOffset, DebugLocListsBase, DwAte, DwTag,
    Encoding, Range, UnitHeader, UnitOffset,
};
use once_cell::sync::OnceCell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// A row in the line number program's resulting matrix.
#[derive(PartialEq, Debug)]
struct LineRow {
    address: u64,
    file_index: u64,
    line: u64,
    column: u64,
    is_stmt: bool,
    prolog_end: bool,
    epilog_begin: bool,
}

/// An address range of debug information entry,
/// also contains a reference to entry itself (as index in unit entries list).
#[derive(Debug)]
pub struct DieRange {
    pub range: Range,
    pub die_idx: usize,
}

/// Represent an place in program text identified by file name
/// line number and column number.
pub struct Place<'a> {
    pub file: &'a Path,
    pub address: GlobalAddress,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    pub epilog_begin: bool,
    pub prolog_end: bool,
    unit: &'a Unit,
}

/// Like a ['Place'] but without reference to compilation unit.
pub struct PlaceOwned {
    pub file: PathBuf,
    pub address: GlobalAddress,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    pub epilog_begin: bool,
    pub prolog_end: bool,
}

impl<'a> Debug for Place<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "file: {:?}, line: {}, addr: {}, is_stmt: {}, col: {}, epilog_begin: {}, prolog_end: {}",
            self.file, self.line_number, self.address, self.is_stmt, self.column_number, self.epilog_begin, self.prolog_end
        ))
    }
}

impl<'a> Place<'a> {
    pub fn next(&self) -> Option<Place<'a>> {
        self.unit.find_place(self.pos_in_unit + 1)
    }

    pub fn line_eq(&self, other: &Place) -> bool {
        self.file == other.file && self.line_number == other.line_number
    }

    pub fn to_owned(&self) -> PlaceOwned {
        PlaceOwned {
            file: self.file.to_path_buf(),
            address: self.address,
            line_number: self.line_number,
            pos_in_unit: self.pos_in_unit,
            is_stmt: self.is_stmt,
            column_number: self.column_number,
            epilog_begin: self.epilog_begin,
            prolog_end: self.prolog_end,
        }
    }
}

impl<'a> PartialEq for Place<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.address == other.address
            && self.line_number == other.line_number
            && self.pos_in_unit == other.pos_in_unit
            && self.column_number == other.column_number
    }
}

#[derive(Debug, PartialEq)]
pub struct DieAttributes {
    pub _tag: DwTag,
    pub name: Option<String>,
    pub ranges: Box<[Range]>,
}

#[derive(Debug, PartialEq)]
pub struct FunctionDie {
    pub namespace: NamespaceHierarchy,
    pub base_attributes: DieAttributes,
    pub fb_addr: Option<Attribute<EndianArcSlice>>,
}

#[derive(Debug)]
pub struct LexicalBlockDie {
    pub base_attributes: DieAttributes,
}

#[derive(Debug)]
pub struct VariableDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub location: Option<Attribute<EndianArcSlice>>,
    pub lexical_block_idx: Option<usize>,
}

#[derive(Debug)]
pub struct BaseTypeDie {
    pub base_attributes: DieAttributes,
    #[allow(unused)]
    pub encoding: Option<DwAte>,
    pub byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct ArrayDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct ArraySubrangeDie {
    pub base_attributes: DieAttributes,
    pub lower_bound: Option<Attribute<EndianArcSlice>>,
    pub upper_bound: Option<Attribute<EndianArcSlice>>,
    pub count: Option<Attribute<EndianArcSlice>>,
}

#[derive(Debug)]
pub struct StructTypeDie {
    pub base_attributes: DieAttributes,
    pub byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct TypeMemberDie {
    pub base_attributes: DieAttributes,
    #[allow(unused)]
    pub byte_size: Option<u64>,
    pub location: Option<Attribute<EndianArcSlice>>,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug)]
pub struct EnumTypeDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct EnumeratorDie {
    pub base_attributes: DieAttributes,
    pub const_value: Option<i64>,
}

#[derive(Debug)]
pub struct VariantPart {
    pub base_attributes: DieAttributes,
    pub discr_ref: Option<DieRef>,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug)]
pub struct Variant {
    pub base_attributes: DieAttributes,
    pub discr_value: Option<i64>,
}

#[derive(Debug)]
pub struct PointerType {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    #[allow(unused)]
    pub address_class: Option<u64>,
}

#[derive(Debug)]
pub struct TemplateTypeParameter {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug)]
pub struct Namespace {
    pub base_attributes: DieAttributes,
}

#[derive(Debug)]
pub struct ParameterDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub location: Option<Attribute<EndianArcSlice>>,
}

#[derive(Debug)]
pub struct UnionTypeDie {
    pub base_attributes: DieAttributes,
    pub byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct InlineSubroutineDie {
    pub base_attributes: DieAttributes,
    pub call_file: Option<u64>,
    pub call_line: Option<u64>,
    pub call_column: Option<u64>,
}

#[derive(Debug)]
pub enum DieVariant {
    Function(FunctionDie),
    LexicalBlock(LexicalBlockDie),
    Variable(VariableDie),
    BaseType(BaseTypeDie),
    StructType(StructTypeDie),
    TypeMember(TypeMemberDie),
    UnionTypeDie(UnionTypeDie),
    ArrayType(ArrayDie),
    ArraySubrange(ArraySubrangeDie),
    Default(DieAttributes),
    EnumType(EnumTypeDie),
    Enumerator(EnumeratorDie),
    VariantPart(VariantPart),
    Variant(Variant),
    PointerType(PointerType),
    TemplateType(TemplateTypeParameter),
    Namespace(Namespace),
    Parameter(ParameterDie),
    InlineSubroutine(InlineSubroutineDie),
}

#[derive(Debug)]
pub struct Node {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

#[derive(Debug)]
pub struct Entry {
    pub die: DieVariant,
    pub node: Node,
}

impl Entry {
    pub(super) fn new(die: DieVariant, parent_idx: Option<usize>) -> Self {
        Self {
            die,
            node: Node {
                parent: parent_idx,
                children: vec![],
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum DieRef {
    Unit(UnitOffset),
    Global(DebugInfoOffset),
}

impl DieRef {
    fn from_attr(attr: Attribute<EndianArcSlice>) -> Option<DieRef> {
        match attr.value() {
            AttributeValue::DebugInfoRef(offset) => Some(DieRef::Global(offset)),
            AttributeValue::UnitRef(offset) => Some(DieRef::Unit(offset)),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct UnitProperties {
    encoding: Encoding,
    offset: Option<DebugInfoOffset>,
    low_pc: u64,
    addr_base: DebugAddrBase,
    loclists_base: DebugLocListsBase,
    address_size: u8,
}

/// This fields is a part of a compilation unit but
/// loaded on first call, for reduce memory consumption.
#[derive(Debug)]
struct UnitLazyPart {
    entries: Vec<Entry>,
    die_ranges: Vec<DieRange>,
    // index for variable die position: variable name -> [namespaces : die position in unit]
    variable_index: HashMap<String, Vec<(NamespaceHierarchy, usize)>>,
    // index for variables: offset in unit -> position in unit entries
    die_offsets_index: HashMap<UnitOffset, usize>,
}

/// Some of compilation unit methods may return UnitResult in order to show
/// that reloading is necessary
pub enum UnitResult<T> {
    Ok(T),
    Reload,
}

/// This macro try to call a unit method, if call failed with UnitResult::Reload
/// then parsing of lazy unit part is happens
#[macro_export]
macro_rules! resolve_unit_call {
    ($dwarf: expr, $unit: expr, $fn_name: tt, $($arg: expr),*) => {{
        use $crate::debugger::debugee::dwarf::unit::DwarfUnitParser;
        use $crate::debugger::debugee::dwarf::unit::UnitResult;
        match $unit.$fn_name( $($arg,)*) {
            UnitResult::Ok(value) => value,
            UnitResult::Reload => {
                let parser = DwarfUnitParser::new(&$dwarf);
                $unit.reload(parser).expect("parse unit fail unexpectedly");
                $unit.$fn_name(
                        $(
                            $arg,
                        )*
                    ).ensure_ok()
            }
        }
    }};
}

impl<T> UnitResult<T> {
    /// Return T if result contains data, panic otherwise.
    pub fn ensure_ok(self) -> T {
        let UnitResult::Ok(val) = self else {
            panic!("value expected")
        };
        val
    }
}

/// DWARF compilation unit representation.
/// In bugstalker any unit load from obj file with partial data on debugee start.
/// Later, if necessary, the data will be loaded additionally.
#[derive(Debug)]
pub struct Unit {
    pub id: Uuid,
    #[allow(unused)]
    pub name: Option<String>,
    /// DWARF unit header, must exists if unit is partial, but contains None if unit is fully load.
    header: RefCell<Option<UnitHeader<EndianArcSlice>>>,
    /// Index in unit registry, may be usize::MAX if the unit is not yet placed in the register
    idx: usize,
    properties: UnitProperties,
    files: Vec<PathBuf>,
    lines: Vec<LineRow>,
    ranges: Vec<Range>,
    lazy_part: OnceCell<UnitLazyPart>,
}

impl Unit {
    /// Update unit to full state.
    /// Note: this method will panic if called twice.
    pub fn reload(&self, parser: DwarfUnitParser) -> anyhow::Result<()> {
        let additional = parser.parse_additional(self.header.take().unwrap())?;
        self.lazy_part.set(additional).unwrap();
        Ok(())
    }

    /// Return unit index in unit registry.
    /// See [`crate::debugger::debugee::dwarf::DebugeeContext`]
    pub fn idx(&self) -> usize {
        if self.idx == usize::MAX {
            panic!("undefined index")
        }
        self.idx
    }

    /// Set index in unit registry.
    /// See [`crate::debugger::debugee::dwarf::DebugeeContext`]
    pub(super) fn set_idx(&mut self, idx: usize) {
        self.idx = idx;
    }

    /// Return the encoding parameters for this unit.
    pub fn encoding(&self) -> Encoding {
        self.properties.encoding
    }

    /// Return unit range lowest pc.
    pub fn low_pc(&self) -> u64 {
        self.properties.low_pc
    }

    /// Return beginning of the compilation unit’s contribution to the .debug_addr section.
    pub fn addr_base(&self) -> DebugAddrBase {
        self.properties.addr_base
    }

    /// Return beginning of the offsets table (immediately following the header)
    /// of the compilation unit’s contribution to the .debug_loclists section
    pub fn loclists_base(&self) -> DebugLocListsBase {
        self.properties.loclists_base
    }

    /// Return offset of this unit within its section.
    pub fn offset(&self) -> Option<DebugInfoOffset> {
        self.properties.offset
    }

    /// Return size of addresses (in bytes) in this compilation unit.
    pub fn address_size(&self) -> u8 {
        self.properties.address_size
    }

    /// Return a list of unit address ranges.
    pub fn ranges(&self) -> &Vec<Range> {
        &self.ranges
    }

    /// Return [`Place`] by index for lines vector in unit.
    fn find_place(&self, line_pos: usize) -> Option<Place> {
        let line = self.lines.get(line_pos)?;
        Some(Place {
            file: self
                .files
                .get(line.file_index as usize)
                .expect("parse error"),
            address: (line.address as usize).into(),
            line_number: line.line,
            column_number: line.column,
            pos_in_unit: line_pos,
            is_stmt: line.is_stmt,
            prolog_end: line.prolog_end,
            epilog_begin: line.epilog_begin,
            unit: self,
        })
    }

    /// Return nearest [`Place`] for given program counter.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter represented by global address.
    pub fn find_place_by_pc(&self, pc: GlobalAddress) -> Option<Place> {
        let pc = u64::from(pc);
        let pos = match self.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => p,
            Err(p) => p - 1,
        };

        self.find_place(pos)
    }

    /// Return place with address equals to given program counter.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter represented by global address.
    pub fn find_exact_place_by_pc(&self, pc: GlobalAddress) -> Option<Place> {
        let pc = u64::from(pc);
        match self.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => self.find_place(p),
            Err(_) => None,
        }
    }

    /// Return [`Place`] for given file and line.
    ///
    /// # Arguments
    ///
    /// * `file`: file name
    /// * `line`: line number
    pub fn find_stmt_line(&self, file: &str, line: u64) -> Option<Place<'_>> {
        let found = self
            .files
            .iter()
            .enumerate()
            .find(|(_, file_path)| file_path.ends_with(file))?;

        for (pos, line_row) in self.lines.iter().enumerate() {
            if line_row.line == line && line_row.file_index == found.0 as u64 {
                return self.find_place(pos);
            }
        }

        None
    }

    /// Return list on debug entries.
    /// Note: this method requires a full unit.
    pub fn entries(&self) -> UnitResult<&Vec<Entry>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(&additional.entries),
        }
    }

    /// Return iterator for debug entries.
    /// Note: this method requires a full unit.
    pub fn entries_it(&self) -> UnitResult<impl Iterator<Item = &Entry>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(additional.entries.iter()),
        }
    }

    /// Return ranges for all debug information entries in unit.
    /// Note: this method requires a full unit.
    pub fn die_ranges(&self) -> UnitResult<&Vec<DieRange>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(&additional.die_ranges),
        }
    }

    /// Return locations of all variables with name equals to `name` parameter.
    /// Note: this method requires a full unit.
    ///
    /// # Arguments
    ///
    /// * `name`: needle variable name
    pub fn locate_var_die(
        &self,
        name: &str,
    ) -> UnitResult<Option<&Vec<(NamespaceHierarchy, usize)>>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(additional.variable_index.get(name)),
        }
    }

    /// Create dwarf expression evaluator.
    /// Note: this method requires a full unit.
    pub fn evaluator<'this>(
        &'this self,
        debugee: &'this Debugee,
    ) -> UnitResult<ExpressionEvaluator> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(_) => UnitResult::Ok(ExpressionEvaluator::new(self, self.encoding(), debugee)),
        }
    }

    /// Return debug entry by its index.
    /// Note: this method requires a full unit.
    pub fn entry(&self, idx: usize) -> UnitResult<&Entry> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(&additional.entries[idx]),
        }
    }

    /// Return debug entry by its offset in unit, `None` if entry not exists.
    /// Note: this method requires a full unit.
    pub fn find_entry(&self, offset: UnitOffset) -> UnitResult<Option<&Entry>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => {
                let die_idx = additional.die_offsets_index.get(&offset);
                match die_idx {
                    None => UnitResult::Ok(None),
                    Some(die_idx) => match self.entry(*die_idx) {
                        UnitResult::Ok(entry) => UnitResult::Ok(Some(entry)),
                        UnitResult::Reload => UnitResult::Reload,
                    },
                }
            }
        }
    }
}
