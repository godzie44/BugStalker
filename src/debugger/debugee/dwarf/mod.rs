pub mod eval;
mod loader;
mod location;
mod symbol;
pub mod r#type;
pub mod unit;
pub mod unwind;
mod utils;

pub use self::unwind::DwarfUnwinder;

use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::eval::AddressKind;
use crate::debugger::debugee::dwarf::location::Location as DwarfLocation;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::r#type::EvaluationContext;
use crate::debugger::debugee::dwarf::symbol::SymbolTab;
use crate::debugger::debugee::dwarf::unit::{
    DieRef, DieVariant, DwarfUnitParser, Entry, FunctionDie, Node, ParameterDie,
    PlaceDescriptorOwned, Unit, VariableDie,
};
use crate::debugger::debugee::dwarf::utils::PathSearchIndex;
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{
    DebugIDFormat, FBANotAnExpression, FunctionNotFound, NoFBA, NoFunctionRanges, UnitNotFound,
};
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::debugger::variable::select::ObjectBinaryRepr;
use crate::debugger::ExplorationContext;
use crate::{muted_error, resolve_unit_call, version_switch, weak_error};
use fallible_iterator::FallibleIterator;
use gimli::CfaRule::RegisterAndOffset;
use gimli::{
    Attribute, BaseAddresses, CfaRule, DebugAddr, DebugInfoOffset, DebugPubTypes, Dwarf, EhFrame,
    Expression, LocationLists, Range, Reader, RunTimeEndian, Section, UnitOffset, UnwindContext,
    UnwindSection, UnwindTableRow,
};
use log::debug;
use memmap2::Mmap;
use object::{Object, ObjectSection};
use rayon::prelude::*;
use regex::Regex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::{Add, Deref};
use std::path::{Path, PathBuf};
use std::{fs, path};
pub use symbol::Symbol;
use trie_rs::Trie;
use unit::PlaceDescriptor;
use walkdir::WalkDir;

pub type EndianArcSlice = gimli::EndianArcSlice<gimli::RunTimeEndian>;

pub struct DebugInformation<R: gimli::Reader = EndianArcSlice> {
    file: PathBuf,
    inner: Dwarf<R>,
    eh_frame: EhFrame<R>,
    bases: BaseAddresses,
    units: Option<Vec<Unit>>,
    symbol_table: Option<SymbolTab>,
    pub_names: Option<Trie<u8>>,
    pub_types: HashMap<String, (DebugInfoOffset, UnitOffset)>,
    /// Index for fast search files by full path or part of file path. Contains unit index and
    /// indexes of lines in [`Unit::lines`] vector that belongs to a file, indexes are ordered by
    /// line number, column number and address.
    files_index: PathSearchIndex<(usize, Vec<usize>)>,
}

impl Clone for DebugInformation {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            inner: Dwarf {
                debug_abbrev: self.inner.debug_abbrev.clone(),
                debug_addr: self.inner.debug_addr.clone(),
                debug_aranges: self.inner.debug_aranges.clone(),
                debug_info: self.inner.debug_info.clone(),
                debug_line: self.inner.debug_line.clone(),
                debug_line_str: self.inner.debug_line_str.clone(),
                debug_str: self.inner.debug_str.clone(),
                debug_str_offsets: self.inner.debug_str_offsets.clone(),
                debug_types: self.inner.debug_types.clone(),
                locations: self.inner.locations.clone(),
                ranges: self.inner.ranges.clone(),
                file_type: self.inner.file_type,
                sup: self.inner.sup.clone(),
                abbreviations_cache: Default::default(),
            },
            eh_frame: self.eh_frame.clone(),
            bases: self.bases.clone(),
            units: self.units.clone(),
            symbol_table: self.symbol_table.clone(),
            // it is ok cause pub_names currently unused, maybe it will be changed in future
            pub_names: None,
            pub_types: self.pub_types.clone(),
            files_index: self.files_index.clone(),
        }
    }
}

/// Using this macro means a promise that debug information exists in context of usage.
#[macro_export]
macro_rules! debug_info_exists {
    ($expr: expr) => {
        $expr.expect("unreachable: debug information must exists")
    };
}

impl DebugInformation {
    /// Return path to executable file with (possible) debug information.
    /// In case of executable contains debug information in separate file this file may not have
    /// a debug information but contains a link to it.
    pub fn pathname(&self) -> &Path {
        self.file.as_path()
    }

    /// The location lists in the .debug_loc and .debug_loclists sections.
    pub fn locations(&self) -> &LocationLists<EndianArcSlice> {
        &self.inner.locations
    }

    /// Return all dwarf units or error if no debug information found.
    fn get_units(&self) -> Result<&[Unit], Error> {
        self.units
            .as_deref()
            .ok_or(Error::NoDebugInformation("file"))
    }

    /// Return false if file dont contains a debug information.
    pub fn has_debug_info(&self) -> bool {
        self.units.is_some()
    }

    /// Return unit by its index.
    ///
    /// # Arguments
    ///
    /// * `idx`: unit index
    ///
    /// # Panics
    ///
    /// Panic if unit not found.
    pub fn unit_ensure(&self, idx: usize) -> &Unit {
        &debug_info_exists!(self.get_units())[idx]
    }

    /// Return unit count. Return 0 if no debug information exists.
    #[inline(always)]
    pub fn unit_count(&self) -> usize {
        self.units
            .as_ref()
            .map(|units| units.len())
            .unwrap_or_default()
    }

    /// Return `Some(true)` if .debug_pubnames section contains template last part (for example
    /// this may be a function name), `Some(false)` if not contains and `None` if no .debug_pubnames
    /// section in debug information file.
    ///
    /// This function is useful, for example, to determine the presence of a function in a file. The
    /// result is false positive, means that if result is `Some(false)` than function not exists, but
    /// it may exists or not exists if result is `None` (we need analyze die's for determine).
    ///
    /// # Arguments
    ///
    /// * `tpl`: template for object or function name.
    pub fn tpl_in_pub_names(&self, tpl: &str) -> Option<bool> {
        debug_assert!(tpl.split("::").count() > 0);
        let needle = tpl.split("::").last().expect("at least one exists");
        self.pub_names.as_ref().map(|pub_names| {
            let found = pub_names.predictive_search(needle);
            !found.is_empty()
        })
    }

    fn evaluate_cfa(
        &self,
        debugee: &Debugee,
        registers: &DwarfRegisterMap,
        utr: &UnwindTableRow<EndianArcSlice>,
        ctx: &ExplorationContext,
    ) -> Result<RelocatedAddress, Error> {
        let rule = utr.cfa();
        match rule {
            RegisterAndOffset { register, offset } => {
                let ra = registers.value(*register)?;
                Ok(RelocatedAddress::from(ra as usize).offset(*offset as isize))
            }
            CfaRule::Expression(expr) => {
                let unit = debug_info_exists!(self.find_unit_by_pc(ctx.location().global_pc))
                    .ok_or(UnitNotFound(ctx.location().global_pc))?;
                let evaluator = resolve_unit_call!(&self.inner, unit, evaluator, debugee);
                let expr_result = evaluator.evaluate(ctx, expr.clone())?;

                Ok((expr_result.into_scalar::<usize>(AddressKind::Value)?).into())
            }
        }
    }

    pub fn get_cfa(
        &self,
        debugee: &Debugee,
        expl_ctx: &ExplorationContext,
    ) -> Result<RelocatedAddress, Error> {
        let mut ctx = Box::new(UnwindContext::new());
        let row = self.eh_frame.unwind_info_for_address(
            &self.bases,
            &mut ctx,
            expl_ctx.location().global_pc.into(),
            EhFrame::cie_from_offset,
        )?;
        self.evaluate_cfa(
            debugee,
            &DwarfRegisterMap::from(RegisterMap::current(expl_ctx.pid_on_focus())?),
            row,
            expl_ctx,
        )
    }

    pub fn debug_addr(&self) -> &DebugAddr<EndianArcSlice> {
        &self.inner.debug_addr
    }

    /// Return a list of all known files.
    pub fn known_files(&self) -> Result<impl Iterator<Item = &PathBuf>, Error> {
        Ok(self.get_units()?.iter().flat_map(|unit| unit.files()))
    }

    /// Searches for a unit by occurrences of PC in its range.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter value
    ///
    /// returns: `None` if unit not found, error if no debug information found
    fn find_unit_by_pc(&self, pc: GlobalAddress) -> Result<Option<&Unit>, Error> {
        Ok(self.get_units()?.iter().find(|&unit| {
            match unit
                .ranges()
                .binary_search_by_key(&(pc.into()), |r| r.begin)
            {
                Ok(_) => true,
                Err(pos) => unit.ranges()[..pos]
                    .iter()
                    .rev()
                    .any(|range| pc.in_range(range)),
            }
        }))
    }

    /// Returns best matched place by program counter global address.
    pub fn find_place_from_pc(&self, pc: GlobalAddress) -> Result<Option<PlaceDescriptor>, Error> {
        let mb_unit = self.find_unit_by_pc(pc)?;
        Ok(mb_unit.and_then(|u| u.find_place_by_pc(pc)))
    }

    /// Returns place with line address equals to program counter global address.
    pub fn find_exact_place_from_pc(
        &self,
        pc: GlobalAddress,
    ) -> Result<Option<PlaceDescriptor>, Error> {
        let mb_unit = self.find_unit_by_pc(pc)?;
        Ok(mb_unit.and_then(|u| u.find_exact_place_by_pc(pc)))
    }

    /// Return a function inside which the given instruction is located.
    ///
    /// # Arguments
    ///
    /// * `pc`: instruction global address.
    pub fn find_function_by_pc(
        &self,
        pc: GlobalAddress,
    ) -> Result<Option<ContextualDieRef<FunctionDie>>, Error> {
        let mb_unit = self.find_unit_by_pc(pc)?;
        Ok(mb_unit.and_then(|unit| {
            let pc = u64::from(pc);
            let die_ranges = resolve_unit_call!(self.dwarf(), unit, die_ranges);
            let find_pos = match die_ranges.binary_search_by_key(&pc, |dr| dr.range.begin) {
                Ok(pos) => {
                    let mut idx = pos + 1;
                    while idx < die_ranges.len() && die_ranges[idx].range.begin == pc {
                        idx += 1;
                    }
                    idx
                }
                Err(pos) => pos,
            };

            die_ranges[..find_pos].iter().rev().find_map(|dr| {
                let entry = resolve_unit_call!(&self.inner, unit, entry, dr.die_idx);
                if let DieVariant::Function(ref func) = entry.die {
                    if dr.range.begin <= pc && pc < dr.range.end {
                        return Some(ContextualDieRef {
                            debug_info: self,
                            node: &entry.node,
                            unit_idx: unit.idx(),
                            die: func,
                        });
                    }
                };
                None
            })
        }))
    }

    /// Return a functions relevant to template.
    ///
    /// # Arguments
    ///
    /// * `template`: search template (full function path or part of this path).
    pub fn search_functions(
        &self,
        template: &str,
    ) -> Result<Vec<ContextualDieRef<FunctionDie>>, Error> {
        let units = self.get_units()?;
        let result: Vec<_> = units
            .par_iter()
            .flat_map(|unit| {
                let entries = resolve_unit_call!(self.dwarf(), unit, search_functions, template);
                entries
                    .iter()
                    .map(|entry| {
                        let func = entry.die.unwrap_function();
                        ContextualDieRef {
                            debug_info: self,
                            unit_idx: unit.idx(),
                            node: &entry.node,
                            die: func,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        Ok(result)
    }

    /// Return closest [`PlaceDescriptor`] for given file and line.
    /// Closest means that returns descriptor for target line or, if no descriptor for target line,
    /// place for next line after target.
    ///
    /// # Arguments
    ///
    /// * `file`: file name template (full path or part of a file path)
    /// * `line`: line number
    pub fn find_closest_place(
        &self,
        file_tpl: &str,
        line: u64,
    ) -> Result<Vec<PlaceDescriptor<'_>>, Error> {
        let files = self.files_index.get(file_tpl);

        let mut unique_subprograms = HashSet::new();
        let mut result = vec![];

        let possible_lines = &[line, line + 1];

        for &needle_line in possible_lines {
            if !result.is_empty() {
                break;
            }

            for (unit_idx, file_lines) in &files {
                let unit = self.unit_ensure(*unit_idx);

                let mut suitable_places_in_unit = vec![];

                let mut i = 0;
                while i < file_lines.len() {
                    let mut line_idx = file_lines[i];
                    let next_line_row = unit.line(line_idx);

                    if suitable_places_in_unit.is_empty() {
                        // no places found at this point,
                        // try to find the closest place to a target line
                        if next_line_row.line != needle_line || !next_line_row.is_stmt() {
                            i += 1;
                            continue;
                        }

                        // now check that there is no prolog end in neighborhood line rows,
                        // if there is one then take it.
                        // This sets priority of line rows with PE over other
                        // line rows at this line as a breakpoint candidate
                        let mut ahead_idx = i + 1;
                        loop {
                            let Some(&ahead_line_idx) = file_lines.get(ahead_idx) else {
                                break;
                            };

                            let line_row = unit.line(ahead_line_idx);
                            if line_row.line != next_line_row.line || !line_row.is_stmt() {
                                break;
                            }

                            if line_row.prolog_end() {
                                line_idx = ahead_line_idx;
                                i = ahead_idx;
                                break;
                            }
                            ahead_idx += 1;
                        }

                        if let Some(place) = unit.find_place_by_idx(line_idx) {
                            suitable_places_in_unit.push(place);
                        }
                    } else {
                        // At least one line is found,
                        // now try to find lines with the same col and row
                        // as in found place in source code.
                        // This covers a case when compiler
                        // generates multiple representations of a single line, for example, when
                        // source code line in a part of a template function.
                        let line = suitable_places_in_unit[0].line_number;
                        let col = suitable_places_in_unit[0].column_number;
                        let pe = suitable_places_in_unit[0].prolog_end;
                        let eb = suitable_places_in_unit[0].epilog_begin;
                        let es = suitable_places_in_unit[0].end_sequence;

                        if next_line_row.line != line
                            || next_line_row.column != col
                            || next_line_row.prolog_end() != pe
                            || next_line_row.epilog_begin() != eb
                            || next_line_row.end_sequence() != es
                            || !next_line_row.is_stmt()
                        {
                            i += 1;
                            continue;
                        }

                        if let Some(place) = unit.find_place_by_idx(line_idx) {
                            suitable_places_in_unit.push(place);
                        }
                    }

                    i += 1;
                }

                for suitable_place in suitable_places_in_unit {
                    // only one place for a single unique subprogram is allowed
                    // to apply this rule as a filter for all places
                    if let Some(func) = self.find_function_by_pc(suitable_place.address)? {
                        if !unique_subprograms.contains(&func.die.base_attributes) {
                            unique_subprograms.insert(&func.die.base_attributes);
                            result.push(suitable_place);
                        }
                    } else {
                        // do we need place if we cant find a function?
                        result.push(suitable_place);
                    }
                }
            }
        }

        Ok(result)
    }

    /// Search all places for functions that relevant to template.
    ///
    /// # Arguments
    ///
    /// * `template`: search template (full function path or part of this path).
    pub fn search_places_for_fn_tpl(
        &self,
        template: &str,
    ) -> Result<Vec<PlaceDescriptorOwned>, Error> {
        Ok(self
            .search_functions(template)?
            .into_iter()
            .filter_map(|fn_die| {
                weak_error!(fn_die.prolog_end_place()).map(|place| place.to_owned())
            })
            .collect())
    }

    pub fn find_symbols(&self, regex: &Regex) -> Vec<&Symbol> {
        let symbols = self
            .symbol_table
            .as_ref()
            .map(|table| {
                let keys = table
                    .keys()
                    .filter(|key| regex.find(key.as_str()).is_some());
                keys.map(|k| &table[k]).collect()
            })
            .unwrap_or_default();
        symbols
    }

    pub fn deref_die<'this>(
        &'this self,
        default_unit: &'this Unit,
        reference: DieRef,
    ) -> Option<(&'this Entry, &'this Unit)> {
        match reference {
            DieRef::Unit(offset) => {
                let entry = resolve_unit_call!(&self.inner, default_unit, find_entry, offset);
                entry.map(|e| (e, default_unit))
            }
            DieRef::Global(offset) => {
                let unit = self.find_unit(offset)?;
                let offset = UnitOffset(offset.0 - unit.offset().unwrap_or(DebugInfoOffset(0)).0);
                let entry = resolve_unit_call!(&self.inner, unit, find_entry, offset);
                entry.map(|e| (e, unit))
            }
        }
    }

    pub fn find_variables(
        &self,
        location: Location,
        name: &str,
    ) -> Result<Vec<ContextualDieRef<'_, VariableDie>>, Error> {
        let units = self.get_units()?;

        let mut found = vec![];
        for unit in units {
            let mb_var_locations = resolve_unit_call!(self.dwarf(), unit, locate_var_die, name);
            if let Some(vars) = mb_var_locations {
                vars.iter().for_each(|(_, entry_idx)| {
                    let entry = resolve_unit_call!(&self.inner, unit, entry, *entry_idx);
                    if let DieVariant::Variable(ref var) = entry.die {
                        let variable = ContextualDieRef {
                            debug_info: self,
                            unit_idx: unit.idx(),
                            node: &entry.node,
                            die: var,
                        };

                        if variable.valid_at(location.global_pc) {
                            found.push(variable);
                        }
                    }
                });
            }
        }

        for unit in units {
            let rustc_version = unit.rustc_version().unwrap_or_default();

            let tls_ns_part = version_switch!(
                rustc_version,
                (1, 0, 0) ..= (1, 79, u32::MAX) => {
                    // now check tls variables
                    // for rust we expect that tls variable represents in dwarf like
                    // variable with name "__KEY" and namespace like [.., variable_name, __getit]
                    vec![name, "__getit"]
                },
                (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => {
                    vec![name]
                },
            );
            let tls_ns_part = tls_ns_part.expect("infallible: all rustc versions are covered");

            let mut tls_collector = |(namespaces, entry_idx): &(NamespaceHierarchy, usize)| {
                if namespaces.contains(&tls_ns_part) {
                    let entry = resolve_unit_call!(&self.inner, unit, entry, *entry_idx);
                    if let DieVariant::Variable(ref var) = entry.die {
                        found.push(ContextualDieRef {
                            debug_info: self,
                            unit_idx: unit.idx(),
                            node: &entry.node,
                            die: var,
                        });
                    }
                }
            };

            if let Some(vars) = resolve_unit_call!(self.dwarf(), unit, locate_var_die, "__KEY") {
                vars.iter().for_each(&mut tls_collector);
            };
            if let Some(vars) = resolve_unit_call!(self.dwarf(), unit, locate_var_die, "VAL") {
                vars.iter().for_each(&mut tls_collector);
            };
        }

        Ok(found)
    }

    /// Return reference (unit and die offsets) to type die by type name.
    ///
    /// Search from `pub_types` section in priority, but if `pub_types` is empty,
    /// then a unit full scan may be occurred.
    pub fn find_type_die_ref(&self, name: &str) -> Option<(DebugInfoOffset, UnitOffset)> {
        if self.pub_types.is_empty() {
            self.get_units().ok()?.iter().find_map(|u| {
                u.offset().and_then(|u_offset| {
                    let type_ref_in_unit = resolve_unit_call!(&self.inner, u, locate_type, name)?;
                    Some((u_offset, type_ref_in_unit))
                })
            })
        } else {
            self.pub_types.get(name).copied()
        }
    }

    /// Return unit found at offset.
    #[inline(always)]
    pub fn find_unit(&self, offset: DebugInfoOffset) -> Option<&Unit> {
        let mb_unit = debug_info_exists!(self.get_units())
            .binary_search_by_key(&Some(offset), |u| u.offset());
        match mb_unit {
            Ok(_) | Err(0) => None,
            Err(pos) => Some(self.unit_ensure(pos - 1)),
        }
    }

    pub fn dwarf(&self) -> &Dwarf<EndianArcSlice> {
        &self.inner
    }

    /// Return the maximum and minimum address from the collection of unit ranges.
    pub fn range(&self) -> Option<Range> {
        let units = self.get_units().ok()?;

        // ranges already sorted by begin addr
        let begin = units
            .iter()
            .filter_map(|u| u.ranges().first().map(|r| r.begin))
            .min()?;

        let end = units
            .iter()
            .map(|u| {
                u.ranges().iter().fold(
                    begin,
                    |end, range| if range.end > end { range.end } else { end },
                )
            })
            .max()?;

        Some(Range { begin, end })
    }
}

#[derive(Default)]
pub struct DebugInformationBuilder;

impl DebugInformationBuilder {
    // todo configure this path
    const DEBUG_FILES_DIR: &'static str = "/usr/lib/debug";

    fn get_dwarf_from_separate_debug_file<'a, 'b, OBJ>(
        &self,
        obj_file: &'a OBJ,
    ) -> Result<Option<(PathBuf, Mmap)>, Error>
    where
        'a: 'b,
        OBJ: Object<'a, 'b>,
    {
        // try build-id
        let debug_id_sect = obj_file.section_by_name(".note.gnu.build-id");
        if let Some(build_id) = debug_id_sect {
            let data = build_id.data()?;
            // skip 16 byte header
            let note = &data[16..];
            if note.len() < 2 {
                return Err(DebugIDFormat);
            }

            let dir = format!("{:02x}", note[0]);
            let file = note[1..]
                .iter()
                .map(|&b| format!("{:02x}", b))
                .collect::<Vec<String>>()
                .join("")
                .add(".debug");

            let path = PathBuf::from(Self::DEBUG_FILES_DIR)
                .join(".build-id")
                .join(dir)
                .join(file);
            let file = fs::File::open(path.as_path())?;
            let mmap = unsafe { memmap2::Mmap::map(&file)? };
            return Ok(Some((path, mmap)));
        }

        // try debug link
        let debug_link_sect = obj_file.section_by_name(".gnu_debuglink");
        if let Some(sect) = debug_link_sect {
            let data = sect.data()?;
            let data: Vec<u8> = data.iter().take_while(|&&b| b != 0).copied().collect();
            let debug_link = std::str::from_utf8(&data)?;

            for entry in WalkDir::new(Self::DEBUG_FILES_DIR)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let name = entry.file_name().to_string_lossy();
                if name.contains(debug_link) {
                    let file = fs::File::open(entry.path())?;
                    let mmap = unsafe { memmap2::Mmap::map(&file)? };
                    return Ok(Some((entry.path().to_path_buf(), mmap)));
                }
            }
        }

        Ok(None)
    }

    pub fn build(&self, obj_path: &Path, file: &object::File) -> Result<DebugInformation, Error> {
        let endian = if file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        let eh_frame = EhFrame::load(|id| -> Result<EndianArcSlice, Error> {
            loader::load_section(id, file, endian)
        })?;
        let section_addr = |name: &str| -> Option<u64> {
            file.sections().find_map(|section| {
                if section.name().ok()? == name {
                    Some(section.address())
                } else {
                    None
                }
            })
        };
        let mut bases = BaseAddresses::default();
        if let Some(got) = section_addr(".got") {
            bases = bases.set_got(got);
        }
        if let Some(text) = section_addr(".text") {
            bases = bases.set_text(text);
        }
        if let Some(eh) = section_addr(".eh_frame") {
            bases = bases.set_eh_frame(eh);
        }
        if let Some(eh_frame_hdr) = section_addr(".eh_frame_hdr") {
            bases = bases.set_eh_frame_hdr(eh_frame_hdr);
        }

        let debug_split_file_data;
        let debug_split_file;
        let debug_info_file =
            if let Ok(Some((path, debug_file))) = self.get_dwarf_from_separate_debug_file(file) {
                debug!(target: "dwarf-loader", "{obj_path:?} has separate debug information file");
                debug!(target: "dwarf-loader", "load debug information from {path:?}");
                debug_split_file_data = debug_file;
                debug_split_file = object::File::parse(&*debug_split_file_data)?;
                &debug_split_file
            } else {
                debug!(target: "dwarf-loader", "load debug information from {obj_path:?}");
                file
            };

        let dwarf = loader::load_par(debug_info_file, endian)?;
        let symbol_table = SymbolTab::new(debug_info_file);

        // let mb_pub_names_sect = muted_error!(DebugPubNames::load(|id| {
        //     loader::load_section(id, debug_info_file, endian)
        // }));
        // let pub_names = mb_pub_names_sect.and_then(|pub_names_sect| {
        //     let mut names_trie = TrieBuilder::new();
        //     muted_error!(pub_names_sect.items().for_each(|pub_name| {
        //         let name = pub_name.name().to_string_lossy()?;
        //         names_trie.push(name.as_bytes());
        //         Ok(())
        //     }))?;
        //     Some(names_trie.build())
        // });

        // Currently pub_names section is not used
        // because the current function-search algorithm anyway
        // will load all dwarf DIE information after name was found in .debug_pubnames section.
        // Maybe this will be changed in the future, when debugger loads only DIE that points by
        // name from .debug_pubnames section.
        let pub_names = None;

        let mb_pub_types_sect = muted_error!(DebugPubTypes::load(|id| {
            loader::load_section(id, debug_info_file, endian)
        }));
        let pub_types = mb_pub_types_sect.and_then(|pub_types_sect| {
            pub_types_sect
                .items()
                .map(|e| {
                    let type_name = e.name().to_string_lossy()?.to_string();
                    let unit_offset = e.unit_header_offset();
                    Ok((type_name, (unit_offset, e.die_offset())))
                })
                .collect()
                .ok()
        });

        let parser = DwarfUnitParser::new(&dwarf);
        let headers = dwarf.units().collect::<Vec<_>>()?;

        if headers.is_empty() {
            // no units means no debug info
            debug!(target: "dwarf-loader", "no debug information for {obj_path:?}");

            return Ok(DebugInformation {
                file: obj_path.to_path_buf(),
                inner: dwarf,
                eh_frame,
                bases,
                units: None,
                symbol_table,
                pub_names,
                pub_types: pub_types.unwrap_or_default(),
                files_index: PathSearchIndex::new(""),
            });
        }

        let mut units = headers
            .into_par_iter()
            .map(|header| -> gimli::Result<Unit> {
                let unit = parser.parse(header)?;
                Ok(unit)
            })
            .collect::<gimli::Result<Vec<_>>>()?;

        units.sort_unstable_by_key(|u| u.offset());
        units.iter_mut().enumerate().for_each(|(i, u)| u.set_idx(i));

        let mut files_index = PathSearchIndex::new(path::MAIN_SEPARATOR_STR);
        units.iter().for_each(|unit| {
            unit.file_path_with_lines_pairs()
                .for_each(|(file_path, lines)| {
                    files_index.insert(file_path, (unit.idx(), lines));
                });
        });

        Ok(DebugInformation {
            file: obj_path.to_path_buf(),
            inner: dwarf,
            eh_frame,
            bases,
            units: Some(units),
            symbol_table,
            pub_names,
            pub_types: pub_types.unwrap_or_default(),
            files_index,
        })
    }
}

pub trait AsAllocatedData {
    fn name(&self) -> Option<&str>;

    fn type_ref(&self) -> Option<DieRef>;

    fn location(&self) -> Option<&Attribute<EndianArcSlice>>;

    fn location_expr(
        &self,
        dwarf_ctx: &DebugInformation<EndianArcSlice>,
        unit: &Unit,
        pc: GlobalAddress,
    ) -> Option<Expression<EndianArcSlice>> {
        let location = self.location()?;
        DwarfLocation(location).try_as_expression(dwarf_ctx, unit, pc)
    }
}

impl AsAllocatedData for VariableDie {
    fn name(&self) -> Option<&str> {
        self.base_attributes.name.as_deref()
    }

    fn type_ref(&self) -> Option<DieRef> {
        self.type_ref
    }

    fn location(&self) -> Option<&Attribute<EndianArcSlice>> {
        self.location.as_ref()
    }
}

impl AsAllocatedData for ParameterDie {
    fn name(&self) -> Option<&str> {
        self.base_attributes.name.as_deref()
    }

    fn type_ref(&self) -> Option<DieRef> {
        self.type_ref
    }

    fn location(&self) -> Option<&Attribute<EndianArcSlice>> {
        self.location.as_ref()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NamespaceHierarchy(Vec<String>);

impl Deref for NamespaceHierarchy {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NamespaceHierarchy {
    /// Create namespace for selected node.
    ///
    /// # Arguments
    ///
    /// * `node`: first node information
    /// * `entries`: die list
    pub fn for_node(node: &Node, entries: &[Entry]) -> Self {
        let mut ns_chain = vec![];

        let mut p_idx = node.parent;
        let mut next_parent = || -> Option<&Entry> {
            let parent = &entries[p_idx?];
            p_idx = parent.node.parent;
            Some(parent)
        };
        while let Some(DieVariant::Namespace(ns)) = next_parent().map(|e| &e.die) {
            ns_chain.push(ns.base_attributes.name.clone().unwrap_or_default());
        }
        ns_chain.reverse();

        NamespaceHierarchy(ns_chain)
    }

    /// Return `true` if namespace part contains in target namespace, `false` otherwise.
    ///
    /// # Arguments
    ///
    /// * `needle`: searched part of the namespace
    pub fn contains(&self, needle: &[&str]) -> bool {
        self.0.windows(needle.len()).any(|slice| slice == needle)
    }

    /// Return (namespace, subroutine name) pair from mangled representation.
    ///
    /// # Arguments
    ///
    /// * `linkage_name`: mangled subroutine name
    #[inline(always)]
    pub fn from_mangled(linkage_name: &str) -> (Self, String) {
        let demangled = rustc_demangle::demangle(linkage_name);
        let demangled = format!("{demangled:#}");
        let mut parts: Vec<_> = demangled.split("::").map(ToString::to_string).collect();
        debug_assert!(!parts.is_empty());
        let fn_name = parts.pop().expect("function name must exists");
        (NamespaceHierarchy(parts), fn_name)
    }
}

pub struct ContextualDieRef<'a, T> {
    pub debug_info: &'a DebugInformation,
    pub unit_idx: usize,
    pub node: &'a Node,
    pub die: &'a T,
}

#[macro_export]
macro_rules! ctx_resolve_unit_call {
    ($self: ident, $fn_name: tt, $($arg: expr),*) => {{
        $crate::resolve_unit_call!($self.debug_info.dwarf(), $self.unit(), $fn_name, $($arg),*)
    }};
}

impl<'a, T> Clone for ContextualDieRef<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> Copy for ContextualDieRef<'a, T> {}

impl<'a, T> ContextualDieRef<'a, T> {
    pub fn namespaces(&self) -> NamespaceHierarchy {
        let entries = ctx_resolve_unit_call!(self, entries,);
        NamespaceHierarchy::for_node(self.node, entries)
    }

    pub fn unit(&self) -> &'a Unit {
        self.debug_info.unit_ensure(self.unit_idx)
    }
}

impl<'ctx> ContextualDieRef<'ctx, FunctionDie> {
    pub fn full_name(&self) -> Option<String> {
        self.die
            .base_attributes
            .name
            .as_ref()
            .map(|name| format!("{}::{}", self.die.namespace.0.join("::"), name))
    }

    pub fn frame_base_addr(
        &self,
        ctx: &ExplorationContext,
        debugee: &Debugee,
    ) -> Result<RelocatedAddress, Error> {
        let attr = self.die.fb_addr.as_ref().ok_or(NoFBA)?;

        let expr = DwarfLocation(attr)
            .try_as_expression(self.debug_info, self.unit(), ctx.location().global_pc)
            .ok_or(FBANotAnExpression)?;

        let evaluator = ctx_resolve_unit_call!(self, evaluator, debugee);
        let result = evaluator
            .evaluate(ctx, expr)?
            .into_scalar::<usize>(AddressKind::Value)?;
        Ok(result.into())
    }

    pub fn local_variables<'this>(
        &'this self,
        pc: GlobalAddress,
    ) -> Vec<ContextualDieRef<'ctx, VariableDie>> {
        let mut result = vec![];
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            let entry = ctx_resolve_unit_call!(self, entry, idx);
            if let DieVariant::Variable(ref var) = entry.die {
                let var_ref = ContextualDieRef {
                    debug_info: self.debug_info,
                    unit_idx: self.unit_idx,
                    node: &entry.node,
                    die: var,
                };

                if var_ref.valid_at(pc) {
                    result.push(var_ref);
                }
            }
            entry.node.children.iter().for_each(|i| queue.push_back(*i));
        }
        result
    }

    pub fn local_variable<'this>(
        &'this self,
        pc: GlobalAddress,
        needle: &str,
    ) -> Option<ContextualDieRef<'ctx, VariableDie>> {
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            let entry = ctx_resolve_unit_call!(self, entry, idx);
            if let DieVariant::Variable(ref var) = entry.die {
                let var_ref = ContextualDieRef {
                    debug_info: self.debug_info,
                    unit_idx: self.unit_idx,
                    node: &entry.node,
                    die: var,
                };

                if var_ref.die.name() == Some(needle) && var_ref.valid_at(pc) {
                    return Some(var_ref);
                }
            }
            entry.node.children.iter().for_each(|i| queue.push_back(*i));
        }
        None
    }

    pub fn parameters(&self) -> Vec<ContextualDieRef<'_, ParameterDie>> {
        let mut result = vec![];
        for &idx in &self.node.children {
            let entry = ctx_resolve_unit_call!(self, entry, idx);
            if let DieVariant::Parameter(ref var) = entry.die {
                result.push(ContextualDieRef {
                    debug_info: self.debug_info,
                    unit_idx: self.unit_idx,
                    node: &entry.node,
                    die: var,
                })
            }
        }
        result
    }

    /// Return function first instruction address.
    /// Address computed from function ranges if ranges is empty.
    pub fn start_instruction(&self) -> Result<GlobalAddress, Error> {
        Ok(self
            .ranges()
            .iter()
            .min_by(|r1, r2| r1.begin.cmp(&r2.begin))
            .ok_or_else(|| NoFunctionRanges(self.full_name()))?
            .begin
            .into())
    }

    /// Return address of the first location past the last instruction associated with the function.
    pub fn end_instruction(&self) -> Result<GlobalAddress, Error> {
        Ok(self
            .ranges()
            .iter()
            .max_by(|r1, r2| r1.begin.cmp(&r2.begin))
            .ok_or_else(|| NoFunctionRanges(self.full_name()))?
            .end
            .into())
    }

    pub fn prolog_start_place(&self) -> Result<PlaceDescriptor, Error> {
        let low_pc = self.start_instruction()?;

        debug_info_exists!(self.debug_info.find_place_from_pc(low_pc))
            .ok_or(FunctionNotFound(low_pc))
    }

    pub fn prolog_end_place(&self) -> Result<PlaceDescriptor, Error> {
        let mut place = self.prolog_start_place()?;
        while !place.prolog_end {
            match place.next() {
                None => break,
                Some(next_place) => place = next_place,
            }
        }

        Ok(place)
    }

    pub fn prolog(&self) -> Result<Range, Error> {
        let start = self.prolog_start_place()?;
        let end = self.prolog_end_place()?;
        Ok(Range {
            begin: start.address.into(),
            end: end.address.into(),
        })
    }

    pub fn ranges(&self) -> &[Range] {
        &self.die.base_attributes.ranges
    }

    pub fn inline_ranges(&self) -> Vec<Range> {
        let mut ranges = vec![];
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            let entry = ctx_resolve_unit_call!(self, entry, idx);
            if let DieVariant::InlineSubroutine(inline_subroutine) = &entry.die {
                ranges.extend(inline_subroutine.base_attributes.ranges.iter());
            }
            entry.node.children.iter().for_each(|i| queue.push_back(*i));
        }
        ranges
    }
}

impl<'ctx> ContextualDieRef<'ctx, VariableDie> {
    pub fn ranges(&self) -> Option<&[Range]> {
        if let Some(lb_idx) = self.die.lexical_block_idx {
            let entry = ctx_resolve_unit_call!(self, entry, lb_idx);
            let lb = entry.die.unwrap_lexical_block();
            Some(lb.base_attributes.ranges.as_ref())
        } else if let Some(fn_idx) = self.die.fn_block_idx {
            let entry = ctx_resolve_unit_call!(self, entry, fn_idx);
            let func = entry.die.unwrap_function();
            Some(func.base_attributes.ranges.as_ref())
        } else {
            None
        }
    }

    pub fn valid_at(&self, pc: GlobalAddress) -> bool {
        self.ranges()
            .map(|ranges| pc.in_ranges(ranges))
            .unwrap_or(true)
    }

    pub fn assume_parent_function(&self) -> Option<ContextualDieRef<'_, FunctionDie>> {
        let mut mb_parent = self.node.parent;

        while let Some(p) = mb_parent {
            let entry = ctx_resolve_unit_call!(self, entry, p);
            if let DieVariant::Function(ref func) = entry.die {
                return Some(ContextualDieRef {
                    debug_info: self.debug_info,
                    unit_idx: self.unit_idx,
                    node: &entry.node,
                    die: func,
                });
            }

            mb_parent = entry.node.parent;
        }

        None
    }
}

impl<'ctx> ContextualDieRef<'ctx, ParameterDie> {
    /// Return max range (with max `end` address) of an underlying function.
    /// If it's possible, `end` address in range equals to function epilog begin.
    pub fn max_range(&self) -> Option<Range> {
        self.die.fn_block_idx.and_then(|fn_idx| {
            let entry = ctx_resolve_unit_call!(self, entry, fn_idx);
            let func = entry.die.unwrap_function();
            let ranges = func.base_attributes.ranges.as_ref();

            if let Some(max_range) = ranges.iter().max_by_key(|r| r.end) {
                let eb = self.unit().find_eb(GlobalAddress::from(max_range.end));
                if let Some(eb) = eb {
                    return Some(Range {
                        begin: max_range.begin,
                        end: u64::from(eb.address),
                    });
                }
            }

            ranges.last().copied()
        })
    }
}

impl<'ctx, D: AsAllocatedData> ContextualDieRef<'ctx, D> {
    pub fn r#type(&self) -> Option<ComplexType> {
        let parser = r#type::TypeParser::new();
        Some(parser.parse(*self, self.die.type_ref()?))
    }

    pub fn read_value(
        &self,
        ctx: &ExplorationContext,
        debugee: &Debugee,
        r#type: &ComplexType,
    ) -> Option<ObjectBinaryRepr> {
        self.die
            .location_expr(self.debug_info, self.unit(), ctx.location().global_pc)
            .and_then(|expr| {
                let evaluator = ctx_resolve_unit_call!(self, evaluator, debugee);
                let eval_result = weak_error!(evaluator.evaluate(ctx, expr))?;
                let type_size = r#type.type_size_in_bytes(
                    &EvaluationContext {
                        evaluator: &evaluator,
                        expl_ctx: ctx,
                    },
                    r#type.root,
                )? as usize;
                let (address, raw_data) =
                    weak_error!(eval_result.into_raw_bytes(type_size, AddressKind::MemoryAddress))?;
                Some(ObjectBinaryRepr {
                    raw_data,
                    size: type_size,
                    address,
                })
            })
    }
}

#[cfg(test)]
mod test {
    use crate::debugger::debugee::dwarf::NamespaceHierarchy;

    #[test]
    fn test_namespace_from_mangled() {
        struct TestCase {
            mangled: &'static str,
            expected_ns: NamespaceHierarchy,
            expected_fn: &'static str,
        }

        let test_cases = vec![
            TestCase {
                mangled: "_ZN5tokio7runtime4task3raw7RawTask4poll17h7b89afb116da4cf2E",
                expected_ns: NamespaceHierarchy(vec![
                    "tokio".to_string(),
                    "runtime".to_string(),
                    "task".to_string(),
                    "raw".to_string(),
                    "RawTask".to_string(),
                ]),
                expected_fn: "poll",
            },
            TestCase {
                mangled: "poll",
                expected_ns: NamespaceHierarchy(vec![]),
                expected_fn: "poll",
            },
        ];

        for tc in test_cases {
            let (ns, name) = NamespaceHierarchy::from_mangled(tc.mangled);
            assert_eq!(ns, tc.expected_ns);
            assert_eq!(name, tc.expected_fn);
        }
    }
}
