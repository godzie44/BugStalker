use crate::debugger::debugee::dwarf;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::unit::{DieRef, Node, VariableDie};
use crate::debugger::debugee::dwarf::{
    AsAllocatedData, ContextualDieRef, EndianArcSlice, NamespaceHierarchy,
};
use crate::debugger::error::Error;
use crate::debugger::error::Error::FunctionNotFound;
use crate::debugger::variable::{AssumeError, ParsingError, VariableIR, VariableIdentity};
use crate::debugger::Error::TypeNotFound;
use crate::debugger::{variable, Debugger};
use crate::{ctx_resolve_unit_call, weak_error};
use bytes::Bytes;
use gimli::{Attribute, DebugInfoOffset, Range, UnitOffset};
use std::collections::hash_map::Entry;
use std::collections::HashMap;

/// This die not exists in debug information.
/// It may be used to represent variables that are
/// declared by user, for example, using pointer cast operator.
struct VirtualVariableDie {
    type_ref: DieRef,
}

impl VirtualVariableDie {
    fn of_unknown_type() -> Self {
        Self {
            type_ref: DieRef::Unit(UnitOffset(0)),
        }
    }
}

impl AsAllocatedData for VirtualVariableDie {
    fn name(&self) -> Option<&str> {
        None
    }

    fn type_ref(&self) -> Option<DieRef> {
        Some(self.type_ref)
    }

    fn location(&self) -> Option<&Attribute<EndianArcSlice>> {
        None
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum VariableSelector {
    Name { var_name: String, only_local: bool },
    Any,
}

impl VariableSelector {
    pub fn by_name(name: &str, only_local: bool) -> Self {
        Self::Name {
            var_name: name.to_string(),
            only_local,
        }
    }
}

/// Literal object. Using it for a searching element by key in key-value containers.
#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    String(String),
    Int(i64),
    Float(f64),
    Address(usize),
    Bool(bool),
    EnumVariant(String, Option<Box<Literal>>),
    Array(Box<[LiteralOrWildcard]>),
    AssocArray(HashMap<String, LiteralOrWildcard>),
}

#[derive(Debug, PartialEq, Clone)]
pub enum LiteralOrWildcard {
    Literal(Literal),
    Wildcard,
}

macro_rules! impl_equal {
    ($lhs: expr, $rhs: expr, $lit: path) => {
        if let $lit(lhs) = $lhs {
            lhs == &$rhs
        } else {
            false
        }
    };
}

impl Literal {
    pub fn equal_with_string(&self, rhs: &str) -> bool {
        impl_equal!(self, rhs, Literal::String)
    }

    pub fn equal_with_address(&self, rhs: usize) -> bool {
        impl_equal!(self, rhs, Literal::Address)
    }

    pub fn equal_with_bool(&self, rhs: bool) -> bool {
        impl_equal!(self, rhs, Literal::Bool)
    }

    pub fn equal_with_int(&self, rhs: i64) -> bool {
        impl_equal!(self, rhs, Literal::Int)
    }

    pub fn equal_with_float(&self, rhs: f64) -> bool {
        const EPS: f64 = 0.0000001f64;
        if let Literal::Float(float) = self {
            let diff = (*float - rhs).abs();
            diff < EPS
        } else {
            false
        }
    }
}

/// Object binary representation in debugee memory.
pub struct ObjectBinaryRepr {
    /// Binary representation.
    pub raw_data: Bytes,
    /// Possible address of object data in debugee memory.
    /// It may not exist if there is no debug information, or if an object is allocated in registers.
    pub address: Option<usize>,
    /// Binary size.
    pub size: usize,
}

/// Data query expression.
/// List of operations for select variables and their properties.
///
/// Expression can be parsed from an input string like `*(*variable1.field2)[1]`
/// (see [`crate::ui::command`] module)
///
/// Supported operations are: dereference, get an element by index, get field by name, make slice from a pointer.
#[derive(Debug, PartialEq, Clone)]
pub enum DQE {
    Variable(VariableSelector),
    PtrCast(usize, String),
    Field(Box<DQE>, String),
    Index(Box<DQE>, Literal),
    Slice(Box<DQE>, Option<usize>, Option<usize>),
    Deref(Box<DQE>),
    Address(Box<DQE>),
    Canonic(Box<DQE>),
}

impl DQE {
    /// Return boxed expression.
    pub fn boxed(self) -> Box<Self> {
        Box::new(self)
    }
}

/// Result of DQE evaluation.
pub struct DqeResult {
    /// Variable intermediate representation.
    pub variable: VariableIR,
    /// PC ranges where value is valid, `None` for global or virtual variables.
    pub scope: Option<Box<[Range]>>,
}

/// Evaluate `Expression` at current breakpoint (for current debugee location).
pub struct SelectExpressionEvaluator<'a> {
    debugger: &'a Debugger,
    expression: DQE,
}

macro_rules! type_from_cache {
    ($variable: expr, $cache: expr) => {
        $variable
            .die
            .type_ref()
            .and_then(
                |type_ref| match $cache.entry(($variable.unit().id, type_ref)) {
                    Entry::Occupied(o) => Some(&*o.into_mut()),
                    Entry::Vacant(v) => $variable.r#type().map(|t| &*v.insert(t)),
                },
            )
            .ok_or_else(|| ParsingError::Assume(AssumeError::NoType("variable")))
    };
}

impl<'a> SelectExpressionEvaluator<'a> {
    pub fn new(debugger: &'a Debugger, expression: DQE) -> Self {
        Self {
            debugger,
            expression,
        }
    }

    fn extract_variable_by_selector(
        &self,
        selector: &VariableSelector,
    ) -> Result<Vec<ContextualDieRef<VariableDie>>, Error> {
        let ctx = self.debugger.exploration_ctx();

        let debugee = &self.debugger.debugee;
        let current_func = debugee
            .debug_info(ctx.location().pc)?
            .find_function_by_pc(ctx.location().global_pc)?
            .ok_or(FunctionNotFound(ctx.location().global_pc))?;

        let vars = match selector {
            VariableSelector::Name {
                var_name,
                only_local: local,
            } => {
                let local_variants = current_func
                    .local_variable(ctx.location().global_pc, var_name)
                    .map(|v| vec![v])
                    .unwrap_or_default();

                let local = *local;

                // local variables is in priority anyway, if there are no local variables and
                // selector allow non-locals then try to search in a whole object
                if !local && local_variants.is_empty() {
                    debugee
                        .debug_info(ctx.location().pc)?
                        .find_variables(ctx.location(), var_name)?
                } else {
                    local_variants
                }
            }
            VariableSelector::Any => current_func.local_variables(ctx.location().global_pc),
        };

        Ok(vars)
    }

    fn fill_virtual_ptr_variable(
        &self,
        vv: &'a mut VirtualVariableDie,
        node: &'a Node,
        type_name: &str,
    ) -> Result<ContextualDieRef<'a, VirtualVariableDie>, Error> {
        let debugee = &self.debugger.debugee;
        let (debug_info, offset_of_unit, offset_of_die) = debugee
            .debug_info_all()
            .iter()
            .find_map(|&debug_info| {
                let (offset_of_unit, offset_of_die) = debug_info.find_type_die_ref(type_name)?;
                Some((debug_info, offset_of_unit, offset_of_die))
            })
            .ok_or(TypeNotFound)?;
        let unit = debug_info
            .find_unit(DebugInfoOffset(offset_of_unit.0 + offset_of_die.0))
            .ok_or(TypeNotFound)?;

        vv.type_ref = DieRef::Unit(offset_of_die);
        let var = ContextualDieRef {
            debug_info,
            unit_idx: unit.idx(),
            node,
            die: vv,
        };

        Ok(var)
    }

    /// Evaluate only variable names.
    /// Only filter expression supported.
    ///
    /// # Panics
    ///
    /// This method will panic
    /// if select expression contains any operators excluding a variable selector.
    pub fn evaluate_names(&self) -> Result<Vec<String>, Error> {
        match &self.expression {
            DQE::Variable(selector) => {
                let vars = self.extract_variable_by_selector(selector)?;
                Ok(vars
                    .into_iter()
                    .filter_map(|die| die.die.name().map(ToOwned::to_owned))
                    .collect())
            }
            _ => unreachable!("unexpected expression variant"),
        }
    }

    fn evaluate_inner(&self, expression: &DQE) -> Result<Vec<DqeResult>, Error> {
        // evaluate variable one by one in `evaluate_single_variable` method
        // here just filter variables
        match expression {
            DQE::Variable(selector) => {
                let vars = self.extract_variable_by_selector(selector)?;
                let mut type_cache = self.debugger.type_cache.borrow_mut();

                Ok(vars
                    .iter()
                    .filter_map(|var| {
                        let r#type = weak_error!(type_from_cache!(var, type_cache))?;
                        let var_ir =
                            self.evaluate_single_variable(&self.expression, var, r#type)?;
                        Some(DqeResult {
                            variable: var_ir,
                            scope: var.ranges().map(Box::from),
                        })
                    })
                    .collect())
            }
            DQE::PtrCast(_, target_type_name) => {
                let vars_ir = self.evaluate_from_ptr_cast(target_type_name)?;
                Ok(vars_ir
                    .into_iter()
                    .map(|var_ir| DqeResult {
                        variable: var_ir,
                        scope: None,
                    })
                    .collect())
            }
            DQE::Field(expr, _)
            | DQE::Index(expr, _)
            | DQE::Slice(expr, _, _)
            | DQE::Deref(expr)
            | DQE::Address(expr)
            | DQE::Canonic(expr) => self.evaluate_inner(expr),
        }
    }

    /// Create virtual DIE from type name and constant address. Evaluate expression then on this DIE.
    fn evaluate_from_ptr_cast(&self, type_name: &str) -> Result<Vec<VariableIR>, Error> {
        let any_node = Node::new_leaf(None);
        let mut var_die = VirtualVariableDie::of_unknown_type();
        let var_die_ref = self.fill_virtual_ptr_variable(&mut var_die, &any_node, type_name)?;

        let mut type_cache = self.debugger.type_cache.borrow_mut();
        let r#type = type_from_cache!(var_die_ref, type_cache)?;

        if let Some(v) = self.evaluate_single_variable(&self.expression, &var_die_ref, r#type) {
            return Ok(vec![v]);
        }
        Ok(vec![])
    }

    /// Evaluate a select expression and returns list of matched variables.
    pub fn evaluate(&self) -> Result<Vec<DqeResult>, Error> {
        self.evaluate_inner(&self.expression)
    }

    /// Same as [`SelectExpressionEvaluator::evaluate_names`] but for function arguments.
    pub fn evaluate_on_arguments_names(&self) -> Result<Vec<String>, Error> {
        match &self.expression {
            DQE::Variable(selector) => {
                let expl_ctx_loc = self.debugger.exploration_ctx().location();
                let current_function = self
                    .debugger
                    .debugee
                    .debug_info(expl_ctx_loc.pc)?
                    .find_function_by_pc(expl_ctx_loc.global_pc)?
                    .ok_or(FunctionNotFound(expl_ctx_loc.global_pc))?;
                let params = current_function.parameters();

                let params = match selector {
                    VariableSelector::Name { var_name, .. } => params
                        .into_iter()
                        .filter(|param| param.die.base_attributes.name.as_ref() == Some(var_name))
                        .collect::<Vec<_>>(),
                    VariableSelector::Any => params,
                };

                Ok(params
                    .into_iter()
                    .filter_map(|die| die.die.name().map(ToOwned::to_owned))
                    .collect())
            }
            _ => unreachable!("unexpected expression variant"),
        }
    }

    /// Same as [`SelectExpressionEvaluator::evaluate`] but for function arguments.
    pub fn evaluate_on_arguments(&self) -> Result<Vec<DqeResult>, Error> {
        self.evaluate_on_arguments_inner(&self.expression)
    }

    fn evaluate_on_arguments_inner(&self, expression: &DQE) -> Result<Vec<DqeResult>, Error> {
        match expression {
            DQE::Variable(selector) => {
                let expl_ctx_loc = self.debugger.exploration_ctx().location();
                let debugee = &self.debugger.debugee;
                let current_function = debugee
                    .debug_info(expl_ctx_loc.pc)?
                    .find_function_by_pc(expl_ctx_loc.global_pc)?
                    .ok_or(FunctionNotFound(expl_ctx_loc.global_pc))?;
                let params = current_function.parameters();

                let params = match selector {
                    VariableSelector::Name { var_name, .. } => params
                        .into_iter()
                        .filter(|param| param.die.base_attributes.name.as_ref() == Some(var_name))
                        .collect::<Vec<_>>(),
                    VariableSelector::Any => params,
                };

                let mut type_cache = self.debugger.type_cache.borrow_mut();

                Ok(params
                    .iter()
                    .filter_map(|var| {
                        let r#type = weak_error!(type_from_cache!(var, type_cache))?;
                        let var_ir =
                            self.evaluate_single_variable(&self.expression, var, r#type)?;
                        Some(DqeResult {
                            variable: var_ir,
                            scope: var.max_range().map(|r| {
                                let scope: Box<[Range]> = Box::new([r]);
                                scope
                            }),
                        })
                    })
                    .collect())
            }
            DQE::PtrCast(_, target_type_name) => {
                let vars = self.evaluate_from_ptr_cast(target_type_name)?;
                Ok(vars
                    .into_iter()
                    .map(|v| DqeResult {
                        variable: v,
                        scope: None,
                    })
                    .collect())
            }
            DQE::Field(expr, _)
            | DQE::Index(expr, _)
            | DQE::Slice(expr, _, _)
            | DQE::Deref(expr)
            | DQE::Address(expr)
            | DQE::Canonic(expr) => self.evaluate_on_arguments_inner(expr),
        }
    }

    fn evaluate_single_variable(
        &self,
        expression: &DQE,
        variable_die: &ContextualDieRef<impl AsAllocatedData>,
        r#type: &ComplexType,
    ) -> Option<VariableIR> {
        let parser = variable::VariableParser::new(r#type);

        let evaluator = ctx_resolve_unit_call!(variable_die, evaluator, &self.debugger.debugee);
        let evaluation_context = &dwarf::r#type::EvaluationContext {
            evaluator: &evaluator,
            expl_ctx: self.debugger.exploration_ctx(),
        };

        match expression {
            DQE::Variable(_) => {
                let data = variable_die.read_value(
                    self.debugger.exploration_ctx(),
                    &self.debugger.debugee,
                    r#type,
                );
                parser.parse(
                    evaluation_context,
                    VariableIdentity::from_variable_die(variable_die),
                    data,
                )
            }
            DQE::PtrCast(addr, ..) => {
                let data = ObjectBinaryRepr {
                    raw_data: Bytes::copy_from_slice(&(*addr).to_le_bytes()),
                    address: None,
                    size: std::mem::size_of::<usize>(),
                };
                parser.parse(
                    evaluation_context,
                    VariableIdentity::new(NamespaceHierarchy::default(), None),
                    Some(data),
                )
            }
            DQE::Field(expr, field) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.field(field)
            }
            DQE::Index(expr, idx) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.index(idx)
            }
            DQE::Slice(expr, left, right) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.slice(evaluation_context, &parser, *left, *right)
            }
            DQE::Deref(expr) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.deref(evaluation_context, &parser)
            }
            DQE::Address(expr) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.address(evaluation_context, &parser)
            }
            DQE::Canonic(expr) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                Some(var.canonic())
            }
        }
    }
}
