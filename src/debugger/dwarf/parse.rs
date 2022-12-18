use crate::debugger::dwarf::eval::{EvalOption, ExpressionEvaluator};
use crate::debugger::dwarf::r#type::TypeDeclaration;
use crate::debugger::dwarf::{parse, EndianRcSlice};
use crate::debugger::rust::Environment;
use crate::weak_error;
use anyhow::anyhow;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use gimli::{
    Attribute, AttributeValue, DW_AT_byte_size, DW_AT_data_member_location, DW_AT_encoding,
    DW_AT_frame_base, DW_AT_location, DW_AT_name, DW_AT_type, DwAte, DwTag, Range, Reader,
    Unit as DwarfUnit, UnitOffset,
};
use nix::unistd::Pid;
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(PartialEq, Debug)]
struct LineRow {
    address: u64,
    file_index: u64,
    line: u64,
    column: u64,
    is_stmt: bool,
}

#[derive(Debug)]
pub struct DieRange {
    pub range: Range,
    pub die_idx: usize,
}

#[derive(Debug)]
pub struct Unit {
    files: Vec<PathBuf>,
    lines: Vec<LineRow>,
    pub ranges: Vec<Range>,
    pub(super) entries: Vec<Entry>,
    die_ranges: Vec<DieRange>,
    die_offsets: HashMap<UnitOffset, usize>,
    name: Option<String>,
    pub(super) expr_evaluator: Rc<ExpressionEvaluator>,
}

impl Unit {
    pub fn from_unit(
        dwarf: &gimli::Dwarf<EndianRcSlice>,
        unit: DwarfUnit<EndianRcSlice>,
    ) -> gimli::Result<Unit> {
        let name = unit
            .name
            .as_ref()
            .and_then(|n| n.to_string_lossy().ok().map(|s| s.to_string()));

        let mut parsed_unit = Unit {
            entries: vec![],
            files: vec![],
            lines: vec![],
            ranges: vec![],
            die_ranges: vec![],
            die_offsets: HashMap::new(),
            expr_evaluator: Rc::new(ExpressionEvaluator::new(unit.encoding())),
            name,
        };

        if let Some(ref lp) = unit.line_program {
            let mut rows = lp.clone().rows();
            parsed_unit.lines = parse_lines(&mut rows)?;
            parsed_unit.files = parse_files(dwarf, &unit, &rows)?;
        }
        parsed_unit.lines.sort_by_key(|x| x.address);

        parsed_unit.ranges = dwarf.unit_ranges(&unit)?.collect::<Vec<_>>()?;
        parsed_unit.ranges.sort_by_key(|r| r.begin);

        let mut cursor = unit.entries();
        while let Some((delta_depth, die)) = cursor.next_dfs()? {
            let current_idx = parsed_unit.entries.len();
            let prev_index = if parsed_unit.entries.is_empty() {
                None
            } else {
                Some(parsed_unit.entries.len() - 1)
            };

            let name = die
                .attr(DW_AT_name)?
                .and_then(|attr| dwarf.attr_string(&unit, attr.value()).ok());

            let parent_idx = match delta_depth {
                // if 1 then previous die is a parent
                1 => prev_index,
                // if 0 then previous die is a sibling
                0 => parsed_unit.entries.last().and_then(|dd| dd.node.parent),
                // if < 0 then parent of previous die is a sibling
                mut x if x < 0 => {
                    let mut parent = parsed_unit.entries.last().unwrap();
                    while x != 0 {
                        parent = &parsed_unit.entries[parent.node.parent.unwrap()];
                        x += 1;
                    }
                    parent.node.parent
                }
                _ => unreachable!(),
            };

            if let Some(parent_idx) = parent_idx {
                parsed_unit.entries[parent_idx]
                    .node
                    .children
                    .push(current_idx)
            }

            let ranges: Box<[Range]> = dwarf
                .die_ranges(&unit, die)?
                .collect::<Vec<Range>>()?
                .into();

            ranges.iter().for_each(|r| {
                parsed_unit.die_ranges.push(DieRange {
                    range: *r,
                    die_idx: current_idx,
                })
            });

            let base_attrs = DieAttributes {
                _tag: die.tag(),
                name: name
                    .map(|s| s.to_string_lossy().map(|s| s.to_string()))
                    .transpose()?,
                ranges,
            };

            let parsed_die = match die.tag() {
                gimli::DW_TAG_subprogram => DieVariant::Function(FunctionDie {
                    base_attributes: base_attrs,
                    fb_addr: die.attr(DW_AT_frame_base)?,
                }),
                gimli::DW_TAG_variable => {
                    let mut lexical_block_idx = None;
                    let mut mb_parent_idx = parent_idx;
                    while let Some(parent_idx) = mb_parent_idx {
                        if let DieVariant::LexicalBlock(_) = parsed_unit.entries[parent_idx].die {
                            lexical_block_idx = Some(parent_idx);
                            break;
                        }
                        mb_parent_idx = parsed_unit.entries[parent_idx].node.parent;
                    }

                    DieVariant::Variable(VariableDie {
                        base_attributes: base_attrs,
                        type_addr: die.attr(DW_AT_type)?,
                        location: die.attr(DW_AT_location)?,
                        lexical_block_idx,
                    })
                }
                gimli::DW_TAG_base_type => {
                    let encoding = die.attr(DW_AT_encoding)?.and_then(|attr| {
                        if let AttributeValue::Encoding(enc) = attr.value() {
                            Some(enc)
                        } else {
                            None
                        }
                    });

                    DieVariant::BaseType(BaseTypeDie {
                        base_attributes: base_attrs,
                        encoding,
                        byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    })
                }
                gimli::DW_TAG_structure_type => DieVariant::StructType(StructTypeDie {
                    base_attributes: base_attrs,
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                }),
                gimli::DW_TAG_member => DieVariant::TypeMember(TypeMemberDie {
                    base_attributes: base_attrs,
                    byte_size: die.attr(DW_AT_byte_size)?.and_then(|val| val.udata_value()),
                    location: die.attr(DW_AT_data_member_location)?,
                    type_addr: die.attr(DW_AT_type)?,
                }),
                gimli::DW_TAG_lexical_block => DieVariant::LexicalBlock(LexicalBlockDie {
                    base_attributes: base_attrs,
                    ranges: dwarf
                        .die_ranges(&unit, die)?
                        .collect::<Vec<Range>>()?
                        .into(),
                }),
                _ => DieVariant::Default(base_attrs),
            };

            parsed_unit.entries.push(Entry::new(parsed_die, parent_idx));

            parsed_unit.die_offsets.insert(die.offset(), current_idx);
        }
        parsed_unit.die_ranges.sort_by_key(|dr| dr.range.begin);

        Ok(parsed_unit)
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
            .name
            .as_ref()
            .map(|name| {
                // TODO find file substring look weird
                name.contains(file)
            })
            .unwrap_or_default();

        if !found {
            return None;
        }

        for (pos, line_row) in self.lines.iter().enumerate() {
            if line_row.line == line {
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
            if let parse::DieVariant::Function(ref func) = self.entries[dr.die_idx].die {
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

    pub(super) fn find_die(&self, offset: UnitOffset) -> Option<&Entry> {
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
    _tag: DwTag,
    pub name: Option<String>,
    pub ranges: Box<[Range]>,
}

#[derive(Debug)]
pub struct FunctionDie {
    pub base_attributes: DieAttributes,
    fb_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct LexicalBlockDie {
    pub base_attributes: DieAttributes,
    pub ranges: Box<[Range]>,
}

#[derive(Debug)]
pub struct VariableDie {
    pub base_attributes: DieAttributes,
    type_addr: Option<Attribute<EndianRcSlice>>,
    location: Option<Attribute<EndianRcSlice>>,
    lexical_block_idx: Option<usize>,
}

#[derive(Debug)]
pub struct BaseTypeDie {
    pub base_attributes: DieAttributes,
    #[allow(unused)]
    pub(super) encoding: Option<DwAte>,
    pub(super) byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct StructTypeDie {
    pub base_attributes: DieAttributes,
    #[allow(unused)]
    pub(super) byte_size: Option<u64>,
}

#[derive(Debug)]
pub struct TypeMemberDie {
    pub base_attributes: DieAttributes,
    #[allow(unused)]
    pub(super) byte_size: Option<u64>,
    pub(super) location: Option<Attribute<EndianRcSlice>>,
    pub(super) type_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub enum DieVariant {
    Function(FunctionDie),
    LexicalBlock(LexicalBlockDie),
    Variable(VariableDie),
    BaseType(BaseTypeDie),
    StructType(StructTypeDie),
    TypeMember(TypeMemberDie),
    Default(DieAttributes),
}

#[derive(Debug)]
pub struct Node {
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
}

#[derive(Debug)]
pub struct Entry {
    pub(super) die: DieVariant,
    pub(super) node: Node,
}

impl Entry {
    fn new(die: DieVariant, parent_idx: Option<usize>) -> Self {
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

impl<'a> ContextualDieRef<'a, FunctionDie> {
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
            .expr_evaluator
            .evaluate(expr, pid)?
            .into_scalar::<usize>()?;
        Ok(result)
    }

    pub fn find_variables(&self, pc: usize) -> Vec<ContextualDieRef<VariableDie>> {
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

impl<'a> ContextualDieRef<'a, VariableDie> {
    pub fn read_value_at_location(
        &self,
        parent_fn: ContextualDieRef<FunctionDie>,
        pid: Pid,
    ) -> Option<Bytes> {
        self.die.location.as_ref().and_then(|loc| {
            let expr = loc.exprloc_value()?;

            let type_decl = self.r#type()?;
            let fb = weak_error!(parent_fn.frame_base_addr(pid))?;
            let eval_result = weak_error!(self.context.expr_evaluator.evaluate_with_opts(
                expr,
                pid,
                EvalOption::new().with_base_frame(fb),
            ))?;
            let bytes =
                weak_error!(eval_result.into_raw_buffer(type_decl.size_in_bytes()? as usize))?;
            Some(bytes)
        })
    }

    pub fn r#type(&self) -> Option<TypeDeclaration> {
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

fn parse_lines<R, Offset>(
    rows: &mut gimli::LineRows<R, gimli::IncompleteLineProgram<R, Offset>, Offset>,
) -> gimli::Result<Vec<LineRow>>
where
    R: gimli::Reader<Offset = Offset>,
    Offset: gimli::ReaderOffset,
{
    let mut lines = vec![];
    while let Some((_, line_row)) = rows.next_row()? {
        let column = match line_row.column() {
            gimli::ColumnType::LeftEdge => 0,
            gimli::ColumnType::Column(x) => x.get() as u64,
        };

        lines.push(LineRow {
            address: line_row.address(),
            file_index: line_row.file_index(),
            line: line_row.line().map(NonZeroU64::get).unwrap_or(0) as u64,
            column,
            is_stmt: line_row.is_stmt(),
        })
    }
    Ok(lines)
}

fn parse_files<R, Offset>(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    rows: &gimli::LineRows<R, gimli::IncompleteLineProgram<R, Offset>, Offset>,
) -> gimli::Result<Vec<PathBuf>>
where
    R: gimli::Reader<Offset = Offset>,
    Offset: gimli::ReaderOffset,
{
    let mut files = vec![];
    let header = rows.header();
    match header.file(0) {
        Some(file) => files.push(render_file_path(unit, file, header, dwarf)?),
        None => files.push(PathBuf::from("")),
    }
    let mut index = 1;
    while let Some(file) = header.file(index) {
        files.push(render_file_path(unit, file, header, dwarf)?);
        index += 1;
    }

    Ok(files)
}

fn render_file_path<R: Reader>(
    dw_unit: &gimli::Unit<R>,
    file: &gimli::FileEntry<R, R::Offset>,
    header: &gimli::LineProgramHeader<R, R::Offset>,
    sections: &gimli::Dwarf<R>,
) -> Result<PathBuf, gimli::Error> {
    let mut path = if let Some(ref comp_dir) = dw_unit.comp_dir {
        PathBuf::from(comp_dir.to_string_lossy()?.as_ref())
    } else {
        PathBuf::new()
    };

    if file.directory_index() != 0 {
        if let Some(directory) = file.directory(header) {
            path.push(
                sections
                    .attr_string(dw_unit, directory)?
                    .to_string_lossy()?
                    .as_ref(),
            );
        }
    }

    if path.starts_with("/rustc/") {
        let rust_env = Environment::current();
        if let Some(ref std_lib_path) = rust_env.std_lib_path {
            let mut new_path = std_lib_path.clone();
            path.iter().skip(3).for_each(|part| {
                new_path.push(part);
            });
            path = new_path;
        }
    }

    path.push(
        sections
            .attr_string(dw_unit, file.path_name())?
            .to_string_lossy()?
            .as_ref(),
    );

    Ok(path)
}
