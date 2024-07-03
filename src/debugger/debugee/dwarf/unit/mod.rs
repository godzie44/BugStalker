mod parser;

pub use parser::DwarfUnitParser;

use crate::debugger::address::GlobalAddress;
use crate::debugger::debugee::dwarf::eval::ExpressionEvaluator;
use crate::debugger::debugee::dwarf::utils::PathSearchIndex;
use crate::debugger::debugee::dwarf::{EndianArcSlice, NamespaceHierarchy};
use crate::debugger::debugee::Debugee;
use crate::debugger::error::Error;
use crate::version::Version;
use gimli::{
    Attribute, AttributeValue, DW_LANG_Rust, DebugAddrBase, DebugInfoOffset, DebugLocListsBase,
    DwAte, DwLang, Encoding, Range, UnitHeader, UnitOffset,
};
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

const IS_STMT: u8 = 1 << 1;
const PROLOG_END: u8 = 1 << 2;
const EPILOG_BEGIN: u8 = 1 << 3;
const END_SEQUENCE: u8 = 1 << 4;

/// A row in the line number program's resulting matrix.
#[derive(PartialEq, Debug, Clone)]
#[repr(packed)]
pub(super) struct LineRow {
    pub(super) address: u64,
    pub(super) file_index: u64,
    pub(super) line: u64,
    pub(super) column: u64,
    flags: u8,
}

impl LineRow {
    #[inline(always)]
    pub fn is_stmt(&self) -> bool {
        self.flags & IS_STMT == IS_STMT
    }

    #[inline(always)]
    pub fn prolog_end(&self) -> bool {
        self.flags & PROLOG_END == PROLOG_END
    }

    #[inline(always)]
    pub fn epilog_begin(&self) -> bool {
        self.flags & EPILOG_BEGIN == EPILOG_BEGIN
    }

    #[inline(always)]
    pub fn end_sequence(&self) -> bool {
        self.flags & END_SEQUENCE == END_SEQUENCE
    }
}

/// An address range of debug information entry,
/// also contains a reference to entry itself (as index in unit entries list).
#[derive(Debug, Clone)]
pub struct DieRange {
    pub range: Range,
    pub die_idx: usize,
}

/// Represent a place in program text identified by file name
/// line number and column number.
#[derive(Clone)]
pub struct PlaceDescriptor<'a> {
    pub file: &'a Path,
    pub address: GlobalAddress,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    pub epilog_begin: bool,
    pub end_sequence: bool,
    pub prolog_end: bool,
    unit: &'a Unit,
}

/// Like a [`PlaceDescriptor`] but without reference to compilation unit.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceDescriptorOwned {
    pub file: PathBuf,
    pub address: GlobalAddress,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    pub epilog_begin: bool,
    pub prolog_end: bool,
}

impl<'a> From<(&'a Unit, usize, &LineRow)> for PlaceDescriptor<'a> {
    fn from((unit, pos_in_unit, line_row): (&'a Unit, usize, &LineRow)) -> Self {
        PlaceDescriptor {
            file: unit
                .files
                .get(line_row.file_index as usize)
                .expect("file should exists"),
            address: line_row.address.into(),
            line_number: line_row.line,
            column_number: line_row.column,
            pos_in_unit,
            is_stmt: line_row.is_stmt(),
            prolog_end: line_row.prolog_end(),
            epilog_begin: line_row.epilog_begin(),
            end_sequence: line_row.end_sequence(),
            unit,
        }
    }
}

impl<'a> Debug for PlaceDescriptor<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "file: {:?}, line: {}, addr: {}, is_stmt: {}, col: {}, epilog_begin: {}, prolog_end: {}",
            self.file, self.line_number, self.address, self.is_stmt, self.column_number, self.epilog_begin, self.prolog_end
        ))
    }
}

impl<'a> PlaceDescriptor<'a> {
    pub fn next(&self) -> Option<PlaceDescriptor<'a>> {
        self.unit.find_place_by_idx(self.pos_in_unit + 1)
    }

    pub fn prev(&self) -> Option<PlaceDescriptor<'a>> {
        self.unit.find_place_by_idx(self.pos_in_unit - 1)
    }

    pub fn line_eq(&self, other: &PlaceDescriptor) -> bool {
        self.file == other.file && self.line_number == other.line_number
    }

    pub fn to_owned(&self) -> PlaceDescriptorOwned {
        PlaceDescriptorOwned {
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

impl<'a> PartialEq for PlaceDescriptor<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.address == other.address
            && self.line_number == other.line_number
            && self.pos_in_unit == other.pos_in_unit
            && self.column_number == other.column_number
    }
}

#[derive(Debug, PartialEq, Clone, Eq, Hash)]
pub struct DieAttributes {
    pub name: Option<String>,
    pub ranges: Box<[Range]>,
}

#[derive(Debug, PartialEq, Clone, Eq)]
pub struct FunctionDie {
    pub namespace: NamespaceHierarchy,
    pub linkage_name: Option<String>,
    pub decl_file_line: Option<(u64, u64)>,
    pub base_attributes: DieAttributes,
    pub fb_addr: Option<Attribute<EndianArcSlice>>,
}

impl FunctionDie {
    /// If a subprogram die contains a `DW_AT_specification` attribute than this subprogram have
    /// a declaration part in another die.
    /// This function will complete subprogram with
    /// information from its declaration (typically this is a name and linkage_name).
    pub fn complete_from_decl(&mut self, declaration: &FunctionDie) {
        if self.linkage_name.is_none() {
            self.namespace = declaration.namespace.clone();
            self.linkage_name.clone_from(&declaration.linkage_name);
        }

        if self.base_attributes.name.is_none() {
            self.base_attributes
                .name
                .clone_from(&declaration.base_attributes.name);
        }

        if self.decl_file_line.is_none() {
            self.decl_file_line = declaration.decl_file_line;
        }
    }
}

#[derive(Debug, Clone)]
pub struct LexicalBlockDie {
    pub base_attributes: DieAttributes,
}

#[derive(Debug, Clone)]
pub struct VariableDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub location: Option<Attribute<EndianArcSlice>>,
    pub lexical_block_idx: Option<usize>,
    pub fn_block_idx: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct BaseTypeDie {
    pub base_attributes: DieAttributes,
    pub encoding: Option<DwAte>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ArrayDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ArraySubrangeDie {
    pub base_attributes: DieAttributes,
    pub lower_bound: Option<Attribute<EndianArcSlice>>,
    pub upper_bound: Option<Attribute<EndianArcSlice>>,
    pub count: Option<Attribute<EndianArcSlice>>,
}

#[derive(Debug, Clone)]
pub struct StructTypeDie {
    pub base_attributes: DieAttributes,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TypeMemberDie {
    pub base_attributes: DieAttributes,
    #[allow(unused)]
    pub byte_size: Option<u64>,
    pub location: Option<Attribute<EndianArcSlice>>,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct EnumTypeDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct EnumeratorDie {
    pub base_attributes: DieAttributes,
    pub const_value: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct VariantPart {
    pub base_attributes: DieAttributes,
    pub discr_ref: Option<DieRef>,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub base_attributes: DieAttributes,
    pub discr_value: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PointerType {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    #[allow(unused)]
    pub address_class: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct TemplateTypeParameter {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct Namespace {
    pub base_attributes: DieAttributes,
}

#[derive(Debug, Clone)]
pub struct ParameterDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub location: Option<Attribute<EndianArcSlice>>,
    pub fn_block_idx: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct UnionTypeDie {
    pub base_attributes: DieAttributes,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SubroutineDie {
    pub base_attributes: DieAttributes,
    pub return_type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct InlineSubroutineDie {
    pub base_attributes: DieAttributes,
    pub call_file: Option<u64>,
    pub call_line: Option<u64>,
    pub call_column: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TypeDefDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct ConstTypeDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct AtomicDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct VolatileDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
pub struct RestrictDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
}

#[derive(Debug, Clone)]
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
    Subroutine(SubroutineDie),
    InlineSubroutine(InlineSubroutineDie),
    TypeDef(TypeDefDie),
    ConstType(ConstTypeDie),
    Atomic(AtomicDie),
    Volatile(VolatileDie),
    Restrict(RestrictDie),
}

impl DieVariant {
    pub fn unwrap_function(&self) -> &FunctionDie {
        let DieVariant::Function(func) = self else {
            panic!("function die expected");
        };
        func
    }

    pub fn unwrap_lexical_block(&self) -> &LexicalBlockDie {
        let DieVariant::LexicalBlock(lb) = self else {
            panic!("lexical block die expected");
        };
        lb
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

impl Node {
    pub const fn new_leaf(parent: Option<usize>) -> Node {
        Self {
            parent,
            children: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub die: DieVariant,
    pub node: Node,
}

impl Entry {
    pub(super) fn new(die: DieVariant, parent_idx: Option<usize>) -> Self {
        Self {
            die,
            node: Node::new_leaf(parent_idx),
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd)]
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

#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
struct UnitLazyPart {
    entries: Vec<Entry>,
    die_ranges: Vec<DieRange>,
    /// index for variable die position: { variable name -> [namespaces: die position in unit] }
    variable_index: HashMap<String, Vec<(NamespaceHierarchy, usize)>>,
    /// index for type die position: { type name -> offset in unit }
    type_index: HashMap<String, UnitOffset>,
    /// index for variables: offset in unit -> position in unit `entries`
    die_offsets_index: HashMap<UnitOffset, usize>,
    /// index for function entries: function -> die position in unit `entries`
    function_index: PathSearchIndex<usize>,
}

/// Some of the compilation unit methods may return UnitResult
/// to show that reloading is necessary
pub enum UnitResult<T> {
    Ok(T),
    Reload,
}

/// This macro try to call a unit method, if call failed with UnitResult::Reload
/// then parsing of lazy unit part is happening
#[macro_export]
macro_rules! resolve_unit_call {
    ($dwarf: expr, $unit: expr, $fn_name: tt) => {
        resolve_unit_call!($dwarf, $unit, $fn_name,)
    };
    ($dwarf: expr, $unit: expr, $fn_name: tt, $($arg: expr),*) => {{
        use $crate::debugger::debugee::dwarf::unit::DwarfUnitParser;
        use $crate::debugger::debugee::dwarf::unit::UnitResult;
        match $unit.$fn_name( $($arg,)*) {
            UnitResult::Ok(value) => value,
            UnitResult::Reload => {
                let parser = DwarfUnitParser::new(&$dwarf);
                $unit.reload(parser).expect("unit parsing was fail unexpectedly");
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
    /// Return T if a result contains data, panic otherwise.
    pub fn ensure_ok(self) -> T {
        let UnitResult::Ok(val) = self else {
            panic!("value expected")
        };
        val
    }
}

/// DWARF compilation unit representation.
/// In BugStalker any unit load from obj file with partial data on debugee start.
/// Later, if necessary, the data will be loaded additionally.
#[derive(Debug)]
pub struct Unit {
    pub id: Uuid,
    #[allow(unused)]
    pub name: Option<String>,
    /// DWARF unit header must exist if unit is partial, but contains None if unit is fully load.
    header: Mutex<Option<UnitHeader<EndianArcSlice>>>,
    /// Index in unit registry may be usize::MAX if the unit is not yet placed in the register
    idx: usize,
    properties: UnitProperties,
    files: Vec<PathBuf>,
    /// List of program lines, ordered by its address
    lines: Vec<LineRow>,
    ranges: Vec<Range>,
    lazy_part: OnceCell<UnitLazyPart>,
    language: Option<DwLang>,
    producer: Option<String>,
}

impl Clone for Unit {
    fn clone(&self) -> Self {
        let header = self.header.lock().unwrap().clone();
        Self {
            id: self.id,
            name: self.name.clone(),
            header: Mutex::new(header),
            idx: self.idx,
            properties: self.properties.clone(),
            files: self.files.clone(),
            lines: self.lines.clone(),
            ranges: self.ranges.clone(),
            lazy_part: self.lazy_part.clone(),
            language: self.language,
            producer: self.producer.clone(),
        }
    }
}

impl Unit {
    /// Update unit to full state.
    /// Note: this method will panic if called twice.
    pub fn reload(&self, parser: DwarfUnitParser) -> Result<(), Error> {
        let additional = parser.parse_additional(
            self.header
                .lock()
                .unwrap()
                .take()
                .expect("unreachable: header must exists"),
        )?;
        self.lazy_part
            .set(additional)
            .expect("unreachable: lazy part must be empty");
        Ok(())
    }

    /// Return unit index in unit registry.
    /// See [`crate::debugger::debugee::dwarf::DebugInformation`]
    pub fn idx(&self) -> usize {
        if self.idx == usize::MAX {
            panic!("undefined index")
        }
        self.idx
    }

    /// Set index in unit registry.
    /// See [`crate::debugger::debugee::dwarf::DebugInformation`]
    pub(super) fn set_idx(&mut self, idx: usize) {
        self.idx = idx;
    }

    /// Return rust SEMVER value. If rust is not unit language or
    /// if version determine fail return `None`.
    pub fn rustc_version(&self) -> Option<Version> {
        if self.language == Some(DW_LANG_Rust) {
            if let Some(producer) = self.producer.as_ref() {
                return Version::rustc_parse(producer);
            }
        }
        None
    }

    /// Return the encoding parameters for this unit.
    pub fn encoding(&self) -> Encoding {
        self.properties.encoding
    }

    /// Return unit range lowest PC.
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

    /// Return [`PlaceDescriptor`] by index for line vector in unit.
    pub(super) fn find_place_by_idx(&self, line_pos: usize) -> Option<PlaceDescriptor> {
        let line = self.lines.get(line_pos)?;
        Some((self, line_pos, line).into())
    }

    /// Return first [`PlaceDescriptor`] matching the file index and line number.
    pub fn find_place_by_line(&self, file_idx: u64, line: u64) -> Option<PlaceDescriptor> {
        let (line_pos, line) = self
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.file_index == file_idx && l.line == line)?;
        Some((self, line_pos, line).into())
    }

    pub(super) fn line(&self, index: usize) -> &LineRow {
        &self.lines[index]
    }

    /// Return nearest [`PlaceDescriptor`] for given program counter.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter represented by global address.
    pub fn find_place_by_pc(&self, pc: GlobalAddress) -> Option<PlaceDescriptor> {
        let pc = u64::from(pc);
        let pos = self
            .lines
            .binary_search_by_key(&pc, |line| line.address)
            .unwrap_or_else(|p| p.saturating_sub(1));

        self.find_place_by_idx(pos)
    }

    /// Return the nearest line with EB (epilog begin).
    /// Nearest means - at given address or at address less than given.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter represented by global address.
    pub fn find_eb(&self, pc: GlobalAddress) -> Option<PlaceDescriptor> {
        let pc = u64::from(pc);

        let mut pos = self
            .lines
            .binary_search_by_key(&pc, |line| line.address)
            .unwrap_or_else(|p| p.saturating_sub(1));

        while pos > 0 {
            if self.lines[pos].epilog_begin() {
                return self.find_place_by_idx(pos);
            }
            pos -= 1;
        }
        None
    }

    /// Return place with address equals to given program counter.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter represented by global address.
    pub fn find_exact_place_by_pc(&self, pc: GlobalAddress) -> Option<PlaceDescriptor> {
        let pc = u64::from(pc);
        match self.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => self.find_place_by_idx(p),
            Err(_) => None,
        }
    }

    /// Return all places that correspond to given range.
    ///
    /// # Arguments
    ///
    /// * `range`: address range
    pub fn find_lines_for_range(&self, range: &Range) -> Vec<PlaceDescriptor> {
        let Some(start_place) = self.find_place_by_pc(GlobalAddress::from(range.begin)) else {
            return vec![];
        };
        let range_end_instr = range.end.saturating_sub(1);
        let Some(end_place) = self.find_place_by_pc(GlobalAddress::from(range_end_instr)) else {
            return vec![start_place];
        };

        debug_assert!(end_place.pos_in_unit >= start_place.pos_in_unit);
        let result_cap = end_place.pos_in_unit - start_place.pos_in_unit;
        let mut result = Vec::with_capacity(result_cap);

        let start_place_pos_in_unit = start_place.pos_in_unit;
        result.push(start_place);
        for pos in start_place_pos_in_unit + 1..end_place.pos_in_unit {
            result.push(self.find_place_by_idx(pos).expect("index must exists"));
        }
        result.push(end_place);

        result
    }

    /// Return list on debug entries.
    /// Note: this method requires a full unit.
    pub fn entries(&self) -> UnitResult<&Vec<Entry>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(&additional.entries),
        }
    }

    /// Return all function entries suitable for template.
    ///
    /// # Arguments
    ///
    /// * `template`: function search template, contains a function name and full or partial namespace.
    ///
    /// For example: "ns1::ns2::fn1" or "ns2::fn1"
    pub fn search_functions(&self, template: &str) -> UnitResult<Vec<&Entry>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => {
                let entry_indexes = additional.function_index.get(template);
                UnitResult::Ok(
                    entry_indexes
                        .iter()
                        .map(|&&idx| &additional.entries[idx])
                        .collect(),
                )
            }
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

    /// Return locations of all variables with name equal to `name` parameter.
    /// Note: this method requires a full unit.
    ///
    /// # Arguments
    ///
    /// * `name`: needle variable name
    pub fn locate_var_die(&self, name: &str) -> UnitResult<Option<&[(NamespaceHierarchy, usize)]>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => {
                UnitResult::Ok(additional.variable_index.get(name).map(|v| v.as_slice()))
            }
        }
    }

    /// Return locations of a type with name equal to `name` parameter.
    /// Note: this method requires a full unit.
    ///
    /// # Arguments
    ///
    /// * `name`: needle type name
    pub fn locate_type(&self, name: &str) -> UnitResult<Option<UnitOffset>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(additional.type_index.get(name).copied()),
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

    /// Return all files related to this unit.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Return pairs (file path, indexes of file lines in unit.lines list). This useful for
    /// create searching indexes.
    pub(super) fn file_path_with_lines_pairs(
        &self,
    ) -> impl Iterator<Item = (impl IntoIterator<Item = impl ToString + '_>, Vec<usize>)> {
        let mut grouped_by_file_lines = HashMap::with_capacity(self.files.len());
        for (line_idx, line) in self.lines.iter().enumerate() {
            let file_idx = line.file_index as usize;
            let entry = grouped_by_file_lines.entry(file_idx).or_insert(vec![]);
            entry.push(line_idx)
        }

        self.files
            .iter()
            .enumerate()
            .filter_map(move |(idx, file)| {
                let file_lines = grouped_by_file_lines.remove(&idx).unwrap_or_default();
                // skip files without lines
                if file_lines.is_empty() {
                    return None;
                }

                Some((file.iter().map(|s| s.to_string_lossy()), file_lines))
            })
    }
}
