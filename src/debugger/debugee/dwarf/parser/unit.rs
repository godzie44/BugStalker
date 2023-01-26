use crate::debugger::debugee::dwarf::eval::ExpressionEvaluator;
use crate::debugger::debugee::dwarf::parser::DieRef;
use crate::debugger::debugee::dwarf::{EndianRcSlice, NamespaceHierarchy};
use crate::debugger::GlobalAddress;
use gimli::{Attribute, DebugInfoOffset, DwAte, DwTag, Encoding, Range, UnitOffset};
use nix::unistd::Pid;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(PartialEq, Debug)]
pub(super) struct LineRow {
    pub(super) address: u64,
    pub(super) file_index: u64,
    pub(super) line: u64,
    pub(super) column: u64,
    pub(super) is_stmt: bool,
}

#[derive(Debug)]
pub struct DieRange {
    pub range: Range,
    pub die_idx: usize,
}

#[derive(Debug)]
pub struct Unit {
    pub id: Uuid,
    pub(super) files: Vec<PathBuf>,
    pub(super) lines: Vec<LineRow>,
    pub ranges: Vec<Range>,
    pub entries: Vec<Entry>,
    pub die_ranges: Vec<DieRange>,
    #[allow(unused)]
    pub(super) name: Option<String>,
    pub(super) encoding: Encoding,
    pub offset: Option<DebugInfoOffset>,
    // index for variable die position: variable name -> [namespaces : die position in unit]
    pub variable_index: HashMap<String, Vec<(NamespaceHierarchy, usize)>>,
    // index for variables: offset in unit -> position in unit entries
    pub die_offsets_index: HashMap<UnitOffset, usize>,
}

impl Unit {
    pub fn evaluator(&self, pid: Pid) -> ExpressionEvaluator {
        ExpressionEvaluator::new(self, self.encoding, pid)
    }

    pub fn find_place(&self, line_pos: usize) -> Option<Place> {
        let line = self.lines.get(line_pos)?;
        Some(Place {
            file: self
                .files
                .get(line.file_index as usize)
                .expect("parse error"),
            address: GlobalAddress(line.address as usize),
            line_number: line.line,
            column_number: line.column,
            pos_in_unit: line_pos,
            is_stmt: line.is_stmt,
            context: self,
        })
    }

    pub fn find_place_by_pc(&self, pc: GlobalAddress) -> Option<Place> {
        let pc = pc.0 as u64;
        let pos = match self.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => p,
            Err(p) => p - 1,
        };

        self.find_place(pos)
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
    context: &'a Unit,
}

impl<'a> Place<'a> {
    pub fn next(&self) -> Option<Place<'a>> {
        self.context.find_place(self.pos_in_unit + 1)
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

#[derive(Debug)]
pub struct DieAttributes {
    pub(super) _tag: DwTag,
    pub name: Option<String>,
    pub ranges: Box<[Range]>,
}

#[derive(Debug)]
pub struct FunctionDie {
    pub base_attributes: DieAttributes,
    pub fb_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct LexicalBlockDie {
    pub base_attributes: DieAttributes,
    pub ranges: Box<[Range]>,
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
    #[allow(unused)]
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
pub enum DieVariant {
    Function(FunctionDie),
    LexicalBlock(LexicalBlockDie),
    Variable(VariableDie),
    BaseType(BaseTypeDie),
    StructType(StructTypeDie),
    TypeMember(TypeMemberDie),
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
