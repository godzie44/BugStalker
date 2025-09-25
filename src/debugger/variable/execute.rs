use crate::debugger::debugee::dwarf::DebugInformation;
use crate::debugger::debugee::dwarf::eval::{EvaluationContext, ExpressionEvaluator};
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::unit::BsUnit;
use crate::debugger::debugee::dwarf::unit::die_ref::{Argument, FatDieRef, Typed, Variable};
use crate::debugger::error::Error;
use crate::debugger::error::Error::FunctionNotFound;
use crate::debugger::variable::dqe::{DataCast, Dqe, PointerCast, Selector};
use crate::debugger::variable::value::Value;
use crate::debugger::variable::value::parser::{ParseContext, ValueModifiers, ValueParser};
use crate::debugger::variable::r#virtual::VirtualVariableDie;
use crate::debugger::variable::{Identity, ObjectBinaryRepr};
use crate::debugger::{Debugger, read_memory_by_pid};
use crate::{ref_resolve_unit_call, resolve_unit_call, weak_error};
use bytes::Bytes;
use gimli::Range;
use std::fmt::Debug;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryResultKind {
    /// Result value is an argument or variable
    Root,
    /// Result value calculated using DQE
    Expression,
}

/// Result of DQE evaluation.
#[derive(Clone)]
pub struct QueryResult<'a> {
    // TODO tmp pub
    pub value: Option<Value>,
    scope: Option<Box<[Range]>>,
    kind: QueryResultKind,
    base_type: Rc<ComplexType>,
    identity: Identity,
    evcx_builder: EvaluationContextBuilder<'a>,
}

impl QueryResult<'_> {
    /// Return CU in which result values are located.
    pub fn unit(&self) -> &BsUnit {
        self.evcx_builder.unit()
    }

    /// Return underlying typed value representation.
    #[inline(always)]
    pub fn value(&self) -> &Value {
        self.value.as_ref().expect("should be `Some`")
    }

    /// Return underlying typed value representation.
    #[inline(always)]
    pub fn into_value(mut self) -> Value {
        self.value.take().expect("should be `Some`")
    }

    /// Return underlying value and result identity (variable or argument identity).
    #[inline(always)]
    pub fn into_identified_value(mut self) -> (Identity, Value) {
        (self.identity, self.value.take().expect("should be `Some`"))
    }

    /// Return result kind:
    /// - `Root` kind means that value is an argument or variable
    /// - `Expression` kind means that value calculated using DQE
    #[inline(always)]
    pub fn kind(&self) -> QueryResultKind {
        self.kind
    }

    /// Return type graph using for parse a result.
    #[inline(always)]
    pub fn type_graph(&self) -> &ComplexType {
        self.base_type.as_ref()
    }

    /// Return result identity.
    #[inline(always)]
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Return variable or argument scope. Scope is a PC ranges where value is valid,
    /// `None` for global or virtual variables.
    #[inline(always)]
    pub fn scope(&self) -> &Option<Box<[Range]>> {
        &self.scope
    }

    /// Evaluate any function with evaluation context.
    pub fn with_evcx<T, F: FnOnce(&EvaluationContext) -> T>(&self, cb: F) -> T {
        self.evcx_builder.with_evcx(cb)
    }

    /// Modify the underlying value and return a new result extended from the current one.
    pub fn modify_value<F: FnOnce(&ParseContext, Value) -> Option<Value>>(
        mut self,
        cb: F,
    ) -> Option<Self> {
        let value = self.value.take().expect("should be `Some`");
        let type_graph = self.type_graph();
        let eval_cb = |evcx: &EvaluationContext| {
            let pcx = &ParseContext { evcx, type_graph };
            cb(pcx, value)
        };
        let new_value = self.evcx_builder.with_evcx(eval_cb)?;
        self.value = Some(new_value);
        Some(self)
    }
}

impl PartialEq for QueryResult<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.identity == other.identity
    }
}

#[derive(Clone)]
enum EvaluationContextBuilder<'a> {
    Ready(&'a Debugger, ExpressionEvaluator<'a>),
    Virtual {
        debugger: &'a Debugger,
        debug_info: &'a DebugInformation,
        unit: &'a BsUnit,
    },
}

impl EvaluationContextBuilder<'_> {
    pub fn unit(&self) -> &BsUnit {
        match self {
            EvaluationContextBuilder::Ready(_, evaluator) => evaluator.unit(),
            EvaluationContextBuilder::Virtual { unit, .. } => unit,
        }
    }

    fn with_evcx<T, F: FnOnce(&EvaluationContext) -> T>(&self, cb: F) -> T {
        let evaluator;
        let evcx = match self {
            EvaluationContextBuilder::Ready(debugger, evaluator) => EvaluationContext {
                evaluator,
                ecx: debugger.ecx(),
            },
            EvaluationContextBuilder::Virtual {
                debugger,
                debug_info,
                unit,
                ..
            } => {
                let dwarf = debug_info.dwarf();
                evaluator = resolve_unit_call!(
                    dwarf,
                    unit,
                    evaluator,
                    &debugger.debugee,
                    debug_info.dwarf()
                );
                EvaluationContext {
                    evaluator: &evaluator,
                    ecx: debugger.ecx(),
                }
            }
        };
        cb(&evcx)
    }
}

#[macro_export]
macro_rules! type_from_cache {
    ($variable: expr, $cache: expr) => {
        $variable
            .deref_ensure()
            .type_ref()
            .and_then(
                |type_ref| match $cache.entry(($variable.unit().id, type_ref)) {
                    std::collections::hash_map::Entry::Occupied(o) => {
                        Some(std::rc::Rc::clone(o.get()))
                    }
                    std::collections::hash_map::Entry::Vacant(v) => $variable.r#type().map(|t| {
                        let t = std::rc::Rc::new(t);
                        v.insert(t.clone());
                        t
                    }),
                },
            )
            .ok_or_else(|| {
                $crate::debugger::variable::value::ParsingError::Assume(
                    $crate::debugger::variable::value::AssumeError::NoType("variable"),
                )
            })
    };
}

/// Evaluate DQE at current location.
pub struct DqeExecutor<'a> {
    debugger: &'a Debugger,
}

impl<'dbg> DqeExecutor<'dbg> {
    pub fn new(debugger: &'dbg Debugger) -> Self {
        Self { debugger }
    }

    fn variable_die_by_selector(
        &self,
        selector: &Selector,
    ) -> Result<Vec<FatDieRef<'dbg, Variable>>, Error> {
        let ecx = self.debugger.ecx();

        let debugee = &self.debugger.debugee;
        let (current_func, _) = debugee
            .debug_info(ecx.location().pc)?
            .find_function_by_pc(ecx.location().global_pc)?
            .ok_or(FunctionNotFound(ecx.location().global_pc))?;

        let vars = match selector {
            Selector::Name {
                var_name,
                local_only: local,
            } => {
                let local_variants = current_func
                    .local_variable(ecx.location().global_pc, var_name)
                    .map(|v| vec![v])
                    .unwrap_or_default();

                let local = *local;

                // local variables is in priority anyway, if there are no local variables and
                // selector allow non-locals then try to search in a whole object
                if !local && local_variants.is_empty() {
                    debugee
                        .debug_info(ecx.location().pc)?
                        .find_variables(ecx.location(), var_name)?
                } else {
                    local_variants
                }
            }
            Selector::Any => current_func.local_variables(ecx.location().global_pc),
        };

        Ok(vars)
    }

    fn param_die_by_selector(
        &self,
        selector: &Selector,
    ) -> Result<Vec<FatDieRef<'dbg, Argument>>, Error> {
        let ecx_loc = self.debugger.ecx().location();
        let debugee = &self.debugger.debugee;
        let (current_function, _) = debugee
            .debug_info(ecx_loc.pc)?
            .find_function_by_pc(ecx_loc.global_pc)?
            .ok_or(FunctionNotFound(ecx_loc.global_pc))?;
        let params = current_function.parameters();
        let params = match selector {
            Selector::Name { var_name, .. } => params
                .into_iter()
                .filter(|r| r.deref_ensure().name().as_ref() == Some(var_name))
                .collect::<Vec<_>>(),
            Selector::Any => params,
        };
        Ok(params)
    }

    /// Select variables or arguments from debugee state.
    fn apply_select_die(
        &self,
        selector: &Selector,
        on_args: bool,
    ) -> Result<Vec<QueryResult<'dbg>>, Error> {
        fn root_from_die<'dbg, H: Typed>(
            debugger: &'dbg Debugger,
            die_ref: &FatDieRef<'dbg, H>,
            ranges: Option<Box<[Range]>>,
        ) -> Option<QueryResult<'dbg>> {
            let r#type = debugger
                .gcx()
                .with_type_cache(|tc| weak_error!(type_from_cache!(die_ref, tc)))?;

            let evaluator = ref_resolve_unit_call!(
                die_ref,
                evaluator,
                &debugger.debugee,
                die_ref.debug_info.dwarf()
            );
            let context_builder = EvaluationContextBuilder::Ready(debugger, evaluator);

            let value = context_builder.with_evcx(|evcx| {
                let data = die_ref.read_value(debugger.ecx(), &debugger.debugee, &r#type);

                let parser = ValueParser::new();
                let pcx = &ParseContext {
                    evcx,
                    type_graph: &r#type,
                };
                let modifiers = &ValueModifiers::from_identity(pcx, Identity::from_die(die_ref));
                parser.parse(pcx, data, modifiers)
            })?;

            Some(QueryResult {
                value: Some(value),
                scope: ranges,
                kind: QueryResultKind::Root,
                base_type: r#type,
                identity: Identity::from_die(die_ref),
                evcx_builder: context_builder,
            })
        }

        match on_args {
            true => {
                let params = self.param_die_by_selector(selector)?;
                Ok(params
                    .iter()
                    .filter_map(|arg_die| {
                        root_from_die(
                            self.debugger,
                            arg_die,
                            arg_die.max_range().map(|r| {
                                let scope: Box<[Range]> = Box::new([r]);
                                scope
                            }),
                        )
                    })
                    .collect())
            }
            false => {
                let vars = self.variable_die_by_selector(selector)?;
                Ok(vars
                    .iter()
                    .filter_map(|var_die| root_from_die(self.debugger, var_die, var_die.ranges()))
                    .collect())
            }
        }
    }

    /// Create virtual DIE from an existing type,
    /// then return a query result with a value from this DIE and address in debugee memory.
    fn apply_ptr_cast_op(&self, ptr_cast: &PointerCast) -> Result<QueryResult<'dbg>, Error> {
        let mut var_die = VirtualVariableDie::workpiece();
        let var_die_ref = var_die.init_with_type(&self.debugger.debugee, &ptr_cast.ty)?;

        let r#type = self
            .debugger
            .gcx()
            .with_type_cache(|tc| type_from_cache!(var_die_ref, tc))?;

        let context_builder = EvaluationContextBuilder::Virtual {
            debugger: self.debugger,
            debug_info: var_die_ref.debug_info,
            unit: var_die_ref.unit(),
        };

        let value = context_builder.with_evcx(|evcx| {
            let data = ObjectBinaryRepr {
                raw_data: Bytes::copy_from_slice(&ptr_cast.ptr.to_le_bytes()),
                address: None,
                size: std::mem::size_of::<usize>(),
            };

            let parser = ValueParser::new();
            let pcx = &ParseContext {
                evcx,
                type_graph: &r#type,
            };
            parser.parse(pcx, Some(data), &ValueModifiers::default())
        });

        Ok(QueryResult {
            value,
            scope: None,
            kind: QueryResultKind::Expression,
            base_type: r#type,
            identity: Identity::default(),
            evcx_builder: context_builder,
        })
    }

    /// Create virtual DIE from an existing type,
    /// then return a query result with a value from this DIE and address in debugee memory.
    fn apply_data_cast(&self, data_cast: &DataCast) -> Result<QueryResult<'dbg>, Error> {
        let mut var_die = VirtualVariableDie::workpiece();
        let debug_info = self
            .debugger
            .debugee
            .debug_info_from_file(&data_cast.ty_debug_info)?;
        let var_die_ref = var_die.init_with_known_type(
            debug_info,
            data_cast.ty_unit_off,
            data_cast.ty_die_off,
        )?;

        let r#type = self
            .debugger
            .gcx()
            .with_type_cache(|tc| type_from_cache!(var_die_ref, tc))?;

        let context_builder = EvaluationContextBuilder::Virtual {
            debugger: self.debugger,
            debug_info: var_die_ref.debug_info,
            unit: var_die_ref.unit(),
        };

        let value = context_builder.with_evcx(|evcx| {
            let size = r#type.type_size_in_bytes(evcx, r#type.root())? as usize;

            let raw_data = weak_error!(read_memory_by_pid(
                evcx.ecx.pid_on_focus(),
                data_cast.ptr,
                size
            ))?;

            let data = ObjectBinaryRepr {
                raw_data: Bytes::copy_from_slice(&raw_data),
                address: Some(data_cast.ptr),
                size,
            };

            let parser = ValueParser::new();
            let pcx = &ParseContext {
                evcx,
                type_graph: &r#type,
            };
            parser.parse(pcx, Some(data), &ValueModifiers::default())
        });

        Ok(QueryResult {
            value,
            scope: None,
            kind: QueryResultKind::Expression,
            base_type: r#type,
            identity: Identity::default(),
            evcx_builder: context_builder,
        })
    }

    fn apply_dqe(&self, dqe: &Dqe, on_args: bool) -> Result<Vec<QueryResult<'dbg>>, Error> {
        match dqe {
            Dqe::Variable(selector) => self.apply_select_die(selector, on_args),
            Dqe::PtrCast(ptr_cast) => self.apply_ptr_cast_op(ptr_cast).map(|q| vec![q]),
            Dqe::DataCast(data_cast) => self.apply_data_cast(data_cast).map(|q| vec![q]),
            Dqe::Field(next, field) => {
                let results = self.apply_dqe(next, on_args)?;
                Ok(results
                    .into_iter()
                    .filter_map(|q| q.modify_value(|_, val| val.field(field)))
                    .collect())
            }
            Dqe::Index(next, idx) => {
                let results = self.apply_dqe(next, on_args)?;
                Ok(results
                    .into_iter()
                    .filter_map(|q| q.modify_value(|_, val| val.index(idx)))
                    .collect())
            }
            Dqe::Slice(next, left, right) => {
                let results = self.apply_dqe(next, on_args)?;
                Ok(results
                    .into_iter()
                    .filter_map(|q| q.modify_value(|pcx, val| val.slice(pcx, *left, *right)))
                    .collect())
            }
            Dqe::Deref(next) => {
                let results = self.apply_dqe(next, on_args)?;
                Ok(results
                    .into_iter()
                    .filter_map(|q| q.modify_value(|pcx, val| val.deref(pcx)))
                    .collect())
            }
            Dqe::Address(next) => {
                let results = self.apply_dqe(next, on_args)?;
                Ok(results
                    .into_iter()
                    .filter_map(|q| q.modify_value(|pcx, val| val.address(pcx)))
                    .collect())
            }
            Dqe::Canonic(next) => {
                let results = self.apply_dqe(next, on_args)?;
                Ok(results
                    .into_iter()
                    .filter_map(|q| q.modify_value(|_, val| Some(val.canonic())))
                    .collect())
            }
        }
    }

    /// Query variables and returns matched list.
    pub fn query(&self, dqe: &Dqe) -> Result<Vec<QueryResult<'dbg>>, Error> {
        self.apply_dqe(dqe, false)
    }

    /// Query only variable names.
    /// Only filter expression supported.
    ///
    /// # Panics
    ///
    /// This method will panic if select expression
    /// contains any operators excluding a variable selector.
    pub fn query_names(&self, dqe: &Dqe) -> Result<Vec<String>, Error> {
        match dqe {
            Dqe::Variable(selector) => {
                let vars = self.variable_die_by_selector(selector)?;
                Ok(vars
                    .into_iter()
                    .filter_map(|die_ref| die_ref.deref_ensure().name())
                    .collect())
            }
            _ => unreachable!("unexpected expression variant"),
        }
    }

    /// Same as [`DqeExecutor::query`] but for function arguments.
    pub fn query_arguments(&self, dqe: &Dqe) -> Result<Vec<QueryResult<'dbg>>, Error> {
        self.apply_dqe(dqe, true)
    }

    /// Same as [`DqeExecutor::query_names`] but for function arguments.
    pub fn query_arguments_names(&self, dqe: &Dqe) -> Result<Vec<String>, Error> {
        match dqe {
            Dqe::Variable(selector) => {
                let params = self.param_die_by_selector(selector)?;
                Ok(params
                    .into_iter()
                    .filter_map(|r| r.deref_ensure().name())
                    .collect())
            }
            _ => unreachable!("unexpected expression variant"),
        }
    }
}
