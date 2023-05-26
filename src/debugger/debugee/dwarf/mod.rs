pub mod eval;
mod location;
mod symbol;
pub mod r#type;
pub mod unit;
pub mod unwind;

pub use self::unwind::DwarfUnwinder;

use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::location::Location as DwarfLocation;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::r#type::EvaluationContext;
use crate::debugger::debugee::dwarf::symbol::SymbolTab;
use crate::debugger::debugee::dwarf::unit::{
    DieRef, DieVariant, DwarfUnitParser, Entry, FunctionDie, Node, ParameterDie, Unit, VariableDie,
};
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::{resolve_unit_call, weak_error};
use anyhow::anyhow;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use gimli::CfaRule::RegisterAndOffset;
use gimli::{
    Attribute, BaseAddresses, CfaRule, DebugAddr, DebugInfoOffset, Dwarf, EhFrame, Expression,
    LocationLists, Range, RunTimeEndian, Section, UnitOffset, UnwindContext, UnwindSection,
    UnwindTableRow,
};
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::ops::Deref;
use std::rc::Rc;
pub use symbol::Symbol;

pub type EndianRcSlice = gimli::EndianRcSlice<gimli::RunTimeEndian>;

pub struct DebugeeContext<R: gimli::Reader = EndianRcSlice> {
    inner: Dwarf<R>,
    eh_frame: EhFrame<R>,
    bases: BaseAddresses,
    units: Vec<Unit>,
    symbol_table: Option<SymbolTab>,
}

impl DebugeeContext {
    pub fn locations(&self) -> &LocationLists<EndianRcSlice> {
        &self.inner.locations
    }

    fn evaluate_cfa(
        &self,
        debugee: &Debugee,
        registers: &DwarfRegisterMap,
        utr: &UnwindTableRow<EndianRcSlice>,
        location: Location,
    ) -> anyhow::Result<RelocatedAddress> {
        let rule = utr.cfa();
        match rule {
            RegisterAndOffset { register, offset } => {
                let ra = registers.value(*register)?;
                Ok(RelocatedAddress::from(ra as usize).offset(*offset as isize))
            }
            CfaRule::Expression(expr) => {
                let unit = self
                    .find_unit_by_pc(location.global_pc)
                    .ok_or_else(|| anyhow!("undefined unit"))?;
                let evaluator = resolve_unit_call!(&debugee.dwarf.inner, unit, evaluator, debugee);
                let expr_result = evaluator.evaluate(location.pid, expr.clone())?;

                Ok((expr_result.into_scalar::<usize>()?).into())
            }
        }
    }

    pub fn get_cfa(
        &self,
        debugee: &Debugee,
        location: Location,
    ) -> anyhow::Result<RelocatedAddress> {
        let mut ctx = Box::new(UnwindContext::new());
        let row = self.eh_frame.unwind_info_for_address(
            &self.bases,
            &mut ctx,
            location.global_pc.into(),
            EhFrame::cie_from_offset,
        )?;
        self.evaluate_cfa(
            debugee,
            &DwarfRegisterMap::from(RegisterMap::current(location.pid)?),
            row,
            location,
        )
    }

    pub fn debug_addr(&self) -> &DebugAddr<EndianRcSlice> {
        &self.inner.debug_addr
    }

    fn find_unit_by_pc(&self, pc: GlobalAddress) -> Option<&Unit> {
        self.units.iter().find(|&unit| {
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
        })
    }

    /// Returns best matched place by program counter global address.
    pub fn find_place_from_pc(&self, pc: GlobalAddress) -> Option<unit::Place> {
        let unit = self.find_unit_by_pc(pc)?;
        unit.find_place_by_pc(pc)
    }

    /// Returns place with line address equals to program counter global address.
    pub fn find_exact_place_from_pc(&self, pc: GlobalAddress) -> Option<unit::Place> {
        let unit = self.find_unit_by_pc(pc)?;
        unit.find_exact_place_by_pc(pc)
    }

    pub fn find_function_by_pc(&self, pc: GlobalAddress) -> Option<ContextualDieRef<FunctionDie>> {
        let unit = self.find_unit_by_pc(pc)?;
        let pc = u64::from(pc);
        let die_ranges = resolve_unit_call!(self.dwarf(), unit, die_ranges,);
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
                        context: self,
                        node: &entry.node,
                        unit_idx: unit.idx(),
                        die: func,
                    });
                }
            };
            None
        })
    }

    pub fn find_function_by_name(&self, needle: &str) -> Option<ContextualDieRef<FunctionDie>> {
        self.units.iter().find_map(|unit| {
            let mut entry_it = resolve_unit_call!(self.dwarf(), unit, entries_it,);
            entry_it.find_map(|entry| {
                if let DieVariant::Function(func) = &entry.die {
                    if func.base_attributes.name.as_deref() == Some(needle) {
                        return Some(ContextualDieRef {
                            context: self,
                            unit_idx: unit.idx(),
                            node: &entry.node,
                            die: func,
                        });
                    }
                }
                None
            })
        })
    }

    pub fn find_stmt_line(&self, file: &str, line: u64) -> Option<unit::Place<'_>> {
        self.units
            .iter()
            .find_map(|unit| unit.find_stmt_line(file, line))
    }

    pub fn find_symbol(&self, name: &str) -> Option<&Symbol> {
        self.symbol_table.as_ref().and_then(|table| table.get(name))
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
                let unit = match self
                    .units
                    .binary_search_by_key(&Some(offset), |u| u.offset())
                {
                    Ok(_) | Err(0) => return None,
                    Err(pos) => &self.units[pos - 1],
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
    ) -> Vec<ContextualDieRef<'_, VariableDie>> {
        let mut found = vec![];
        for unit in &self.units {
            let mb_var_locations = resolve_unit_call!(self.dwarf(), unit, locate_var_die, name);
            if let Some(vars) = mb_var_locations {
                vars.iter().for_each(|(_, entry_idx)| {
                    let entry = resolve_unit_call!(&self.inner, unit, entry, *entry_idx);
                    if let DieVariant::Variable(ref var) = entry.die {
                        let variable = ContextualDieRef {
                            context: self,
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
        for unit in &self.units {
            let mb_var_locations = resolve_unit_call!(self.dwarf(), unit, locate_var_die, "__KEY");
            if let Some(vars) = mb_var_locations {
                vars.iter().for_each(|(namespaces, entry_idx)| {
                    if namespaces.contains(tls_ns_part) {
                        let entry = resolve_unit_call!(&self.inner, unit, entry, *entry_idx);
                        if let DieVariant::Variable(ref var) = entry.die {
                            found.push(ContextualDieRef {
                                context: self,
                                unit_idx: unit.idx(),
                                node: &entry.node,
                                die: var,
                            });
                        }
                    }
                });
            }
        }

        found
    }

    pub fn dwarf(&self) -> &Dwarf<EndianRcSlice> {
        &self.inner
    }
}

#[derive(Default)]
pub struct DebugeeContextBuilder;

impl DebugeeContextBuilder {
    fn load_section<'a: 'b, 'b, OBJ, Endian>(
        id: gimli::SectionId,
        file: &'a OBJ,
        endian: Endian,
    ) -> anyhow::Result<gimli::EndianRcSlice<Endian>>
    where
        OBJ: Object<'a, 'b>,
        Endian: gimli::Endianity,
    {
        let data = file
            .section_by_name(id.name())
            .and_then(|section| section.uncompressed_data().ok())
            .unwrap_or(Cow::Borrowed(&[]));
        Ok(gimli::EndianRcSlice::new(Rc::from(&*data), endian))
    }

    pub fn build<'a, 'b, OBJ>(
        &self,
        obj_file: &'a OBJ,
    ) -> anyhow::Result<DebugeeContext<EndianRcSlice>>
    where
        'a: 'b,
        OBJ: Object<'a, 'b>,
    {
        let endian = if obj_file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        let dwarf = Dwarf::load(|id| Self::load_section(id, obj_file, endian))?;
        let symbol_table = SymbolTab::new(obj_file);
        let eh_frame = EhFrame::load(|id| Self::load_section(id, obj_file, endian))?;

        let section_addr = |name: &str| -> Option<u64> {
            obj_file.sections().find_map(|section| {
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

        let parser = DwarfUnitParser::new(&dwarf);

        let mut units = dwarf
            .units()
            .map(|header| {
                let unit = parser.parse(header)?;
                Ok(unit)
            })
            .collect::<Vec<_>>()?;
        units.sort_unstable_by_key(|u| u.offset());
        units.iter_mut().enumerate().for_each(|(i, u)| u.set_idx(i));

        Ok(DebugeeContext {
            inner: dwarf,
            eh_frame,
            bases,
            units,
            symbol_table,
        })
    }
}

pub trait AsAllocatedValue {
    fn name(&self) -> Option<&str>;

    fn type_ref(&self) -> Option<DieRef>;

    fn location(&self) -> Option<&Attribute<EndianRcSlice>>;

    fn location_expr(
        &self,
        dwarf_ctx: &DebugeeContext<EndianRcSlice>,
        unit: &Unit,
        pc: GlobalAddress,
    ) -> Option<Expression<EndianRcSlice>> {
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

    fn location(&self) -> Option<&Attribute<EndianRcSlice>> {
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

    fn location(&self) -> Option<&Attribute<EndianRcSlice>> {
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
    pub context: &'a DebugeeContext,
    // pub unit: &'a Unit,
    pub unit_idx: usize,
    pub node: &'a Node,
    pub die: &'a T,
}

#[macro_export]
macro_rules! ctx_resolve_unit_call {
    ($self: ident, $fn_name: tt, $($arg: expr),*) => {{
        $crate::resolve_unit_call!($self.context.dwarf(), $self.unit(), $fn_name, $($arg),*)
    }};
}

impl<'a, T> Clone for ContextualDieRef<'a, T> {
    fn clone(&self) -> Self {
        Self {
            context: self.context,
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
        &self.context.units[self.unit_idx]
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
        pid: Pid,
        debugee: &Debugee,
        pc: GlobalAddress,
    ) -> anyhow::Result<RelocatedAddress> {
        let attr = self
            .die
            .fb_addr
            .as_ref()
            .ok_or_else(|| anyhow!("no frame base attr"))?;

        let expr = DwarfLocation(attr)
            .try_as_expression(self.context, self.unit(), pc)
            .ok_or_else(|| anyhow!("frame base attribute not an expression"))?;

        let evaluator = ctx_resolve_unit_call!(self, evaluator, debugee);
        let result = evaluator.evaluate(pid, expr)?.into_scalar::<usize>()?;
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
                    context: self.context,
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
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            let entry = ctx_resolve_unit_call!(self, entry, idx);
            if let DieVariant::Parameter(ref var) = entry.die {
                result.push(ContextualDieRef {
                    context: self.context,
                    unit_idx: self.unit_idx,
                    node: &entry.node,
                    die: var,
                })
            }
            entry.node.children.iter().for_each(|i| queue.push_back(*i));
        }
        result
    }

    pub fn prolog_start_place(&self) -> anyhow::Result<unit::Place> {
        let low_pc = self
            .die
            .base_attributes
            .ranges
            .iter()
            .min_by(|r1, r2| r1.begin.cmp(&r2.begin))
            .ok_or(anyhow!("function ranges not found"))?
            .begin;
        self.context
            .find_place_from_pc(GlobalAddress::from(low_pc))
            .ok_or_else(|| anyhow!("invalid function entry"))
    }

    pub fn prolog_end_place(&self) -> anyhow::Result<unit::Place> {
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
                    context: self.context,
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
        location: Location,
        debugee: &Debugee,
        r#type: &ComplexType,
    ) -> Option<Bytes> {
        self.die
            .location_expr(self.context, self.unit(), location.global_pc)
            .and_then(|expr| {
                let evaluator = ctx_resolve_unit_call!(self, evaluator, debugee);
                let eval_result = weak_error!(evaluator.evaluate(location.pid, expr))?;
                weak_error!(eval_result.into_raw_buffer(r#type.type_size_in_bytes(
                    &EvaluationContext {
                        evaluator: &evaluator,
                        pid: location.pid,
                    },
                    r#type.root
                )? as usize))
            })
    }
}
