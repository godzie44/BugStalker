pub mod die;
pub mod die_ref;
mod parser;

use indexmap::IndexMap;
pub use parser::DwarfUnitParser;

use crate::debugger::address::GlobalAddress;
use crate::debugger::debugee::Debugee;
use crate::debugger::debugee::dwarf::eval::ExpressionEvaluator;
use crate::debugger::debugee::dwarf::utils::PathSearchIndex;
use crate::debugger::debugee::dwarf::{EndianArcSlice, NamespaceHierarchy};
use crate::debugger::error::Error;
use crate::version::RustVersion;
use gimli::{
    Attribute, AttributeValue, DW_LANG_Rust, DebugAddrBase, DebugInfoOffset, DebugLocListsBase,
    DwLang, Dwarf, Encoding, Range, UnitOffset,
};
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const IS_STMT: u8 = 1 << 1;
const PROLOG_END: u8 = 1 << 2;
const EPILOG_BEGIN: u8 = 1 << 3;
const END_SEQUENCE: u8 = 1 << 4;

/// A row in the line number program's resulting matrix.
#[derive(PartialEq, Debug, Clone)]
#[repr(Rust, packed)]
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
    pub die_off: UnitOffset,
}

/// Represent a place in program text identified by file name
/// line number and column number.
#[derive(Clone)]
pub struct PlaceDescriptor<'a> {
    pub file: &'a Path,
    pub file_idx: u64,
    pub address: GlobalAddress,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    pub epilog_begin: bool,
    pub end_sequence: bool,
    pub prolog_end: bool,
    unit: &'a BsUnit,
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

impl<'a> From<(&'a BsUnit, usize, &LineRow)> for PlaceDescriptor<'a> {
    fn from((unit, pos_in_unit, line_row): (&'a BsUnit, usize, &LineRow)) -> Self {
        PlaceDescriptor {
            file: unit
                .files
                .get(line_row.file_index as usize)
                .expect("file should exists"),
            file_idx: line_row.file_index,
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

impl Debug for PlaceDescriptor<'_> {
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

impl PartialEq for PlaceDescriptor<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.address == other.address
            && self.line_number == other.line_number
            && self.pos_in_unit == other.pos_in_unit
            && self.column_number == other.column_number
    }
}

#[derive(Debug, PartialEq, Clone, Eq)]
pub struct FunctionInfo {
    pub namespace: NamespaceHierarchy,
    pub linkage_name: Option<String>,
    pub decl_file_line: Option<(u64, u64)>,
    pub name: Option<String>,
}

impl FunctionInfo {
    /// If a subprogram die contains a `DW_AT_specification` attribute than this subprogram have
    /// a declaration part in another die.
    /// This function will complete subprogram with
    /// information from its declaration (typically this is a name and linkage_name).
    pub fn complete_from_decl(&mut self, declaration: &FunctionInfo) {
        if self.linkage_name.is_none() {
            self.namespace = declaration.namespace.clone();
            self.linkage_name.clone_from(&declaration.linkage_name);
        }

        if self.name.is_none() {
            self.name.clone_from(&declaration.name);
        }

        if self.decl_file_line.is_none() {
            self.decl_file_line = declaration.decl_file_line;
        }
    }

    pub fn full_name(&self) -> Option<String> {
        self.name
            .as_ref()
            .map(|name| format!("{}::{}", self.namespace.0.join("::"), name))
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd)]
pub enum DieAddr {
    Unit(UnitOffset),
    Global(DebugInfoOffset),
}

impl DieAddr {
    fn from_attr(attr: Attribute<EndianArcSlice>) -> Option<DieAddr> {
        match attr.value() {
            AttributeValue::DebugInfoRef(offset) => Some(DieAddr::Global(offset)),
            AttributeValue::UnitRef(offset) => Some(DieAddr::Unit(offset)),
            _ => None,
        }
    }

    pub fn unit_offset(&self, unit: &BsUnit) -> UnitOffset {
        match self {
            DieAddr::Unit(unit_offset) => *unit_offset,
            DieAddr::Global(debug_info_offset) => {
                UnitOffset(debug_info_offset.0 - unit.offset().unwrap_or(DebugInfoOffset(0)).0)
            }
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
    /// ranges for each function
    fn_ranges: Vec<DieRange>,
    /// index for variable die position: { variable name -> [namespaces: die position in unit] }
    variable_index: HashMap<String, Vec<(NamespaceHierarchy, UnitOffset)>>,
    /// index for type die position: { type name -> offset in unit }
    type_index: HashMap<String, UnitOffset>,
    /// all found functions
    function_index: HashMap<UnitOffset, FunctionInfo>,
    /// index for function entries: function -> die offset in unit
    function_name_index: PathSearchIndex<UnitOffset>,
    /// {die ; die parent} pairs
    parent_index: IndexMap<UnitOffset, UnitOffset>,
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
pub struct BsUnit {
    pub id: Uuid,
    #[allow(unused)]
    pub name: Option<String>,
    /// Index in unit registry may be usize::MAX if the unit is not yet placed in the register
    idx: usize,
    unit: gimli::Unit<EndianArcSlice>,
    properties: UnitProperties,
    files: Vec<PathBuf>,
    /// List of program lines, ordered by its address
    lines: Vec<LineRow>,
    ranges: Vec<Range>,
    lazy_part: OnceCell<UnitLazyPart>,
    language: Option<DwLang>,
    producer: Option<String>,
}

impl BsUnit {
    pub fn clone(&self, dwarf: &Dwarf<EndianArcSlice>) -> Self {
        let unit = {
            dwarf
                .unit(self.unit.header.clone())
                .expect("clone unit should not fail")
        };
        Self {
            id: self.id,
            name: self.name.clone(),
            idx: self.idx,
            properties: self.properties.clone(),
            files: self.files.clone(),
            lines: self.lines.clone(),
            ranges: self.ranges.clone(),
            lazy_part: self.lazy_part.clone(),
            language: self.language,
            producer: self.producer.clone(),
            unit,
        }
    }

    #[inline(always)]
    pub fn unit(&self) -> &gimli::Unit<EndianArcSlice> {
        &self.unit
    }

    /// Update unit to full state.
    /// Note: this method will panic if called twice.
    pub fn reload(&self, parser: DwarfUnitParser) -> Result<(), Error> {
        let additional = parser.parse_additional(self)?;
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
    pub fn rustc_version(&self) -> Option<RustVersion> {
        if self.language == Some(DW_LANG_Rust)
            && let Some(producer) = self.producer.as_ref()
        {
            return RustVersion::parse(producer);
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
    pub(super) fn find_place_by_idx(&self, line_pos: usize) -> Option<PlaceDescriptor<'_>> {
        let line = self.lines.get(line_pos)?;
        Some((self, line_pos, line).into())
    }

    /// Return first [`PlaceDescriptor`] matching the file index and line number.
    pub fn find_place_by_line(&self, file_idx: u64, line: u64) -> Option<PlaceDescriptor<'_>> {
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
    pub fn find_place_by_pc(&self, pc: GlobalAddress) -> Option<PlaceDescriptor<'_>> {
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
    pub fn find_eb(&self, pc: GlobalAddress) -> Option<PlaceDescriptor<'_>> {
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
    pub fn find_exact_place_by_pc(&self, pc: GlobalAddress) -> Option<PlaceDescriptor<'_>> {
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
    pub fn find_lines_for_range(&self, range: &Range) -> Vec<PlaceDescriptor<'_>> {
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

    /// Return parent index.
    /// Note: this method requires a full unit.
    pub fn parent_index(&self) -> UnitResult<&IndexMap<UnitOffset, UnitOffset>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(&additional.parent_index),
        }
    }

    /// Return iterator over pairs (type_name, offset).
    pub fn type_iter(&self) -> UnitResult<impl Iterator<Item = (&String, &UnitOffset)>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(additional.type_index.iter()),
        }
    }

    /// Return all function entries suitable for template.
    /// Note: this method requires a full unit.
    ///
    /// # Arguments
    ///
    /// * `template`: function search template, contains a function name and full or partial namespace.
    ///
    /// For example: "ns1::ns2::fn1" or "ns2::fn1"
    pub fn search_functions(&self, template: &str) -> UnitResult<Vec<(UnitOffset, &FunctionInfo)>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => {
                let functions = additional.function_name_index.get(template);

                UnitResult::Ok(
                    functions
                        .into_iter()
                        .map(|off| (*off, &additional.function_index[off]))
                        .collect(),
                )
            }
        }
    }

    /// Return ranges for fn information entries in unit.
    /// Note: this method requires a full unit.
    pub fn fn_ranges(&self) -> UnitResult<&Vec<DieRange>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(&additional.fn_ranges),
        }
    }

    /// Return locations of all variables with name equal to `name` parameter.
    /// Note: this method requires a full unit.
    ///
    /// # Arguments
    ///
    /// * `name`: needle variable name
    pub fn locate_var_die(
        &self,
        name: &str,
    ) -> UnitResult<Option<&[(NamespaceHierarchy, UnitOffset)]>> {
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
        dwarf: &'this Dwarf<EndianArcSlice>,
    ) -> UnitResult<ExpressionEvaluator<'this>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(_) => UnitResult::Ok(ExpressionEvaluator::new(
                dwarf,
                self,
                self.encoding(),
                debugee,
            )),
        }
    }

    /// Return function information by its offset in unit.
    /// Note: this method requires a full unit.
    pub fn fn_info(&self, off: UnitOffset) -> UnitResult<Option<&FunctionInfo>> {
        match self.lazy_part.get() {
            None => UnitResult::Reload,
            Some(additional) => UnitResult::Ok(additional.function_index.get(&off)),
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
