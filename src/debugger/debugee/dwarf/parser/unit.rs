use crate::debugger::address::GlobalAddress;
use crate::debugger::debugee::dwarf::eval::ExpressionEvaluator;
use crate::debugger::debugee::dwarf::parser::DieRef;
use crate::debugger::debugee::dwarf::{EndianRcSlice, NamespaceHierarchy};
use crate::debugger::debugee::Debugee;
use gimli::{
    Attribute, DebugAddrBase, DebugInfoOffset, DebugLocListsBase, DwAte, DwTag, Encoding, Range,
    UnitOffset,
};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(PartialEq, Debug)]
pub(super) struct LineRow {
    pub(super) address: u64,
    pub(super) file_index: u64,
    pub(super) line: u64,
    pub(super) column: u64,
    pub(super) is_stmt: bool,
    pub(super) prolog_end: bool,
    pub(super) epilog_begin: bool,
}

#[derive(Debug)]
pub struct DieRange {
    pub range: Range,
    pub die_idx: usize,
}

#[derive(Debug)]
pub(super) struct UnitProperties {
    pub(super) encoding: Encoding,
    pub(super) offset: Option<DebugInfoOffset>,
    pub(super) low_pc: u64,
    pub(super) addr_base: DebugAddrBase,
    pub(super) loclists_base: DebugLocListsBase,
    pub(super) address_size: u8,
}

#[derive(Debug)]
pub struct Unit {
    pub id: Uuid,
    pub(super) properties: UnitProperties,
    pub(super) files: Vec<PathBuf>,
    pub(super) lines: Vec<LineRow>,
    pub ranges: Vec<Range>,
    pub entries: Vec<Entry>,
    pub die_ranges: Vec<DieRange>,
    #[allow(unused)]
    pub(super) name: Option<String>,
    // index for variable die position: variable name -> [namespaces : die position in unit]
    pub variable_index: HashMap<String, Vec<(NamespaceHierarchy, usize)>>,
    // index for variables: offset in unit -> position in unit entries
    pub die_offsets_index: HashMap<UnitOffset, usize>,
}

impl Unit {
    pub fn encoding(&self) -> Encoding {
        self.properties.encoding
    }

    pub fn low_pc(&self) -> u64 {
        self.properties.low_pc
    }

    pub fn addr_base(&self) -> DebugAddrBase {
        self.properties.addr_base
    }

    pub fn loclists_base(&self) -> DebugLocListsBase {
        self.properties.loclists_base
    }

    pub fn offset(&self) -> Option<DebugInfoOffset> {
        self.properties.offset
    }

    pub fn address_size(&self) -> u8 {
        self.properties.address_size
    }

    pub fn evaluator<'this>(&'this self, debugee: &'this Debugee) -> ExpressionEvaluator {
        ExpressionEvaluator::new(self, self.encoding(), debugee)
    }

    pub fn find_place(&self, line_pos: usize) -> Option<Place> {
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
            context: self,
        })
    }

    pub fn find_place_by_pc(&self, pc: GlobalAddress) -> Option<Place> {
        let pc = u64::from(pc);
        let pos = match self.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => p,
            Err(p) => p - 1,
        };

        self.find_place(pos)
    }

    pub fn find_exact_place_by_pc(&self, pc: GlobalAddress) -> Option<Place> {
        let pc = u64::from(pc);
        match self.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => self.find_place(p),
            Err(_) => None,
        }
    }

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

    pub fn find_entry(&self, offset: UnitOffset) -> Option<&Entry> {
        let die_idx = self.die_offsets_index.get(&offset)?;
        Some(&self.entries[*die_idx])
    }
}

pub struct Place<'a> {
    pub file: &'a Path,
    pub address: GlobalAddress,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    pub epilog_begin: bool,
    pub prolog_end: bool,
    context: &'a Unit,
}

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
        self.context.find_place(self.pos_in_unit + 1)
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
    pub(super) _tag: DwTag,
    pub name: Option<String>,
    pub ranges: Box<[Range]>,
}

#[derive(Debug, PartialEq)]
pub struct FunctionDie {
    pub namespace: NamespaceHierarchy,
    pub base_attributes: DieAttributes,
    pub fb_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct LexicalBlockDie {
    pub base_attributes: DieAttributes,
}

#[derive(Debug)]
pub struct VariableDie {
    pub base_attributes: DieAttributes,
    pub type_ref: Option<DieRef>,
    pub location: Option<Attribute<EndianRcSlice>>,
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
    pub lower_bound: Option<Attribute<EndianRcSlice>>,
    pub upper_bound: Option<Attribute<EndianRcSlice>>,
    pub count: Option<Attribute<EndianRcSlice>>,
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
    pub location: Option<Attribute<EndianRcSlice>>,
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
    pub location: Option<Attribute<EndianRcSlice>>,
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
