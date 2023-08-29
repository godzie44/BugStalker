pub mod eval;
mod loader;
mod location;
mod symbol;
pub mod r#type;
pub mod unit;
pub mod unwind;

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
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::debugger::ExplorationContext;
use crate::{resolve_unit_call, weak_error};
use anyhow::{anyhow, bail};
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use gimli::CfaRule::RegisterAndOffset;
use gimli::{
    Attribute, BaseAddresses, CfaRule, DebugAddr, DebugInfoOffset, Dwarf, EhFrame, Expression,
    LocationLists, Range, RunTimeEndian, Section, UnitOffset, UnwindContext, UnwindSection,
    UnwindTableRow,
};
use log::{debug, info};
use memmap2::Mmap;
use object::{Object, ObjectSection};
use rayon::prelude::*;
use regex::Regex;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::fs;
use std::ops::{Add, Deref};
use std::path::{Path, PathBuf};
use std::sync::Arc;
pub use symbol::Symbol;
use walkdir::WalkDir;

pub type EndianArcSlice = gimli::EndianArcSlice<gimli::RunTimeEndian>;

pub struct DebugInformation<R: gimli::Reader = EndianArcSlice> {
    file: PathBuf,
    inner: Dwarf<R>,
    eh_frame: EhFrame<R>,
    bases: BaseAddresses,
    units: Option<Vec<Unit>>,
    symbol_table: Option<SymbolTab>,
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
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DebugInformationError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("not enough debug information to complete the request")]
    IncompleteInformation,
}

type Result<T> = std::result::Result<T, DebugInformationError>;

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
    fn get_units(&self) -> Result<&[Unit]> {
        self.units
            .as_deref()
            .ok_or(DebugInformationError::IncompleteInformation)
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

    fn evaluate_cfa(
        &self,
        debugee: &Debugee,
        registers: &DwarfRegisterMap,
        utr: &UnwindTableRow<EndianArcSlice>,
        ctx: &ExplorationContext,
    ) -> anyhow::Result<RelocatedAddress> {
        let rule = utr.cfa();
        match rule {
            RegisterAndOffset { register, offset } => {
                let ra = registers.value(*register)?;
                Ok(RelocatedAddress::from(ra as usize).offset(*offset as isize))
            }
            CfaRule::Expression(expr) => {
                let unit = debug_info_exists!(self.find_unit_by_pc(ctx.location().global_pc))
                    .ok_or_else(|| anyhow!("undefined unit"))?;
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
    ) -> anyhow::Result<RelocatedAddress> {
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
    pub fn known_files(&self) -> Result<impl Iterator<Item = &PathBuf>> {
        Ok(self.get_units()?.iter().flat_map(|unit| unit.files()))
    }

    /// Searches for a unit by occurrences of PC in its range.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter value
    ///
    /// returns: `None` if unit not found, error if no debug information found
    fn find_unit_by_pc(&self, pc: GlobalAddress) -> Result<Option<&Unit>> {
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
    pub fn find_place_from_pc(&self, pc: GlobalAddress) -> Result<Option<unit::PlaceDescriptor>> {
        let mb_unit = self.find_unit_by_pc(pc)?;
        Ok(mb_unit.and_then(|u| u.find_place_by_pc(pc)))
    }

    /// Returns place with line address equals to program counter global address.
    pub fn find_exact_place_from_pc(
        &self,
        pc: GlobalAddress,
    ) -> Result<Option<unit::PlaceDescriptor>> {
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
    ) -> Result<Option<ContextualDieRef<FunctionDie>>> {
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

    /// Return a function by its name.
    ///
    /// # Arguments
    ///
    /// * `needle`: function name.
    pub fn find_function_by_name(
        &self,
        needle: &str,
    ) -> Result<Option<ContextualDieRef<FunctionDie>>> {
        Ok(self.get_units()?.iter().find_map(|unit| {
            let mut entry_it = resolve_unit_call!(self.dwarf(), unit, entries_it);
            entry_it.find_map(|entry| {
                if let DieVariant::Function(func) = &entry.die {
                    if func.base_attributes.name.as_deref() == Some(needle) {
                        return Some(ContextualDieRef {
                            debug_info: self,
                            unit_idx: unit.idx(),
                            node: &entry.node,
                            die: func,
                        });
                    }
                }
                None
            })
        }))
    }

    pub fn find_place(&self, file: &str, line: u64) -> Result<Option<unit::PlaceDescriptor<'_>>> {
        Ok(self
            .get_units()?
            .iter()
            .find_map(|unit| unit.find_stmt_line(file, line)))
    }

    pub fn get_function_place(&self, fn_name: &str) -> Result<PlaceDescriptorOwned> {
        let func = self
            .find_function_by_name(fn_name)?
            .ok_or_else(|| anyhow!("function not found"))?;
        Ok(func.prolog_end_place()?.to_owned())
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
                let mb_unit = debug_info_exists!(self.get_units())
                    .binary_search_by_key(&Some(offset), |u| u.offset());
                let unit = match mb_unit {
                    Ok(_) | Err(0) => return None,
                    Err(pos) => self.unit_ensure(pos - 1),
                };
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
    ) -> Result<Vec<ContextualDieRef<'_, VariableDie>>> {
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

        // now check tls variables
        // for rust we expect that tls variable represents in dwarf like
        // variable with name "__KEY" and namespace like [.., variable_name, __getit]
        let tls_ns_part = &[name, "__getit"];
        for unit in units {
            let mb_var_locations = resolve_unit_call!(self.dwarf(), unit, locate_var_die, "__KEY");
            if let Some(vars) = mb_var_locations {
                vars.iter().for_each(|(namespaces, entry_idx)| {
                    if namespaces.contains(tls_ns_part) {
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
                });
            }
        }

        Ok(found)
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
    ) -> anyhow::Result<Option<(PathBuf, Mmap)>>
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
                bail!("invalid debug-id note format")
            }

            let dir = format!("{:x}", note[0]);
            let file = note[1..]
                .iter()
                .map(|&b| format!("{:x}", b))
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

    pub fn build(&self, obj_path: &Path, file: &object::File) -> anyhow::Result<DebugInformation> {
        let endian = if file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        let eh_frame = EhFrame::load(|id| -> gimli::Result<EndianArcSlice> {
            let data = file
                .section_by_name(id.name())
                .and_then(|section| section.uncompressed_data().ok())
                .unwrap_or(Cow::Borrowed(&[]));
            Ok(gimli::EndianArcSlice::new(Arc::from(&*data), endian))
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

        let parser = DwarfUnitParser::new(&dwarf);
        let headers = dwarf.units().collect::<Vec<_>>()?;

        if headers.is_empty() {
            // no units means no debug info
            info!(target: "dwarf-loader", "no debug information for {obj_path:?}");

            return Ok(DebugInformation {
                file: obj_path.to_path_buf(),
                inner: dwarf,
                eh_frame,
                bases,
                units: None,
                symbol_table,
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

        Ok(DebugInformation {
            file: obj_path.to_path_buf(),
            inner: dwarf,
            eh_frame,
            bases,
            units: Some(units),
            symbol_table,
        })
    }
}

pub trait AsAllocatedValue {
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

impl AsAllocatedValue for VariableDie {
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

impl AsAllocatedValue for ParameterDie {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NamespaceHierarchy(Vec<String>);

impl Deref for NamespaceHierarchy {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NamespaceHierarchy {
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

    pub fn contains(&self, needle: &[&str]) -> bool {
        self.0.windows(needle.len()).any(|slice| slice == needle)
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
        Self {
            debug_info: self.debug_info,
            unit_idx: self.unit_idx,
            node: self.node,
            die: self.die,
        }
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
    ) -> anyhow::Result<RelocatedAddress> {
        let attr = self
            .die
            .fb_addr
            .as_ref()
            .ok_or_else(|| anyhow!("no frame base attr"))?;

        let expr = DwarfLocation(attr)
            .try_as_expression(self.debug_info, self.unit(), ctx.location().global_pc)
            .ok_or_else(|| anyhow!("frame base attribute not an expression"))?;

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

    pub fn prolog_start_place(&self) -> anyhow::Result<unit::PlaceDescriptor> {
        let low_pc = self
            .die
            .base_attributes
            .ranges
            .iter()
            .min_by(|r1, r2| r1.begin.cmp(&r2.begin))
            .ok_or(anyhow!("function ranges not found"))?
            .begin;

        debug_info_exists!(self
            .debug_info
            .find_place_from_pc(GlobalAddress::from(low_pc)))
        .ok_or_else(|| anyhow!("invalid function entry"))
    }

    pub fn prolog_end_place(&self) -> anyhow::Result<unit::PlaceDescriptor> {
        let mut place = self.prolog_start_place()?;
        while !place.prolog_end {
            match place.next() {
                None => break,
                Some(next_place) => place = next_place,
            }
        }

        Ok(place)
    }

    pub fn prolog(&self) -> anyhow::Result<Range> {
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
    pub fn valid_at(&self, pc: GlobalAddress) -> bool {
        self.die
            .lexical_block_idx
            .map(|lb_idx| {
                let entry = ctx_resolve_unit_call!(self, entry, lb_idx);
                let DieVariant::LexicalBlock(lb) = &entry.die else {
                    unreachable!();
                };

                lb.base_attributes.ranges.iter().any(|r| pc.in_range(r))
            })
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

impl<'ctx, D: AsAllocatedValue> ContextualDieRef<'ctx, D> {
    pub fn r#type(&self) -> Option<ComplexType> {
        let parser = r#type::TypeParser::new();
        Some(parser.parse(*self, self.die.type_ref()?))
    }

    pub fn read_value(
        &self,
        ctx: &ExplorationContext,
        debugee: &Debugee,
        r#type: &ComplexType,
    ) -> Option<Bytes> {
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
                weak_error!(eval_result.into_raw_buffer(type_size, AddressKind::MemoryAddress))
            })
    }
}
