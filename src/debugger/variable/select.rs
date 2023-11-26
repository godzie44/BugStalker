use crate::debugger::debugee::dwarf;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::{AsAllocatedValue, ContextualDieRef};
use crate::debugger::error::Error;
use crate::debugger::error::Error::FunctionNotFound;
use crate::debugger::variable::{AssumeError, ParsingError, VariableIR};
use crate::debugger::{variable, Debugger, FunctionDie};
use crate::{ctx_resolve_unit_call, weak_error};
use std::collections::hash_map::Entry;

#[derive(Debug, PartialEq, Clone)]
pub enum VariableSelector {
    Name { var_name: String, local: bool },
    Any,
}

/// List of operations for select variables and their properties.
/// Expression can be parsed from an input string like `*(*variable1.field2)[1]` (see debugger::command module)
///
/// Supported operations are: dereference, get element by index, get field by name, make slice from pointer.
#[derive(Debug, PartialEq, Clone)]
pub enum Expression {
    Variable(VariableSelector),
    Field(Box<Expression>, String),
    Index(Box<Expression>, u64),
    Slice(Box<Expression>, u64),
    Parentheses(Box<Expression>),
    Deref(Box<Expression>),
}

/// Evaluate `Expression` at current breakpoint (for current debugee location).
pub struct SelectExpressionEvaluator<'a> {
    debugger: &'a Debugger,
    expression: Expression,
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
    pub fn new(debugger: &'a Debugger, expression: Expression) -> Self {
        Self {
            debugger,
            expression,
        }
    }

    /// Evaluate only variable names.
    /// Only filter expression supported.
    pub fn evaluate_names(&self) -> Result<Vec<String>, Error> {
        let ctx = self.debugger.exploration_ctx();
        let get_current_fn = || -> Result<ContextualDieRef<FunctionDie>, Error> {
            self.debugger
                .debugee
                .debug_info(ctx.location().pc)?
                .find_function_by_pc(ctx.location().global_pc)?
                .ok_or(FunctionNotFound(ctx.location().global_pc))
        };

        match &self.expression {
            Expression::Variable(selector) => {
                let vars = match selector {
                    VariableSelector::Name { var_name, local } => {
                        if *local {
                            let current_func = get_current_fn()?;
                            current_func
                                .local_variable(ctx.location().global_pc, var_name)
                                .map(|v| vec![v])
                                .unwrap_or_default()
                        } else {
                            self.debugger
                                .debugee
                                .debug_info(ctx.location().pc)?
                                .find_variables(ctx.location(), var_name)?
                        }
                    }
                    VariableSelector::Any => {
                        let current_func = get_current_fn()?;
                        current_func.local_variables(ctx.location().global_pc)
                    }
                };
                Ok(vars
                    .into_iter()
                    .filter_map(|die| die.die.name().map(ToOwned::to_owned))
                    .collect())
            }
            _ => unreachable!("unexpected expression variant"),
        }
    }

    /// Evaluate select expression and returns list of matched variables.
    pub fn evaluate(&self) -> Result<Vec<VariableIR>, Error> {
        self.evaluate_inner(&self.expression)
    }

    fn evaluate_inner(&self, expression: &Expression) -> Result<Vec<VariableIR>, Error> {
        let ctx = self.debugger.exploration_ctx();

        let get_current_fn = || -> Result<ContextualDieRef<FunctionDie>, Error> {
            self.debugger
                .debugee
                .debug_info(ctx.location().pc)?
                .find_function_by_pc(ctx.location().global_pc)?
                .ok_or(FunctionNotFound(ctx.location().global_pc))
        };

        // evaluate variable one by one in `evaluate_single_variable` method
        // here just filter variables
        match expression {
            Expression::Variable(selector) => {
                let vars = match selector {
                    VariableSelector::Name { var_name, local } => {
                        if *local {
                            let current_func = get_current_fn()?;
                            current_func
                                .local_variable(ctx.location().global_pc, var_name)
                                .map(|v| vec![v])
                                .unwrap_or_default()
                        } else {
                            self.debugger
                                .debugee
                                .debug_info(ctx.location().pc)?
                                .find_variables(ctx.location(), var_name)?
                        }
                    }
                    VariableSelector::Any => {
                        let current_func = get_current_fn()?;
                        current_func.local_variables(ctx.location().global_pc)
                    }
                };

                let mut type_cache = self.debugger.type_cache.borrow_mut();

                Ok(vars
                    .iter()
                    .filter_map(|var| {
                        let r#type = weak_error!(type_from_cache!(var, type_cache))?;
                        self.evaluate_single_variable(&self.expression, var, r#type)
                    })
                    .collect())
            }
            Expression::Field(expr, _)
            | Expression::Index(expr, _)
            | Expression::Slice(expr, _)
            | Expression::Parentheses(expr)
            | Expression::Deref(expr) => self.evaluate_inner(expr),
        }
    }

    /// Same as [`SelectExpressionEvaluator::evaluate_names`] but for function arguments.
    pub fn evaluate_on_arguments_names(&self) -> Result<Vec<String>, Error> {
        match &self.expression {
            Expression::Variable(selector) => {
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
    pub fn evaluate_on_arguments(&self) -> Result<Vec<VariableIR>, Error> {
        self.evaluate_on_arguments_inner(&self.expression)
    }

    fn evaluate_on_arguments_inner(
        &self,
        expression: &Expression,
    ) -> Result<Vec<VariableIR>, Error> {
        match expression {
            Expression::Variable(selector) => {
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

                let mut type_cache = self.debugger.type_cache.borrow_mut();

                Ok(params
                    .iter()
                    .filter_map(|var| {
                        let r#type = weak_error!(type_from_cache!(var, type_cache))?;
                        self.evaluate_single_variable(&self.expression, var, r#type)
                    })
                    .collect())
            }
            Expression::Field(expr, _)
            | Expression::Index(expr, _)
            | Expression::Slice(expr, _)
            | Expression::Parentheses(expr)
            | Expression::Deref(expr) => self.evaluate_on_arguments_inner(expr),
        }
    }

    fn evaluate_single_variable(
        &self,
        expression: &Expression,
        variable_die: &ContextualDieRef<impl AsAllocatedValue>,
        r#type: &ComplexType,
    ) -> Option<VariableIR> {
        let parser = variable::VariableParser::new(r#type);

        let evaluator = ctx_resolve_unit_call!(variable_die, evaluator, &self.debugger.debugee);
        let evaluation_context = &dwarf::r#type::EvaluationContext {
            evaluator: &evaluator,
            expl_ctx: self.debugger.exploration_ctx(),
        };

        match expression {
            Expression::Variable(_) => Some(parser.parse(
                evaluation_context,
                variable::VariableIdentity::from_variable_die(variable_die),
                variable_die.read_value(
                    self.debugger.exploration_ctx(),
                    &self.debugger.debugee,
                    r#type,
                ),
            )),
            Expression::Field(expr, field) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.field(field)
            }
            Expression::Index(expr, idx) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.index(*idx as usize)
            }
            Expression::Slice(expr, len) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.slice(evaluation_context, &parser, *len as usize)
            }
            Expression::Parentheses(expr) => {
                self.evaluate_single_variable(expr, variable_die, r#type)
            }
            Expression::Deref(expr) => {
                let var = self.evaluate_single_variable(expr, variable_die, r#type)?;
                var.deref(evaluation_context, &parser)
            }
        }
    }
}
