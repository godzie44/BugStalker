pub mod eval;
mod location;
pub mod parser;
mod symbol;
pub mod r#type;

use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::eval::ExpressionEvaluator;
use crate::debugger::debugee::dwarf::location::Location as DwarfLocation;
use crate::debugger::debugee::dwarf::parser::unit::{
    DieVariant, Entry, FunctionDie, Node, ParameterDie, Unit, VariableDie,
};
use crate::debugger::debugee::dwarf::parser::DieRef;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::r#type::EvaluationContext;
use crate::debugger::debugee::dwarf::symbol::SymbolTab;
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::debugger::utils::TryGetOrInsert;
use crate::debugger::{register, Place};
use crate::{debugger, weak_error};
use anyhow::anyhow;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use gimli::CfaRule::RegisterAndOffset;
use gimli::{
    Attribute, BaseAddresses, CfaRule, DebugAddr, DebugInfoOffset, Dwarf, EhFrame, Expression,
    LocationLists, Range, Register, RegisterRule, RunTimeEndian, Section, UnitOffset,
    UnwindContext, UnwindSection, UnwindTableRow,
};
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use smallvec::{smallvec, SmallVec};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::mem;
use std::ops::Deref;
use std::rc::Rc;
pub use symbol::Symbol;

pub type EndianRcSlice = gimli::EndianRcSlice<gimli::RunTimeEndian>;

#[derive(Default)]
pub struct DebugeeContextBuilder();

impl DebugeeContextBuilder {
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

        let dwarf = gimli::Dwarf::load(|id| Self::load_section(id, obj_file, endian))?;
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

        let parser = parser::DwarfUnitParser::new(&dwarf);

        let mut units = dwarf
            .units()
            .map(|header| parser.parse(dwarf.unit(header)?))
            .collect::<Vec<_>>()?;
        units.sort_unstable_by_key(|u| u.offset());

        Ok(DebugeeContext {
            inner: dwarf,
            eh_frame,
            bases,
            units,
            symbol_table,
        })
    }
}

pub struct RegisterDump(SmallVec<[Option<u64>; 0x80]>);

impl RegisterDump {
    pub fn get(&self, register: Register) -> Option<u64> {
        self.0.get(register.0 as usize).copied().and_then(|v| v)
    }
}

pub struct DebugeeContext<R: gimli::Reader = EndianRcSlice> {
    inner: Dwarf<R>,
    eh_frame: EhFrame<R>,
    bases: BaseAddresses,
    units: Vec<parser::unit::Unit>,
    symbol_table: Option<SymbolTab>,
}

impl DebugeeContext {
    pub fn locations(&self) -> &LocationLists<EndianRcSlice> {
        &self.inner.locations
    }

    fn evaluate_cfa(
        &self,
        debugee: &Debugee,
        utr: &UnwindTableRow<EndianRcSlice>,
        location: Location,
    ) -> anyhow::Result<RelocatedAddress> {
        let rule = utr.cfa();
        match rule {
            RegisterAndOffset { register, offset } => {
                let ra =
                    DwarfRegisterMap::from(RegisterMap::current(location.pid)?).value(*register)?;
                Ok(RelocatedAddress::from(ra as usize).offset(*offset as isize))
            }
            CfaRule::Expression(expr) => {
                let unit = self
                    .find_unit_by_pc(location.global_pc)
                    .ok_or_else(|| anyhow!("undefined unit"))?;
                let evaluator = unit.evaluator(debugee);
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
        self.evaluate_cfa(debugee, row, location)
    }

    pub fn registers(
        &self,
        debugee: &Debugee,
        location: Location,
        current_location: Location,
    ) -> anyhow::Result<RegisterDump> {
        let mut ctx = Box::new(UnwindContext::new());
        let row = self.eh_frame.unwind_info_for_address(
            &self.bases,
            &mut ctx,
            location.global_pc.into(),
            EhFrame::cie_from_offset,
        )?;

        let mut lazy_cfa = None;
        let cfa_init_fn = || self.evaluate_cfa(debugee, row, current_location);

        let mut lazy_evaluator = None;
        let evaluator_init_fn = || -> anyhow::Result<ExpressionEvaluator> {
            let unit = self
                .find_unit_by_pc(location.global_pc)
                .ok_or_else(|| anyhow!("undefined unit"))?;
            Ok(unit.evaluator(debugee))
        };

        let mut registers: SmallVec<[Option<u64>; 0x80]> = smallvec![None; 0x80];

        row.registers()
            .filter_map(|(register, rule)| {
                let value = match rule {
                    RegisterRule::Undefined => return None,
                    RegisterRule::SameValue => {
                        let register_map = weak_error!(RegisterMap::current(location.pid))?;
                        weak_error!(DwarfRegisterMap::from(register_map).value(*register))?
                    }
                    RegisterRule::Offset(offset) => {
                        let cfa = *weak_error!(lazy_cfa.try_get_or_insert_with(cfa_init_fn))?;
                        let addr = cfa.offset(*offset as isize);
                        let bytes = weak_error!(debugger::read_memory_by_pid(
                            location.pid,
                            addr.into(),
                            mem::size_of::<u64>()
                        ))?;
                        u64::from_ne_bytes(weak_error!(bytes
                            .try_into()
                            .map_err(|e| anyhow!("{e:?}")))?)
                    }
                    RegisterRule::ValOffset(offset) => {
                        let cfa = *weak_error!(lazy_cfa.try_get_or_insert_with(cfa_init_fn))?;
                        cfa.offset(*offset as isize).into()
                    }
                    RegisterRule::Register(reg) => {
                        let register_map = weak_error!(RegisterMap::current(location.pid))?;
                        weak_error!(DwarfRegisterMap::from(register_map).value(*reg))?
                    }
                    RegisterRule::Expression(expr) => {
                        let evaluator =
                            weak_error!(lazy_evaluator.try_get_or_insert_with(evaluator_init_fn))?;
                        let expr_result =
                            weak_error!(evaluator.evaluate(location.pid, expr.clone()))?;
                        let addr = weak_error!(expr_result.into_scalar::<usize>())?;
                        let bytes = weak_error!(debugger::read_memory_by_pid(
                            location.pid,
                            addr,
                            mem::size_of::<u64>()
                        ))?;
                        u64::from_ne_bytes(weak_error!(bytes
                            .try_into()
                            .map_err(|e| anyhow!("{e:?}")))?)
                    }
                    RegisterRule::ValExpression(expr) => {
                        let evaluator =
                            weak_error!(lazy_evaluator.try_get_or_insert_with(evaluator_init_fn))?;
                        let expr_result =
                            weak_error!(evaluator.evaluate(location.pid, expr.clone()))?;
                        weak_error!(expr_result.into_scalar::<u64>())?
                    }
                    RegisterRule::Architectural => return None,
                };

                Some((*register, value))
            })
            .for_each(|(reg, val)| registers.insert(reg.0 as usize, Some(val)));

        Ok(RegisterDump(registers))
    }

    pub fn debug_addr(&self) -> &DebugAddr<EndianRcSlice> {
        &self.inner.debug_addr
    }

    fn find_unit_by_pc(&self, pc: GlobalAddress) -> Option<&parser::unit::Unit> {
        self.units.iter().find(|&unit| {
            match unit.ranges.binary_search_by_key(&(pc.into()), |r| r.begin) {
                Ok(_) => true,
                Err(pos) => unit.ranges[..pos]
                    .iter()
                    .rev()
                    .any(|range| range.begin <= u64::from(pc) && u64::from(pc) < range.end),
            }
        })
    }

    /// Returns best matched place by program counter global address.
    pub fn find_place_from_pc(&self, pc: GlobalAddress) -> Option<parser::unit::Place> {
        let unit = self.find_unit_by_pc(pc)?;
        unit.find_place_by_pc(pc)
    }

    /// Returns place with line address equals to program counter global address.
    pub fn find_exact_place_from_pc(&self, pc: GlobalAddress) -> Option<parser::unit::Place> {
        let unit = self.find_unit_by_pc(pc)?;
        unit.find_exact_place_by_pc(pc)
    }

    pub fn find_function_by_pc(&self, pc: GlobalAddress) -> Option<ContextualDieRef<FunctionDie>> {
        let unit = self.find_unit_by_pc(pc)?;
        let pc = u64::from(pc);
        let find_pos = match unit
            .die_ranges
            .binary_search_by_key(&pc, |dr| dr.range.begin)
        {
            Ok(pos) => {
                let mut idx = pos + 1;
                while idx < unit.die_ranges.len() && unit.die_ranges[idx].range.begin == pc {
                    idx += 1;
                }
                idx
            }
            Err(pos) => pos,
        };

        unit.die_ranges[..find_pos].iter().rev().find_map(|dr| {
            if let DieVariant::Function(ref func) = unit.entries[dr.die_idx].die {
                if dr.range.begin <= pc && pc < dr.range.end {
                    return Some(ContextualDieRef {
                        context: self,
                        node: &unit.entries[dr.die_idx].node,
                        unit,
                        die: func,
                    });
                }
            };
            None
        })
    }

    pub fn find_function_by_name(&self, needle: &str) -> Option<ContextualDieRef<FunctionDie>> {
        self.units.iter().find_map(|unit| {
            unit.entries.iter().find_map(|entry| {
                if let DieVariant::Function(func) = &entry.die {
                    if func.base_attributes.name.as_deref() == Some(needle) {
                        return Some(ContextualDieRef {
                            context: self,
                            unit,
                            node: &entry.node,
                            die: func,
                        });
                    }
                }
                None
            })
        })
    }

    pub fn find_stmt_line(&self, file: &str, line: u64) -> Option<parser::unit::Place<'_>> {
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
            DieRef::Unit(offset) => default_unit.find_entry(offset).map(|e| (e, default_unit)),
            DieRef::Global(offset) => {
                let unit = match self
                    .units
                    .binary_search_by_key(&Some(offset), |u| u.offset())
                {
                    Ok(_) | Err(0) => return None,
                    Err(pos) => &self.units[pos - 1],
                };
                unit.find_entry(UnitOffset(
                    offset.0 - unit.offset().unwrap_or(DebugInfoOffset(0)).0,
                ))
                .map(|e| (e, unit))
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
            if let Some(vars) = unit.variable_index.get(name) {
                vars.iter().for_each(|(_, entry_idx)| {
                    if let DieVariant::Variable(ref var) = unit.entries[*entry_idx].die {
                        let variable = ContextualDieRef {
                            context: self,
                            unit,
                            node: &unit.entries[*entry_idx].node,
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
            if let Some(vars) = unit.variable_index.get("__KEY") {
                vars.iter().for_each(|(namespaces, entry_idx)| {
                    if namespaces.contains(tls_ns_part) {
                        if let DieVariant::Variable(ref var) = unit.entries[*entry_idx].die {
                            found.push(ContextualDieRef {
                                context: self,
                                unit,
                                node: &unit.entries[*entry_idx].node,
                                die: var,
                            });
                        }
                    }
                });
            }
        }

        found
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

#[derive(Clone, Debug, Default)]
pub struct NamespaceHierarchy(Vec<String>);

impl Deref for NamespaceHierarchy {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NamespaceHierarchy {
    pub fn for_node(node: &Node, unit: &Unit) -> Self {
        let mut ns_chain = vec![];

        let mut p_idx = node.parent;
        let mut next_parent = || -> Option<&Entry> {
            let parent = &unit.entries[p_idx?];
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
    pub unit: &'a Unit,
    pub node: &'a Node,
    pub die: &'a T,
}

impl<'a, T> Clone for ContextualDieRef<'a, T> {
    fn clone(&self) -> Self {
        Self {
            context: self.context,
            unit: self.unit,
            node: self.node,
            die: self.die,
        }
    }
}

impl<'a, T> Copy for ContextualDieRef<'a, T> {}

impl<'a, T> ContextualDieRef<'a, T> {
    pub fn namespaces(&self) -> NamespaceHierarchy {
        NamespaceHierarchy::for_node(self.node, self.unit)
    }
}

impl<'ctx> ContextualDieRef<'ctx, FunctionDie> {
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
            .try_as_expression(self.context, self.unit, pc)
            .ok_or_else(|| anyhow!("frame base attribute not an expression"))?;

        let result = self
            .unit
            .evaluator(debugee)
            .evaluate(pid, expr)?
            .into_scalar::<usize>()?;

        Ok(result.into())
    }

    pub fn local_variables<'this>(
        &'this self,
        pc: GlobalAddress,
    ) -> Vec<ContextualDieRef<'ctx, VariableDie>> {
        let mut result = vec![];
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            if let DieVariant::Variable(ref var) = self.unit.entries[idx].die {
                let var_ref = ContextualDieRef {
                    context: self.context,
                    unit: self.unit,
                    node: &self.unit.entries[idx].node,
                    die: var,
                };

                if var_ref.valid_at(pc) {
                    result.push(var_ref);
                }
            }
            self.unit.entries[idx]
                .node
                .children
                .iter()
                .for_each(|i| queue.push_back(*i));
        }
        result
    }

    pub fn parameters(&self) -> Vec<ContextualDieRef<'_, ParameterDie>> {
        let mut result = vec![];
        let mut queue = VecDeque::from(self.node.children.clone());
        while let Some(idx) = queue.pop_front() {
            if let DieVariant::Parameter(ref var) = self.unit.entries[idx].die {
                result.push(ContextualDieRef {
                    context: self.context,
                    unit: self.unit,
                    node: &self.unit.entries[idx].node,
                    die: var,
                })
            }
            self.unit.entries[idx]
                .node
                .children
                .iter()
                .for_each(|i| queue.push_back(*i));
        }
        result
    }

    pub fn prolog_start_place(&self) -> anyhow::Result<Place> {
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

    pub fn prolog_end_place(&self) -> anyhow::Result<Place> {
        let mut place = self.prolog_start_place()?;
        while !place.prolog_end {
            match place.next() {
                None => break,
                Some(next_place) => place = next_place,
            }
        }

        Ok(place)
    }
}

impl<'ctx> ContextualDieRef<'ctx, VariableDie> {
    pub fn valid_at(&self, pc: GlobalAddress) -> bool {
        self.die
            .lexical_block_idx
            .map(|lb_idx| {
                let DieVariant::LexicalBlock(lb) = &self.unit.entries[lb_idx].die else {
                    unreachable!();
                };

                lb.base_attributes
                    .ranges
                    .iter()
                    .any(|r| u64::from(pc) >= r.begin && u64::from(pc) < r.end)
            })
            .unwrap_or(true)
    }

    pub fn assume_parent_function(&self) -> Option<ContextualDieRef<'_, FunctionDie>> {
        let mut mb_parent = self.node.parent;

        while let Some(p) = mb_parent {
            if let DieVariant::Function(ref func) = self.unit.entries[p].die {
                return Some(ContextualDieRef {
                    context: self.context,
                    unit: self.unit,
                    node: &self.unit.entries[p].node,
                    die: func,
                });
            }

            mb_parent = self.unit.entries[p].node.parent;
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
            .location_expr(self.context, self.unit, location.global_pc)
            .and_then(|expr| {
                let evaluator = self.unit.evaluator(debugee);
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
