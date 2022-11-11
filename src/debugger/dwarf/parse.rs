use crate::debugger::dwarf::eval::{EvalOption, ExpressionEvaluator};
use crate::debugger::dwarf::{parse, EndianRcSlice};
use anyhow::{anyhow, bail};
use fallible_iterator::FallibleIterator;
use gimli::{
    Attribute, DW_AT_frame_base, DW_AT_high_pc, DW_AT_location, DW_AT_low_pc, DW_AT_name, DwTag,
    Encoding, Range, Reader, Unit as DwarfUnit,
};
use nix::unistd::Pid;
use std::collections::VecDeque;
use std::num::NonZeroU64;

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
    files: Vec<String>,
    lines: Vec<LineRow>,
    pub ranges: Vec<Range>,
    pub dies: Vec<DeterminedDie>,
    pub die_ranges: Vec<DieRange>,
    name: Option<String>,
    pub encoding: Encoding,
    expr_evaluator: ExpressionEvaluator,
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
            dies: vec![],
            files: vec![],
            lines: vec![],
            ranges: vec![],
            die_ranges: vec![],
            encoding: unit.encoding(),
            expr_evaluator: ExpressionEvaluator::new(unit.encoding()),
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
            let low_pc = extract_low_pc(dwarf, &unit, die)?;
            let high_pc = extract_high_pc(dwarf, &unit, die, low_pc)?;

            let current_idx = parsed_unit.dies.len();
            let prev_index = if parsed_unit.dies.is_empty() {
                None
            } else {
                Some(parsed_unit.dies.len() - 1)
            };

            let name = die
                .attr(DW_AT_name)?
                .and_then(|attr| dwarf.attr_string(&unit, attr.value()).ok());

            let parent_idx = match delta_depth {
                // if 1 then previous die is a parent
                1 => prev_index,
                // if 0 then previous die is a sibling
                0 => parsed_unit.dies.last().and_then(|dd| dd.node.parent),
                // if < 0 then parent of previous die is a sibling
                mut x if x < 0 => {
                    let mut parent = parsed_unit.dies.last().unwrap();
                    while x != 0 {
                        parent = &parsed_unit.dies[parent.node.parent.unwrap()];
                        x += 1;
                    }
                    parent.node.parent
                }
                _ => unreachable!(),
            };

            if let Some(parent_idx) = parent_idx {
                parsed_unit.dies[parent_idx].node.children.push(current_idx)
            }

            let generic_die = DieAttributes {
                _tag: die.tag(),
                name: name
                    .map(|s| s.to_string_lossy().map(|s| s.to_string()))
                    .transpose()?,
                low_pc,
                high_pc,
            };

            let parsed_die = match die.tag() {
                gimli::DW_TAG_subprogram => DieVariant::Function(FunctionDie {
                    base_attributes: generic_die,
                    fb_addr: die.attr(DW_AT_frame_base)?,
                }),
                gimli::DW_TAG_variable => DieVariant::Variable(VariableDie {
                    base_attributes: generic_die,
                    location: die.attr(DW_AT_location)?,
                }),
                _ => DieVariant::Default(generic_die),
            };

            parsed_unit
                .dies
                .push(DeterminedDie::new(parsed_die, parent_idx));

            dwarf.die_ranges(&unit, die)?.for_each(|r| {
                parsed_unit.die_ranges.push(DieRange {
                    range: r,
                    die_idx: current_idx,
                });
                Ok(())
            })?;
        }
        parsed_unit.die_ranges.sort_by_key(|dr| dr.range.begin);

        Ok(parsed_unit)
    }

    pub fn find_function_by_name(&self, name: &str) -> Option<ContextualDieRef<FunctionDie>> {
        self.dies.iter().find_map(|det_die| {
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
                .map(|s| s.as_str())
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
            if let parse::DieVariant::Function(ref func) = self.dies[dr.die_idx].die {
                if dr.range.begin <= pc && pc <= dr.range.end {
                    return Some(ContextualDieRef {
                        node: &self.dies[dr.die_idx].node,
                        context: self,
                        die: func,
                    });
                }
            };
            None
        })
    }
}

pub struct Place<'a> {
    pub file: &'a str,
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
    pub low_pc: Option<u64>,
    pub high_pc: Option<u64>,
}

#[derive(Debug)]
pub struct FunctionDie {
    pub base_attributes: DieAttributes,
    fb_addr: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub struct VariableDie {
    pub base_attributes: DieAttributes,
    location: Option<Attribute<EndianRcSlice>>,
}

#[derive(Debug)]
pub enum DieVariant {
    Function(FunctionDie),
    Variable(VariableDie),
    Default(DieAttributes),
}

#[derive(Debug)]
pub struct Node {
    parent: Option<usize>,
    children: Vec<usize>,
}

#[derive(Debug)]
pub struct DeterminedDie {
    die: DieVariant,
    node: Node,
}

impl DeterminedDie {
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

        let result = self.context.expr_evaluator.evaluate(expr, pid)?.as_u64()?;
        Ok(result as usize)
    }

    pub fn find_variables(&self) -> Vec<ContextualDieRef<VariableDie>> {
        let mut result = vec![];
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            if let DieVariant::Variable(ref var) = self.context.dies[idx].die {
                result.push(ContextualDieRef {
                    context: self.context,
                    node: &self.context.dies[idx].node,
                    die: var,
                });
            }
            self.context.dies[idx]
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
    ) -> anyhow::Result<u64> {
        if let Some(ref loc) = self.die.location {
            let expr = loc
                .exprloc_value()
                .ok_or_else(|| anyhow!("var location is not expression"))?;
            Ok(self
                .context
                .expr_evaluator
                .evaluate_with_opts(
                    expr,
                    pid,
                    EvalOption::new().with_base_frame(parent_fn.frame_base_addr(pid)?),
                )?
                .as_u64()?)
        } else {
            bail!("no location attr")
        }
    }
}

fn extract_low_pc(
    dwarf: &gimli::Dwarf<EndianRcSlice>,
    unit: &gimli::Unit<EndianRcSlice>,
    die: &gimli::DebuggingInformationEntry<EndianRcSlice>,
) -> gimli::Result<Option<u64>> {
    if let Some(attr) = die.attr(DW_AT_low_pc)? {
        return match attr.value() {
            gimli::AttributeValue::Addr(val) => Ok(Some(val)),
            gimli::AttributeValue::DebugAddrIndex(index) => Ok(Some(dwarf.address(unit, index)?)),
            _ => Ok(None),
        };
    }
    Ok(None)
}

pub fn extract_high_pc(
    dwarf: &gimli::Dwarf<EndianRcSlice>,
    unit: &gimli::Unit<EndianRcSlice>,
    die: &gimli::DebuggingInformationEntry<EndianRcSlice>,
    low_pc: Option<u64>,
) -> gimli::Result<Option<u64>> {
    if let Some(attr) = die.attr(DW_AT_high_pc)? {
        return match attr.value() {
            gimli::AttributeValue::Udata(val) => Ok(Some(low_pc.unwrap_or(0) + val)),
            gimli::AttributeValue::Addr(val) => Ok(Some(val)),
            gimli::AttributeValue::DebugAddrIndex(index) => Ok(Some(dwarf.address(unit, index)?)),
            _ => Ok(None),
        };
    }
    Ok(None)
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
) -> gimli::Result<Vec<String>>
where
    R: gimli::Reader<Offset = Offset>,
    Offset: gimli::ReaderOffset,
{
    let mut files = vec![];
    let header = rows.header();
    match header.file(0) {
        Some(file) => files.push(render_file_path(unit, file, header, dwarf)?),
        None => files.push(String::from("")),
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
) -> Result<String, gimli::Error> {
    let mut path = if let Some(ref comp_dir) = dw_unit.comp_dir {
        comp_dir.to_string_lossy()?.into_owned()
    } else {
        String::new()
    };

    if file.directory_index() != 0 {
        if let Some(directory) = file.directory(header) {
            path_push(
                &mut path,
                sections
                    .attr_string(dw_unit, directory)?
                    .to_string_lossy()?
                    .as_ref(),
            );
        }
    }

    path_push(
        &mut path,
        sections
            .attr_string(dw_unit, file.path_name())?
            .to_string_lossy()?
            .as_ref(),
    );

    Ok(path)
}

fn path_push(path: &mut String, p: &str) {
    if p.starts_with('/') {
        *path = p.to_string();
    } else {
        let dir_separator = '/';
        if !path.is_empty() && !path.ends_with(dir_separator) {
            path.push(dir_separator);
        }
        *path += p;
    }
}
