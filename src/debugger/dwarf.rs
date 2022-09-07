use fallible_iterator::FallibleIterator;
use gimli::{Dwarf, Reader, RunTimeEndian, Unit};
use itertools::Itertools;
use object::{Object, ObjectSection};
use std::borrow::Cow;
use std::num::NonZeroU64;
use std::rc::Rc;

type EndianRcSlice = gimli::EndianRcSlice<gimli::RunTimeEndian>;

pub struct DwarfContext<R: gimli::Reader> {
    _inner: Dwarf<R>,
    unit_ranges: Vec<ParsedUnit<R>>,
}

#[derive(Debug)]
pub struct LineRow {
    pub address: u64,
    pub file_index: u64,
    pub line: u64,
    pub column: u64,
}

struct ParsedUnit<R: gimli::Reader> {
    files: Vec<String>,
    ranges: Vec<gimli::Range>,
    lines: Vec<LineRow>,
    _unit: Rc<Unit<R>>,
}

impl DwarfContext<EndianRcSlice> {
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

        Ok(Self {
            unit_ranges: Self::parse(&dwarf)?,
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

    pub fn find_line_from_pc(&self, pc: u64) -> Option<(String, &LineRow)> {
        let unit = self.find_unit(pc)?;

        let pos = match unit.lines.binary_search_by_key(&pc, |line| line.address) {
            Ok(p) => p,
            Err(p) => p - 1,
        };

        unit.lines.get(pos).map(|line| {
            (
                unit.files
                    .get(line.file_index as usize)
                    .cloned()
                    .unwrap_or_default(),
                line,
            )
        })
    }

    fn parse(
        dwarf: &gimli::Dwarf<EndianRcSlice>,
    ) -> anyhow::Result<Vec<ParsedUnit<EndianRcSlice>>> {
        dwarf
            .units()
            .map(|header| {
                let unit = dwarf.unit(header)?;

                let mut lines = vec![];
                let mut files = vec![];

                if let Some(ref lp) = unit.line_program {
                    let mut rows = lp.clone().rows();
                    lines = parse_lines(&mut rows)?;
                    files = parse_files(dwarf, &unit, &rows)?;
                }

                lines.sort_by_key(|x| x.address);

                let mut ranges = dwarf.unit_ranges(&unit)?.collect::<Vec<_>>()?;
                ranges.sort_by_key(|r| r.begin);

                let parsed_unit = ParsedUnit {
                    files,
                    lines,
                    ranges,
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

    pub fn find_function_from_pc<F>(&self, pc: u64) {}
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
