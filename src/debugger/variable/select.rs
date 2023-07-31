use crate::debugger::debugee::dwarf;
use crate::debugger::debugee::dwarf::r#type::ComplexType;
use crate::debugger::debugee::dwarf::{AsAllocatedValue, ContextualDieRef};
use crate::debugger::variable::VariableIR;
use crate::debugger::{variable, Debugger};
use crate::{ctx_resolve_unit_call, weak_error};
use anyhow::anyhow;
use std::collections::hash_map::Entry;

#[derive(Debug, PartialEq)]
pub enum VariableSelector {
    Name(String),
    Any,
}

/// List of operations for select variables and their properties.
/// Expression can be parsed from an input string like "*(*variable1.field2)[1]" (see debugger::command module)
/// Supported operations are: dereference, get element by index, get field by name, make slice from pointer.
#[derive(Debug, PartialEq)]
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
            .ok_or(anyhow!(
                "unknown type for variable {name}",
                name = $variable.die.name().unwrap_or_default()
            ))
    };
}

impl<'a> SelectExpressionEvaluator<'a> {
    pub fn new(debugger: &'a Debugger, expression: Expression) -> anyhow::Result<Self> {
        Ok(Self {
            debugger,
            //  expl_context: debugger.exploration_ctx(),
            expression,
        })
    }

    /// Evaluate only variable names.
    /// Only filter expression supported.
    pub fn evaluate_names(&self) -> anyhow::Result<Vec<String>> {
        let ctx = self.debugger.exploration_ctx();
        match &self.expression {
            Expression::Variable(selector) => {
                let vars = match selector {
                    VariableSelector::Name(variable_name) => self
                        .debugger
                        .debugee
                        .dwarf
                        .find_variables(ctx.location(), variable_name),
                    VariableSelector::Any => {
                        let current_func = self
                            .debugger
                            .debugee
                            .dwarf
                            .find_function_by_pc(ctx.location().global_pc)
                            .ok_or_else(|| anyhow!("not in function"))?;
                        current_func.local_variables(ctx.location().global_pc)
                    }
                };
                Ok(vars
                    .into_iter()
                    .filter_map(|die| die.die.name().map(ToOwned::to_owned))
                    .collect())
            }
            _ => panic!("unsupported"),
        }
    }

    /// Evaluate select expression and returns list of matched variables.
    pub fn evaluate(&self) -> anyhow::Result<Vec<VariableIR>> {
        self.evaluate_inner(&self.expression)
    }

    fn evaluate_inner(&self, expression: &Expression) -> anyhow::Result<Vec<VariableIR>> {
        let ctx = self.debugger.exploration_ctx();
        // evaluate variable one by one in `evaluate_single_variable` method
        // here just filter variables
        match expression {
            Expression::Variable(selector) => {
                let vars = match selector {
                    VariableSelector::Name(variable_name) => self
                        .debugger
                        .debugee
                        .dwarf
                        .find_variables(ctx.location(), variable_name),
                    VariableSelector::Any => {
                        let current_func = self
                            .debugger
                            .debugee
                            .dwarf
                            .find_function_by_pc(ctx.location().global_pc)
                            .ok_or_else(|| anyhow!("not in function"))?;
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
    pub fn evaluate_on_arguments_names(&self) -> anyhow::Result<Vec<String>> {
        match &self.expression {
            Expression::Variable(selector) => {
                let current_function = self
                    .debugger
                    .debugee
                    .dwarf
                    .find_function_by_pc(self.debugger.exploration_ctx().location().global_pc)
                    .ok_or_else(|| anyhow!("not in function"))?;
                let params = current_function.parameters();

                let params = match selector {
                    VariableSelector::Name(variable_name) => params
                        .into_iter()
                        .filter(|param| {
                            param.die.base_attributes.name.as_ref() == Some(variable_name)
                        })
                        .collect::<Vec<_>>(),
                    VariableSelector::Any => params,
                };

                Ok(params
                    .into_iter()
                    .filter_map(|die| die.die.name().map(ToOwned::to_owned))
                    .collect())
            }
            _ => panic!("unsupported"),
        }
    }

    /// Same as [`SelectExpressionEvaluator::evaluate`] but for function arguments.
    pub fn evaluate_on_arguments(&self) -> anyhow::Result<Vec<VariableIR>> {
        self.evaluate_on_arguments_inner(&self.expression)
    }

    fn evaluate_on_arguments_inner(
        &self,
        expression: &Expression,
    ) -> anyhow::Result<Vec<VariableIR>> {
        match expression {
            Expression::Variable(selector) => {
                let current_function = self
                    .debugger
                    .debugee
                    .dwarf
                    .find_function_by_pc(self.debugger.exploration_ctx().location().global_pc)
                    .ok_or_else(|| anyhow!("not in function"))?;
                let params = current_function.parameters();

                let params = match selector {
                    VariableSelector::Name(variable_name) => params
                        .into_iter()
                        .filter(|param| {
                            param.die.base_attributes.name.as_ref() == Some(variable_name)
                        })
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
                    &self.debugger.exploration_ctx(),
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
