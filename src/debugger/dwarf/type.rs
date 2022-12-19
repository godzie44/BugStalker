use crate::debugger::dwarf::eval::ExpressionEvaluator;
use crate::debugger::dwarf::parse::{
    ArrayDie, BaseTypeDie, ContextualDieRef, DieVariant, StructTypeDie,
};
use crate::debugger::dwarf::{eval, EndianRcSlice};
use crate::weak_error;
use bytes::Bytes;
use gimli::{Attribute, AttributeValue, DwAte, Expression};
use nix::unistd::Pid;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

pub(super) type TypeDeclarationCache = HashMap<(usize, usize), TypeDeclaration>;

#[derive(Clone)]
pub struct MemberLocationExpression {
    evaluator: Rc<ExpressionEvaluator>,
    expr: Expression<EndianRcSlice>,
}

impl MemberLocationExpression {
    fn base_addr(&self, entity_addr: usize, pid: Pid) -> anyhow::Result<usize> {
        Ok(self
            .evaluator
            .evaluate_with_opts(
                self.expr.clone(),
                pid,
                eval::EvalOption::new().with_at_location(entity_addr.to_le_bytes()),
            )?
            .into_scalar::<usize>()?)
    }
}

#[derive(Clone)]
pub enum MemberLocation {
    Offset(i64),
    Expr(MemberLocationExpression),
}

#[derive(Clone)]
pub struct StructureMember {
    pub in_struct_location: Option<MemberLocation>,
    pub name: Option<String>,
    pub r#type: Option<TypeDeclaration>,
}

impl StructureMember {
    pub fn value(&self, base_entity_addr: usize, pid: Pid) -> Option<Bytes> {
        let type_size = self.r#type.as_ref()?.size_in_bytes(pid)? as usize;

        let addr = match self.in_struct_location.as_ref()? {
            MemberLocation::Offset(offset) => {
                Some((base_entity_addr as isize + (*offset as isize)) as usize)
            }
            MemberLocation::Expr(expr) => weak_error!(expr.base_addr(base_entity_addr, pid)),
        }? as *const u8;

        Some(Bytes::from(unsafe {
            std::slice::from_raw_parts(addr, type_size)
        }))
    }
}

#[derive(Clone)]
pub struct ArrayBoundValueExpression {
    evaluator: Rc<ExpressionEvaluator>,
    expr: Expression<EndianRcSlice>,
}

impl ArrayBoundValueExpression {
    fn bound(&self, pid: Pid) -> anyhow::Result<i64> {
        Ok(self
            .evaluator
            .evaluate_with_opts(self.expr.clone(), pid, eval::EvalOption::new())?
            .into_scalar::<i64>()?)
    }
}

#[derive(Clone)]
pub enum ArrayBoundValue {
    Const(i64),
    Expr(ArrayBoundValueExpression),
}

impl ArrayBoundValue {
    pub fn value(&self, pid: Pid) -> anyhow::Result<i64> {
        match self {
            ArrayBoundValue::Const(v) => Ok(*v),
            ArrayBoundValue::Expr(e) => e.bound(pid),
        }
    }
}

#[derive(Clone)]
pub enum UpperBound {
    UpperBound(ArrayBoundValue),
    Count(ArrayBoundValue),
}

#[derive(Clone)]
pub struct ArrayDeclaration {
    byte_size: Option<u64>,
    pub element_type: Option<Box<TypeDeclaration>>,
    lower_bound: ArrayBoundValue,
    upper_bound: Option<UpperBound>,
    byte_size_memo: Cell<Option<u64>>,
    bounds_memo: Cell<Option<(i64, i64)>>,
}

impl ArrayDeclaration {
    fn lower_bound(&self, pid: Pid) -> i64 {
        self.lower_bound.value(pid).unwrap_or(0)
    }

    pub fn bounds(&self, pid: Pid) -> Option<(i64, i64)> {
        if self.bounds_memo.get().is_none() {
            let lb = self.lower_bound(pid);
            let bounds = match self.upper_bound.as_ref()? {
                UpperBound::UpperBound(ub) => (lb, ub.value(pid).ok()? - lb),
                UpperBound::Count(c) => (lb, c.value(pid).ok()?),
            };
            self.bounds_memo.set(Some(bounds));
        }
        self.bounds_memo.get()
    }

    pub fn size_in_bytes(&self, pid: Pid) -> Option<u64> {
        if self.byte_size.is_some() {
            return self.byte_size;
        }

        if self.byte_size_memo.get().is_none() {
            let bounds = self.bounds(pid)?;
            let inner_type = self.element_type.as_ref()?;
            let inner_type_size = inner_type.size_in_bytes(pid)?;
            self.byte_size_memo
                .set(Some(inner_type_size * (bounds.1 - bounds.0) as u64));
        }

        self.byte_size_memo.get()
    }
}

#[derive(Clone)]
pub enum TypeDeclaration {
    Scalar {
        name: Option<String>,
        byte_size: Option<u64>,
        encoding: Option<DwAte>,
    },
    Array(ArrayDeclaration),
    Structure {
        name: Option<String>,
        byte_size: Option<u64>,
        members: Vec<StructureMember>,
    },
}

impl TypeDeclaration {
    pub fn size_in_bytes(&self, pid: Pid) -> Option<u64> {
        match self {
            TypeDeclaration::Scalar { byte_size, .. } => *byte_size,
            TypeDeclaration::Structure { byte_size, .. } => *byte_size,
            TypeDeclaration::Array(arr) => arr.size_in_bytes(pid),
        }
    }

    pub fn name(&self) -> Option<String> {
        match self {
            TypeDeclaration::Scalar { name, .. } => name.clone(),
            TypeDeclaration::Structure { name, .. } => name.clone(),
            TypeDeclaration::Array(arr) => Some(format!(
                "[{}]",
                arr.element_type
                    .as_ref()
                    .and_then(|t| { t.name() })
                    .as_deref()
                    .unwrap_or("unknown")
            )),
        }
    }

    fn from_type_addr_attr<T>(
        ctx_die: ContextualDieRef<'_, T>,
        attr: &Attribute<EndianRcSlice>,
    ) -> Option<Self> {
        if let gimli::AttributeValue::UnitRef(unit_offset) = attr.value() {
            let mb_type_die = ctx_die.context.find_die(unit_offset);
            mb_type_die.and_then(|entry| match &entry.die {
                DieVariant::BaseType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                    context: ctx_die.context,
                    node: &entry.node,
                    die,
                })),
                DieVariant::StructType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                    context: ctx_die.context,
                    node: &entry.node,
                    die,
                })),
                DieVariant::ArrayType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                    context: ctx_die.context,
                    node: &entry.node,
                    die,
                })),
                _ => None,
            })
        } else {
            None
        }
    }
}

impl From<ContextualDieRef<'_, BaseTypeDie>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'_, BaseTypeDie>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();
        TypeDeclaration::Scalar {
            name,
            byte_size: ctx_die.die.byte_size,
            encoding: ctx_die.die.encoding,
        }
    }
}

impl From<ContextualDieRef<'_, StructTypeDie>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'_, StructTypeDie>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();
        let members = ctx_die
            .node
            .children
            .iter()
            .filter_map(|child_idx| {
                let entry = &ctx_die.context.entries[*child_idx];

                if let DieVariant::TypeMember(member) = &entry.die {
                    let loc = member.location.as_ref().map(|attr| attr.value());
                    let in_struct_location =
                        if let Some(offset) = loc.as_ref().and_then(|l| l.sdata_value()) {
                            Some(MemberLocation::Offset(offset))
                        } else if let Some(AttributeValue::Exprloc(ref expr)) = loc {
                            Some(MemberLocation::Expr(MemberLocationExpression {
                                evaluator: Rc::clone(&ctx_die.context.expr_evaluator),
                                expr: expr.clone(),
                            }))
                        } else {
                            None
                        };

                    let type_decl = member
                        .type_addr
                        .as_ref()
                        .and_then(|addr| TypeDeclaration::from_type_addr_attr(ctx_die, addr));

                    Some(StructureMember {
                        in_struct_location,
                        name: member.base_attributes.name.clone(),
                        r#type: type_decl,
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        TypeDeclaration::Structure {
            name,
            byte_size: ctx_die.die.byte_size,
            members,
        }
    }
}

impl From<ContextualDieRef<'_, ArrayDie>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'_, ArrayDie>) -> Self {
        let type_decl = ctx_die
            .die
            .type_addr
            .as_ref()
            .and_then(|addr| TypeDeclaration::from_type_addr_attr(ctx_die, addr));

        let subrange = ctx_die.node.children.iter().find_map(|&child_idx| {
            let entry = &ctx_die.context.entries[child_idx];
            if let DieVariant::ArraySubrange(ref subrange) = entry.die {
                Some(subrange)
            } else {
                None
            }
        });

        let lower_bound = subrange
            .map(|sr| {
                let lower_bound = sr.lower_bound.as_ref().map(|lb| lb.value());
                let in_struct_location =
                    if let Some(bound) = lower_bound.as_ref().and_then(|l| l.sdata_value()) {
                        ArrayBoundValue::Const(bound)
                    } else if let Some(AttributeValue::Exprloc(ref expr)) = lower_bound {
                        ArrayBoundValue::Expr(ArrayBoundValueExpression {
                            evaluator: Rc::clone(&ctx_die.context.expr_evaluator),
                            expr: expr.clone(),
                        })
                    } else {
                        // rust default lower bound
                        ArrayBoundValue::Const(0)
                    };
                in_struct_location
            })
            .unwrap_or(ArrayBoundValue::Const(0));

        let upper_bound = subrange.and_then(|sr| {
            if let Some(ref count) = sr.count {
                return if let Some(cnt) = count.value().sdata_value() {
                    Some(UpperBound::Count(ArrayBoundValue::Const(cnt)))
                } else if let AttributeValue::Exprloc(ref expr) = count.value() {
                    Some(UpperBound::Count(ArrayBoundValue::Expr(
                        ArrayBoundValueExpression {
                            evaluator: Rc::clone(&ctx_die.context.expr_evaluator),
                            expr: expr.clone(),
                        },
                    )))
                } else {
                    None
                };
            }

            if let Some(ref bound) = sr.upper_bound {
                if let Some(bound) = bound.value().sdata_value() {
                    return Some(UpperBound::UpperBound(ArrayBoundValue::Const(bound)));
                } else if let AttributeValue::Exprloc(ref expr) = bound.value() {
                    return Some(UpperBound::UpperBound(ArrayBoundValue::Expr(
                        ArrayBoundValueExpression {
                            evaluator: Rc::clone(&ctx_die.context.expr_evaluator),
                            expr: expr.clone(),
                        },
                    )));
                };
            }

            None
        });

        TypeDeclaration::Array(ArrayDeclaration {
            byte_size: ctx_die.die.byte_size,
            element_type: type_decl.map(Box::new),
            lower_bound,
            upper_bound,
            byte_size_memo: Cell::new(None),
            bounds_memo: Cell::new(None),
        })
    }
}
