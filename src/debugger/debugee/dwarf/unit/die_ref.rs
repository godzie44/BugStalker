use crate::debugger::ExplorationContext;
use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::Debugee;
use crate::debugger::debugee::dwarf::eval::AddressKind;
use crate::debugger::debugee::dwarf::eval::EvaluationContext;
use crate::debugger::debugee::dwarf::location::Location as DwarfLocation;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::unit::die::{DerefContext, Die};
use crate::debugger::debugee::dwarf::unit::{BsUnit, PlaceDescriptor};
use crate::debugger::debugee::dwarf::{DebugInformation, NamespaceHierarchy, r#type};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{
    FBANotAnExpression, FunctionNotFound, NoFBA, NoFunctionRanges,
};
use crate::debugger::variable::ObjectBinaryRepr;
use crate::debugger::variable::r#virtual::VirtualVariableDie;
use crate::{debug_info_exists, ref_resolve_unit_call, weak_error};
use gimli::{DW_TAG_lexical_block, DW_TAG_subprogram, Range, UnitOffset};
use indexmap::IndexMap;
use std::marker::PhantomData;

#[derive(Clone, Copy)]
pub enum DieReference {
    Offset(UnitOffset),
    Virtual(VirtualVariableDie),
}

pub trait Hint {}

pub trait Typed: Hint {}

pub struct NoHint;
impl Hint for NoHint {}

pub struct Function;
impl Hint for Function {}

pub struct Argument;
impl Hint for Argument {}
impl Typed for Argument {}

pub struct Variable;
impl Hint for Variable {}
impl Typed for Variable {}

/// Reference to debug information entry. Can be dereferenced without external dependencies.
pub struct FatDieRef<'dbg, H: Hint = NoHint> {
    pub debug_info: &'dbg DebugInformation,
    unit_idx: usize,
    reference: DieReference,
    _hint: PhantomData<H>,
}

#[macro_export]
macro_rules! ref_resolve_unit_call {
    ($self: ident, $fn_name: tt, $($arg: expr),*) => {{
        $crate::resolve_unit_call!($self.debug_info.dwarf(), $self.unit(), $fn_name, $($arg),*)
    }};
}

impl<'dbg> FatDieRef<'dbg> {
    pub fn new<H: Hint>(
        debug_info: &'dbg DebugInformation,
        unit_idx: usize,
        die_off: DieReference,
    ) -> FatDieRef<'dbg, H> {
        FatDieRef {
            debug_info,
            unit_idx,
            reference: die_off,
            _hint: PhantomData,
        }
    }

    pub fn new_no_hint(
        debug_info: &'dbg DebugInformation,
        unit_idx: usize,
        offset: UnitOffset,
    ) -> Self {
        Self::new::<NoHint>(debug_info, unit_idx, DieReference::Offset(offset))
    }

    pub fn new_func(
        debug_info: &'dbg DebugInformation,
        unit_idx: usize,
        offset: UnitOffset,
    ) -> FatDieRef<'dbg, Function> {
        Self::new::<Function>(debug_info, unit_idx, DieReference::Offset(offset))
    }

    pub fn new_arg(
        debug_info: &'dbg DebugInformation,
        unit_idx: usize,
        offset: UnitOffset,
    ) -> FatDieRef<'dbg, Argument> {
        Self::new::<Argument>(debug_info, unit_idx, DieReference::Offset(offset))
    }

    pub fn new_var(
        debug_info: &'dbg DebugInformation,
        unit_idx: usize,
        offset: UnitOffset,
    ) -> FatDieRef<'dbg, Variable> {
        Self::new::<Variable>(debug_info, unit_idx, DieReference::Offset(offset))
    }

    pub fn new_virt_var(
        debug_info: &'dbg DebugInformation,
        unit_idx: usize,
        die: VirtualVariableDie,
    ) -> FatDieRef<'dbg, Variable> {
        Self::new::<Variable>(debug_info, unit_idx, DieReference::Virtual(die))
    }

    pub fn with_new_hint<NH: Hint>(self) -> FatDieRef<'dbg, NH> {
        FatDieRef {
            debug_info: self.debug_info,
            unit_idx: self.unit_idx,
            reference: self.reference,
            _hint: Default::default(),
        }
    }
}

impl<'dbg, H: Hint> Clone for FatDieRef<'dbg, H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'dbg, H: Hint> Copy for FatDieRef<'dbg, H> {}

impl<'dbg, H: Hint> FatDieRef<'dbg, H> {
    #[inline(always)]
    pub fn unit(&self) -> &'dbg BsUnit {
        self.debug_info.unit_ensure(self.unit_idx)
    }

    pub fn dcx(&'_ self) -> DerefContext<'_, '_> {
        DerefContext::new(self.debug_info.dwarf(), self.unit().unit())
    }

    pub fn deref(&self) -> Result<Die<'dbg>, Error> {
        let die = match self.reference {
            DieReference::Offset(unit_offset) => Die::Dwarf {
                dcx: DerefContext::new(self.debug_info.dwarf(), self.unit().unit()),
                die: self
                    .unit()
                    .unit()
                    .entry(unit_offset)
                    .map_err(|_| Error::DieNotFound(super::DieAddr::Unit(unit_offset)))?,
            },
            DieReference::Virtual(virtual_variable_die) => Die::Virtual {
                type_ref: Some(virtual_variable_die.type_ref),
            },
        };
        Ok(die)
    }

    pub fn deref_ensure(&self) -> Die<'dbg> {
        self.deref().expect("DIE should exist")
    }

    pub fn namespace(&self) -> NamespaceHierarchy {
        let parent_index = ref_resolve_unit_call!(self, parent_index,);

        let DieReference::Offset(offset) = self.reference else {
            unimplemented!("virtual die are unsupported")
        };

        NamespaceHierarchy::for_die(self.dcx(), offset, parent_index)
    }
}

impl<'dbg> FatDieRef<'dbg, Argument> {
    /// Return max range (with max `end` address) of an underlying function.
    /// If it's possible, `end` address in range equals to function epilog begin.
    pub fn max_range(&self) -> Option<Range> {
        let fn_block_die = {
            let die = weak_error!(self.deref())?;

            let mut fn_block = None;
            let parent_index = ref_resolve_unit_call!(self, parent_index,);

            let mut parent_offset = parent_index.get(&die.offset()).copied();
            while let Some(off) = parent_offset {
                let die = weak_error!(Die::new(self.dcx(), off))?;
                if die.tag() == DW_TAG_subprogram {
                    fn_block = Some(die);
                    break;
                }
                parent_offset = parent_index.get(&die.offset()).copied();
            }

            fn_block
        }?;

        let ranges = fn_block_die.ranges();
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
    }
}

impl<'dbg> FatDieRef<'dbg, Variable> {
    pub fn ranges(&self) -> Option<Box<[Range]>> {
        let die = weak_error!(self.deref())?;

        let parent_index: &IndexMap<UnitOffset, UnitOffset> =
            ref_resolve_unit_call!(self, parent_index,);
        let mut parent_offset = parent_index.get(&die.offset()).copied();
        while let Some(off) = parent_offset {
            let die = weak_error!(Die::new(self.dcx(), off))?;
            if die.tag() == DW_TAG_lexical_block || die.tag() == DW_TAG_subprogram {
                return Some(die.ranges());
            }
            parent_offset = parent_index.get(&die.offset()).copied();
        }

        None
    }

    pub fn valid_at(&self, pc: GlobalAddress) -> bool {
        self.ranges()
            .map(|ranges| pc.in_ranges(&ranges))
            .unwrap_or(true)
    }
}

impl<'dbg, H: Typed> FatDieRef<'dbg, H> {
    pub fn r#type(&self) -> Option<ComplexType> {
        let parser = r#type::TypeParser::new();
        let die_type_ref = weak_error!(self.deref())?.type_ref()?;
        Some(parser.parse(*self, die_type_ref))
    }

    pub fn read_value(
        &self,
        ecx: &ExplorationContext,
        debugee: &Debugee,
        r#type: &ComplexType,
    ) -> Option<ObjectBinaryRepr> {
        let die = weak_error!(self.deref())?;
        let location = die.location()?;
        let location_expr = DwarfLocation(&location).try_as_expression(
            self.debug_info,
            self.unit(),
            ecx.location().global_pc,
        );

        location_expr.and_then(|expr| {
            let evaluator =
                ref_resolve_unit_call!(self, evaluator, debugee, self.debug_info.dwarf());
            let eval_result = weak_error!(evaluator.evaluate(ecx, expr))?;
            let type_size = r#type.type_size_in_bytes(
                &EvaluationContext {
                    evaluator: &evaluator,
                    ecx,
                },
                r#type.root(),
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

impl<'dbg> FatDieRef<'dbg, Function> {
    pub fn frame_base_addr(
        &self,
        ecx: &ExplorationContext,
        debugee: &Debugee,
    ) -> Result<RelocatedAddress, Error> {
        let attr = self.deref()?.frame_base().ok_or(NoFBA)?;

        let expr = DwarfLocation(&attr)
            .try_as_expression(self.debug_info, self.unit(), ecx.location().global_pc)
            .ok_or(FBANotAnExpression)?;

        let evaluator = ref_resolve_unit_call!(self, evaluator, debugee, self.debug_info.dwarf());
        let result = evaluator
            .evaluate(ecx, expr)?
            .into_scalar::<usize>(AddressKind::Value)?;
        Ok(result.into())
    }

    pub fn local_variables<'this>(
        &'this self,
        pc: GlobalAddress,
    ) -> Vec<FatDieRef<'dbg, Variable>> {
        let Some(die) = weak_error!(self.deref()) else {
            return vec![];
        };

        let mut result = vec![];

        die.for_each_children_recursive(|child| {
            if child.tag() == gimli::DW_TAG_variable {
                let var_ref = FatDieRef {
                    debug_info: self.debug_info,
                    unit_idx: self.unit_idx,
                    reference: DieReference::Offset(child.offset()),
                    _hint: Default::default(),
                };

                if var_ref.valid_at(pc) {
                    result.push(var_ref);
                }
            }
        });

        result
    }

    pub fn local_variable<'this>(
        &'this self,
        pc: GlobalAddress,
        needle: &str,
    ) -> Option<FatDieRef<'dbg, Variable>> {
        weak_error!(self.deref())?.for_each_children_recursive_t(|child| {
            if child.tag() == gimli::DW_TAG_variable {
                let var_ref = FatDieRef::new_var(self.debug_info, self.unit_idx, child.offset());

                if child.name().as_deref() == Some(needle) && var_ref.valid_at(pc) {
                    return Some(var_ref);
                }
            }
            None
        })
    }

    pub fn parameters(&self) -> Vec<FatDieRef<'dbg, Argument>> {
        let Some(die) = weak_error!(self.deref()) else {
            return vec![];
        };

        die.for_each_children_filter_collect(|child| {
            if child.tag() == gimli::DW_TAG_formal_parameter {
                Some(FatDieRef::new_arg(
                    self.debug_info,
                    self.unit_idx,
                    child.offset(),
                ))
            } else {
                None
            }
        })
    }

    /// Return function first instruction address.
    /// Address computed from function ranges if ranges is empty.
    pub fn start_instruction(&self) -> Result<GlobalAddress, Error> {
        Ok(self
            .ranges()
            .iter()
            .min_by(|r1, r2| r1.begin.cmp(&r2.begin))
            .ok_or_else(|| {
                let name = self.deref().map(|d| d.name()).unwrap_or_default();
                NoFunctionRanges(name)
            })?
            .begin
            .into())
    }

    /// Return address of the first location past the last instruction associated with the function.
    pub fn end_instruction(&self) -> Result<GlobalAddress, Error> {
        Ok(self
            .ranges()
            .iter()
            .max_by(|r1, r2| r1.begin.cmp(&r2.begin))
            .ok_or_else(|| {
                let name = self.deref().map(|d| d.name()).unwrap_or_default();
                NoFunctionRanges(name)
            })?
            .end
            .into())
    }

    pub fn prolog_start_place(&self) -> Result<PlaceDescriptor<'_>, Error> {
        let low_pc = self.start_instruction()?;

        debug_info_exists!(self.debug_info.find_place_from_pc(low_pc))
            .ok_or(FunctionNotFound(low_pc))
    }

    pub fn prolog_end_place(&self) -> Result<PlaceDescriptor<'_>, Error> {
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

    pub fn ranges(&self) -> Box<[Range]> {
        let Some(die) = weak_error!(self.deref()) else {
            return Box::default();
        };
        die.ranges()
    }

    pub fn inline_ranges(&self) -> Vec<Range> {
        let Some(die) = weak_error!(self.deref()) else {
            return vec![];
        };

        let mut ranges = vec![];

        die.for_each_children_recursive(|child| {
            if child.tag() == gimli::DW_TAG_inlined_subroutine {
                ranges.extend(child.ranges());
            }
        });

        ranges
    }

    /// Return template parameter die by its name.
    ///
    /// # Arguments
    ///
    /// * `name`: tpl parameter name
    pub fn get_template_parameter(&self, name: &str) -> Option<Die<'dbg>> {
        weak_error!(self.deref())?.for_each_children_t(|child| {
            (child.tag() == gimli::DW_TAG_template_type_parameter
                && child.name().as_deref() == Some(name))
            .then_some(child)
        })
    }
}
