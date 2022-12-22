use crate::debugger::dwarf::eval::{EvalOption, ExpressionEvaluator};
use crate::debugger::dwarf::r#type::TypeDeclaration;
use crate::debugger::dwarf::EndianRcSlice;
use crate::weak_error;
use anyhow::anyhow;
use bytes::Bytes;
use gimli::{Attribute, DwAte, DwTag, Encoding, Range, UnitOffset};
use nix::unistd::Pid;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

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
    pub(super) files: Vec<PathBuf>,
    pub(super) lines: Vec<LineRow>,
    pub ranges: Vec<Range>,
    pub entries: Vec<Entry>,
    pub(super) die_ranges: Vec<DieRange>,
    pub(super) die_offsets: HashMap<UnitOffset, usize>,
    #[allow(unused)]
    pub(super) name: Option<String>,
    pub(super) encoding: Encoding,
}

impl Unit {
    pub fn evaluator(&self) -> ExpressionEvaluator {
        ExpressionEvaluator::new(self.encoding)
    }

    pub fn find_function_by_name(&self, name: &str) -> Option<ContextualDieRef<FunctionDie>> {
        self.entries.iter().find_map(|det_die| {
            if let DieVariant::Function(func) = &det_die.die {
                if func
                    .base_attributes
                    .name
                    .as_ref()
                    .map(|fn_name| fn_name == name)
                    .unwrap_or(false)
                {
                    return Some(ContextualDieRef {
                        context: self,
                        node: &det_die.node,
                        die: func,
                    });
                }
            }
            None
        })
    }

    pub fn find_place(&self, line_pos: usize) -> Option<Place> {
        let line = self.lines.get(line_pos)?;
        Some(Place {
            file: self
                .files
                .get(line.file_index as usize)
                .expect("parse error"),
            address: line.address,
            line_number: line.line,
            column_number: line.column,
            pos_in_unit: line_pos,
            is_stmt: line.is_stmt,
            context: self,
        })
    }

    pub fn find_place_by_pc(&self, pc: u64) -> Option<Place> {
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

    pub fn find_function_by_pc(&self, pc: u64) -> Option<ContextualDieRef<FunctionDie>> {
        let find_pos = match self
            .die_ranges
            .binary_search_by_key(&pc, |dr| dr.range.begin)
        {
            Ok(pos) => pos + 1,
            Err(pos) => pos,
        };

        self.die_ranges[..find_pos].iter().rev().find_map(|dr| {
            if let DieVariant::Function(ref func) = self.entries[dr.die_idx].die {
                if dr.range.begin <= pc && pc <= dr.range.end {
                    return Some(ContextualDieRef {
                        node: &self.entries[dr.die_idx].node,
                        context: self,
                        die: func,
                    });
                }
            };
            None
        })
    }

    pub fn find_die(&self, offset: UnitOffset) -> Option<&Entry> {
        let die_idx = self.die_offsets.get(&offset)?;
        Some(&self.entries[*die_idx])
    }
}

pub struct Place<'a> {
    pub file: &'a Path,
    pub address: u64,
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
    pub(super) fb_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct LexicalBlockDie {
    pub base_attributes: DieAttributes,
    pub ranges: Box<[Range]>,
}

#[derive(Debug)]
pub struct VariableDie {
    pub base_attributes: DieAttributes,
    pub(super) type_addr: Option<Attribute<EndianRcSlice>>,
    pub(super) location: Option<Attribute<EndianRcSlice>>,
    pub(super) lexical_block_idx: Option<usize>,
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
    pub type_addr: Option<Attribute<EndianRcSlice>>,
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
    pub type_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct EnumTypeDie {
    pub base_attributes: DieAttributes,
    pub type_addr: Option<Attribute<EndianRcSlice>>,
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
    pub discr_addr: Option<Attribute<EndianRcSlice>>,
    pub type_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct Variant {
    pub base_attributes: DieAttributes,
    pub discr_value: Option<i64>,
}

#[derive(Debug)]
pub struct PointerType {
    pub base_attributes: DieAttributes,
    pub type_addr: Option<Attribute<EndianRcSlice>>,
    #[allow(unused)]
    pub address_class: Option<u64>,
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

pub struct ContextualDieRef<'a, T> {
    pub context: &'a Unit,
    pub node: &'a Node,
    pub die: &'a T,
}

impl<'a, T> Clone for ContextualDieRef<'a, T> {
    fn clone(&self) -> Self {
        Self {
            context: self.context,
            node: self.node,
            die: self.die,
        }
    }
}

impl<'a, T> Copy for ContextualDieRef<'a, T> {}

impl<'unit> ContextualDieRef<'unit, FunctionDie> {
    pub fn frame_base_addr(&self, pid: Pid) -> anyhow::Result<usize> {
        let attr = self
            .die
            .fb_addr
            .as_ref()
            .ok_or_else(|| anyhow!("no frame base attr"))?;
        let expr = attr
            .exprloc_value()
            .ok_or_else(|| anyhow!("frame base attribute not an expression"))?;

        let result = self
            .context
            .evaluator()
            .evaluate(expr, pid)?
            .into_scalar::<usize>()?;
        Ok(result)
    }

    pub fn find_variables<'this>(
        &'this self,
        pc: usize,
    ) -> Vec<ContextualDieRef<'unit, VariableDie>> {
        let mut result = vec![];
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            if let DieVariant::Variable(ref var) = self.context.entries[idx].die {
                let var_ref = ContextualDieRef {
                    context: self.context,
                    node: &self.context.entries[idx].node,
                    die: var,
                };

                if var_ref.valid_at(pc) {
                    result.push(var_ref);
                }
            }
            self.context.entries[idx]
                .node
                .children
                .iter()
                .for_each(|i| queue.push_back(*i));
        }
        result
    }
}

impl<'unit> ContextualDieRef<'unit, VariableDie> {
    pub fn read_value_at_location(
        &self,
        type_decl: &TypeDeclaration,
        parent_fn: ContextualDieRef<FunctionDie>,
        pid: Pid,
    ) -> Option<Bytes> {
        self.die.location.as_ref().and_then(|loc| {
            let expr = loc.exprloc_value()?;
            let fb = weak_error!(parent_fn.frame_base_addr(pid))?;
            let eval_result = weak_error!(self.context.evaluator().evaluate_with_opts(
                expr,
                pid,
                EvalOption::new().with_base_frame(fb),
            ))?;
            let bytes =
                weak_error!(eval_result.into_raw_buffer(type_decl.size_in_bytes(pid)? as usize))?;
            Some(bytes)
        })
    }

    pub fn r#type<'this>(&'this self) -> Option<TypeDeclaration<'unit>> {
        self.die.type_addr.as_ref().and_then(|addr| {
            if let gimli::AttributeValue::UnitRef(unit_offset) = addr.value() {
                let entry = &self.context.find_die(unit_offset)?;
                let type_decl = match entry.die {
                    DieVariant::BaseType(ref type_die) => TypeDeclaration::from(ContextualDieRef {
                        context: self.context,
                        node: &entry.node,
                        die: type_die,
                    }),
                    DieVariant::StructType(ref type_die) => {
                        TypeDeclaration::from(ContextualDieRef {
                            context: self.context,
                            node: &entry.node,
                            die: type_die,
                        })
                    }
                    DieVariant::ArrayType(ref type_die) => {
                        TypeDeclaration::from(ContextualDieRef {
                            context: self.context,
                            node: &entry.node,
                            die: type_die,
                        })
                    }
                    DieVariant::EnumType(ref type_die) => TypeDeclaration::from(ContextualDieRef {
                        context: self.context,
                        node: &entry.node,
                        die: type_die,
                    }),
                    DieVariant::PointerType(ref type_die) => {
                        TypeDeclaration::from(ContextualDieRef {
                            context: self.context,
                            node: &entry.node,
                            die: type_die,
                        })
                    }
                    _ => None?,
                };
                Some(type_decl)
            } else {
                None
            }
        })
    }

    pub fn valid_at(&self, pc: usize) -> bool {
        self.die
            .lexical_block_idx
            .map(|lb_idx| {
                let DieVariant::LexicalBlock(lb) = &self.context.entries[lb_idx].die else {
                    unreachable!();
                };

                lb.ranges
                    .iter()
                    .any(|r| pc >= r.begin as usize && pc <= r.end as usize)
            })
            .unwrap_or(true)
    }
}
