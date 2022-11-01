pub mod eval;

use crate::debugger::dwarf::eval::ExpressionEvaluator;
use anyhow::anyhow;
use fallible_iterator::FallibleIterator;
use gimli::Location::Address;
use gimli::{
    Attribute, DW_AT_frame_base, DW_AT_high_pc, DW_AT_location, DW_AT_low_pc, DW_AT_name,
    DW_TAG_subprogram, DW_TAG_variable, DwTag, Dwarf, EvaluationResult, Range, Reader,
    RunTimeEndian, Unit,
};
use itertools::Itertools;
use nix::libc::uintptr_t;
use nix::sys;
use nix::unistd::Pid;
use object::{Object, ObjectSection, ObjectSymbol, ObjectSymbolTable, SymbolKind};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::rc::Rc;

pub type EndianRcSlice = gimli::EndianRcSlice<gimli::RunTimeEndian>;

pub struct DwarfContext<R: gimli::Reader = EndianRcSlice> {
    _inner: Dwarf<R>,
    unit_ranges: Vec<ParsedUnit<R>>,
    symbol_table: Option<SymbolTab>,
}

pub struct Place<'a, R: gimli::Reader = EndianRcSlice> {
    pub file: &'a str,
    pub address: u64,
    pub line_number: u64,
    pub pos_in_unit: usize,
    pub is_stmt: bool,
    pub column_number: u64,
    unit: &'a ParsedUnit<R>,
}

impl<'a> Place<'a> {
    pub fn next(&self) -> Option<Place<'a>> {
        self.unit.get_place(self.pos_in_unit + 1)
    }
}

impl<'a, R: gimli::Reader> PartialEq for Place<'a, R> {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.address == other.address
            && self.line_number == other.line_number
            && self.pos_in_unit == other.pos_in_unit
            && self.column_number == other.column_number
    }
}

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
    range: Range,
    die_idx: usize,
}

#[derive(Debug)]
pub struct Die {
    tag: DwTag,
    idx: usize,
    unit_idx: usize,
    pub name: Option<String>,
    pub low_pc: Option<u64>,
    pub high_pc: Option<u64>,
    location: Option<Attribute<EndianRcSlice>>,
    fb_addr: Option<Attribute<EndianRcSlice>>,
}

impl Die {
    pub fn read_value_at_location(
        &self,
        pid: Pid,
        current_fn: &Die,
        dwarf: &DwarfContext,
    ) -> anyhow::Result<Vec<u64>> {
        let mut result = vec![];
        if let Some(ref loc) = self.location {
            let expr = loc.exprloc_value().unwrap();

            let parsed_unit = dwarf.unit_ranges.get(self.unit_idx).unwrap();
            let mut eval = expr.evaluation(parsed_unit._unit.encoding());
            let mut evaluate_res = eval.evaluate()?;
            while evaluate_res != EvaluationResult::Complete {
                match evaluate_res {
                    EvaluationResult::RequiresMemory {
                        address,
                        size,
                        space,
                        base_type,
                    } => {
                        println!("req mem {address} {size} ");
                        break;
                    }
                    EvaluationResult::RequiresRegister {
                        register,
                        base_type,
                    } => {
                        println!("req mem {:?} ", register);
                        break;
                    }
                    EvaluationResult::RequiresFrameBase => {
                        let evaluator = ExpressionEvaluator::new(pid, dwarf);

                        let fba = current_fn.frame_base_addr(dwarf, &evaluator)?;
                        evaluate_res = eval.resume_with_frame_base(fba as u64)?;
                    }
                    _ => {
                        break;
                    }
                };
            }

            let pieces = eval.as_result();
            result = pieces
                .iter()
                .map(|piece| match piece.location {
                    Address { address } => {
                        sys::ptrace::read(pid, address as *mut c_void).map(|v| v as u64)
                    }
                    _ => todo!(),
                })
                .collect::<nix::Result<Vec<u64>>>()?;
        }

        Ok(result)
    }

    pub fn frame_base_addr(
        &self,
        dwarf: &DwarfContext,
        evaluator: &eval::ExpressionEvaluator,
    ) -> anyhow::Result<usize> {
        let attr = self
            .fb_addr
            .as_ref()
            .ok_or_else(|| anyhow!("no frame base attr"))?;
        let expr = attr
            .exprloc_value()
            .ok_or_else(|| anyhow!("frame base attribute not an expression"))?;

        let parsed_unit = dwarf.unit_ranges.get(self.unit_idx).unwrap();
        let result = evaluator.evaluate(parsed_unit, expr)?.as_u64()?;
        Ok(result as usize)
    }
}

#[derive(Debug)]
pub struct ParsedUnit<R: gimli::Reader = EndianRcSlice> {
    files: Vec<String>,
    ranges: Vec<Range>,
    lines: Vec<LineRow>,
    dies: Vec<Die>,
    local_vars: HashMap<usize, Vec<usize>>,
    die_ranges: Vec<DieRange>,
    _unit: Rc<Unit<R>>,
}

impl ParsedUnit {
    pub fn get_place(&self, line_pos: usize) -> Option<Place<EndianRcSlice>> {
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
            unit: self,
        })
    }
}

impl DwarfContext {
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

        Ok(Self {
            unit_ranges: Self::parse(&dwarf)?,
            symbol_table,
            _inner: dwarf,
        })
    }

    fn find_unit(&self, pc: u64) -> Option<&ParsedUnit<EndianRcSlice>> {
        self.unit_ranges.iter().find(|range| {
            match range.ranges.binary_search_by_key(&pc, |r| r.begin) {
                Ok(_) => true,
                Err(pos) => {
                    let found = range.ranges[..pos]
                        .iter()
                        .rev()
                        .any(|range| range.begin <= pc && pc <= range.end);
                    found
                }
            }
        })
    }

    pub fn find_place_from_pc(&self, pc: usize) -> Option<Place<EndianRcSlice>> {
        let pc = pc as u64;
        let unit = self.find_unit(pc)?;

        let pos = match unit.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => p,
            Err(p) => p - 1,
        };

        unit.get_place(pos)
    }

    fn parse(
        dwarf: &gimli::Dwarf<EndianRcSlice>,
    ) -> anyhow::Result<Vec<ParsedUnit<EndianRcSlice>>> {
        dwarf
            .units()
            .enumerate()
            .map(|(unit_idx, header)| {
                let unit = dwarf.unit(header)?;

                let mut lines = vec![];
                let mut files = vec![];

                if let Some(ref lp) = unit.line_program {
                    let mut rows = lp.clone().rows();
                    lines = parse_lines(&mut rows)?;
                    files = parse_files(dwarf, &unit, &rows)?;
                }

                lines.sort_by_key(|x| x.address);

                let mut unit_ranges = dwarf.unit_ranges(&unit)?.collect::<Vec<_>>()?;
                unit_ranges.sort_by_key(|r| r.begin);

                let mut dies = vec![];
                let mut current_fn_die_idx = 0;
                let mut die_ranges = vec![];
                let mut variables = HashMap::new();

                let mut cursor = unit.entries();
                while let Some((_, die)) = cursor.next_dfs()? {
                    let mut low_pc = None;
                    if let Some(l_pc_attr) = die.attr(DW_AT_low_pc)? {
                        match l_pc_attr.value() {
                            gimli::AttributeValue::Addr(val) => low_pc = Some(val),
                            gimli::AttributeValue::DebugAddrIndex(index) => {
                                low_pc = Some(dwarf.address(&unit, index)?)
                            }
                            _ => {}
                        }
                    }

                    let mut high_pc = None;
                    if let Some(h_pc_attr) = die.attr(DW_AT_high_pc)? {
                        match h_pc_attr.value() {
                            gimli::AttributeValue::Addr(val) => high_pc = Some(val),
                            gimli::AttributeValue::DebugAddrIndex(index) => {
                                high_pc = Some(dwarf.address(&unit, index)?)
                            }
                            gimli::AttributeValue::Udata(val) => {
                                high_pc = Some(low_pc.unwrap_or(0) + val)
                            }
                            _ => {}
                        }
                    }

                    let name = die
                        .attr(DW_AT_name)?
                        .and_then(|attr| dwarf.attr_string(&unit, attr.value()).ok());

                    dies.push(Die {
                        tag: die.tag(),
                        name: name
                            .map(|s| s.to_string_lossy().map(|s| s.to_string()))
                            .transpose()?,
                        low_pc,
                        high_pc,
                        idx: dies.len(),
                        unit_idx,
                        location: None,
                        fb_addr: None,
                    });

                    dwarf.die_ranges(&unit, die)?.for_each(|r| {
                        die_ranges.push(DieRange {
                            range: r,
                            die_idx: dies.len() - 1,
                        });
                        Ok(())
                    })?;

                    if die.tag() == DW_TAG_subprogram {
                        current_fn_die_idx = dies.len() - 1;

                        if let Some(fb) = die.attr(DW_AT_frame_base)? {
                            dies.last_mut().unwrap().fb_addr = Some(fb);
                        }
                    }
                    if die.tag() == DW_TAG_variable {
                        if let Some(at_loc) = die.attr(DW_AT_location)? {
                            dies.last_mut().unwrap().location = Some(at_loc);
                            let vars: &mut Vec<_> =
                                variables.entry(current_fn_die_idx).or_default();

                            (*vars).push(dies.len() - 1);
                        }
                    }
                }
                die_ranges.sort_by_key(|dr| dr.range.begin);

                let parsed_unit = ParsedUnit {
                    files,
                    lines,
                    ranges: unit_ranges,
                    local_vars: variables,
                    dies,
                    die_ranges,
                    _unit: Rc::new(unit),
                };

                debug_assert!(parsed_unit
                    .ranges
                    .iter()
                    .tuple_windows()
                    .all(|(r1, r2)| r1.begin <= r2.begin));

                Ok(parsed_unit)
            })
            .collect::<Vec<_>>()
            .map_err(Into::into)
    }

    pub fn find_variables(&self, function: &Die) -> Vec<&Die> {
        let idx = function.idx;

        match self.unit_ranges[function.unit_idx].local_vars.get(&idx) {
            None => vec![],
            Some(indexes) => indexes
                .iter()
                .map(|die_idx| {
                    self.unit_ranges[function.unit_idx]
                        .dies
                        .get(*die_idx)
                        .unwrap()
                })
                .collect(),
        }
    }

    pub fn find_function_from_pc(&self, pc: usize) -> Option<&Die> {
        let pc = pc as u64;
        let unit = self.find_unit(pc)?;

        let find_pos = match unit
            .die_ranges
            .binary_search_by_key(&pc, |dr| dr.range.begin)
        {
            Ok(pos) => pos + 1,
            Err(pos) => pos,
        };

        unit.die_ranges[..find_pos]
            .iter()
            .rev()
            .find(|dr| {
                if unit.dies[dr.die_idx].tag != DW_TAG_subprogram {
                    return false;
                };
                dr.range.begin <= pc && pc <= dr.range.end
            })
            .map(|dr| &unit.dies[dr.die_idx])
    }

    pub fn find_function_from_name(&self, fn_name: &str) -> Option<&Die> {
        for unit in &self.unit_ranges {
            for die in &unit.dies {
                if let Some(name) = die.name.as_ref() {
                    if name == fn_name {
                        return Some(die);
                    }
                }
            }
        }
        None
    }

    pub fn find_stmt_line(&self, file: &str, line: u64) -> Option<Place<'_>> {
        for unit in &self.unit_ranges {
            let found = unit
                ._unit
                .name
                .as_ref()
                .map(|n| {
                    n.to_string_lossy()
                        // TODO find file substring look weird
                        .map(|s| s.find(file).is_some())
                        .unwrap_or(false)
                })
                .unwrap_or_default();

            if !found {
                continue;
            }

            for (pos, line_row) in unit.lines.iter().enumerate() {
                if line_row.line == line {
                    return unit.get_place(pos);
                }
            }
        }
        None
    }

    pub fn find_symbol(&self, name: &str) -> Option<&Symbol> {
        if let Some(ref st) = self.symbol_table {
            return st.get(name);
        };

        None
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
