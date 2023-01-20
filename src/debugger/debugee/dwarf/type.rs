use crate::debugger::debugee::dwarf::parser::unit::{
    ArrayDie, BaseTypeDie, DieVariant, EnumTypeDie, PointerType, StructTypeDie, TypeMemberDie, Unit,
};
use crate::debugger::debugee::dwarf::parser::DieRef;
use crate::debugger::debugee::dwarf::{eval, ContextualDieRef, EndianRcSlice};
use crate::weak_error;
use bytes::Bytes;
use gimli::{AttributeValue, DwAte, Expression};
use nix::unistd::Pid;
use std::cell::Cell;
use std::collections::HashMap;
use std::mem;
use uuid::Uuid;

pub type TypeDeclarationCache = HashMap<(Uuid, DieRef), TypeDeclaration>;

#[derive(Clone)]
pub struct MemberLocationExpression {
    expr: Expression<EndianRcSlice>,
}

impl MemberLocationExpression {
    fn base_addr(&self, eval_ctx: &EvaluationContext, entity_addr: usize) -> anyhow::Result<usize> {
        Ok(eval_ctx
            .unit
            .evaluator(eval_ctx.pid)
            .evaluate_with_opts(
                self.expr.clone(),
                eval::EvalOption::new().with_at_location(entity_addr.to_ne_bytes()),
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

impl<'a> From<ContextualDieRef<'a, TypeMemberDie>> for StructureMember {
    fn from(ctx_die: ContextualDieRef<'a, TypeMemberDie>) -> Self {
        let loc = ctx_die.die.location.as_ref().map(|attr| attr.value());
        let in_struct_location = if let Some(offset) = loc.as_ref().and_then(|l| l.sdata_value()) {
            Some(MemberLocation::Offset(offset))
        } else if let Some(AttributeValue::Exprloc(ref expr)) = loc {
            Some(MemberLocation::Expr(MemberLocationExpression {
                expr: expr.clone(),
            }))
        } else {
            None
        };

        let type_decl = ctx_die
            .die
            .type_ref
            .and_then(|reference| TypeDeclaration::from_type_ref(ctx_die, reference));

        StructureMember {
            in_struct_location,
            name: ctx_die.die.base_attributes.name.clone(),
            r#type: type_decl,
        }
    }
}

impl StructureMember {
    pub fn value(&self, eval_ctx: &EvaluationContext, base_entity_addr: usize) -> Option<Bytes> {
        let type_size = self.r#type.as_ref()?.size_in_bytes(eval_ctx)? as usize;

        let addr = match self.in_struct_location.as_ref()? {
            MemberLocation::Offset(offset) => {
                Some((base_entity_addr as isize + (*offset as isize)) as usize)
            }
            MemberLocation::Expr(expr) => {
                weak_error!(expr.base_addr(eval_ctx, base_entity_addr))
            }
        }? as *const u8;

        Some(Bytes::from(unsafe {
            std::slice::from_raw_parts(addr, type_size)
        }))
    }
}

#[derive(Clone)]
pub struct ArrayBoundValueExpression {
    expr: Expression<EndianRcSlice>,
}

impl ArrayBoundValueExpression {
    fn bound(&self, eval_ctx: &EvaluationContext) -> anyhow::Result<i64> {
        Ok(eval_ctx
            .unit
            .evaluator(eval_ctx.pid)
            .evaluate_with_opts(self.expr.clone(), eval::EvalOption::new())?
            .into_scalar::<i64>()?)
    }
}

#[derive(Clone)]
pub enum ArrayBoundValue {
    Const(i64),
    Expr(ArrayBoundValueExpression),
}

impl ArrayBoundValue {
    pub fn value(&self, eval_ctx: &EvaluationContext) -> anyhow::Result<i64> {
        match self {
            ArrayBoundValue::Const(v) => Ok(*v),
            ArrayBoundValue::Expr(e) => e.bound(eval_ctx),
        }
    }
}

#[derive(Clone)]
pub enum UpperBound {
    UpperBound(ArrayBoundValue),
    Count(ArrayBoundValue),
}

#[derive(Clone)]
pub struct ArrayType {
    byte_size: Option<u64>,
    pub element_type: Option<Box<TypeDeclaration>>,
    lower_bound: ArrayBoundValue,
    upper_bound: Option<UpperBound>,
    byte_size_memo: Cell<Option<u64>>,
    bounds_memo: Cell<Option<(i64, i64)>>,
}

impl ArrayType {
    pub fn new(
        byte_size: Option<u64>,
        element_type: Option<Box<TypeDeclaration>>,
        lower_bound: ArrayBoundValue,
        upper_bound: Option<UpperBound>,
    ) -> Self {
        Self {
            byte_size,
            element_type,
            lower_bound,
            upper_bound,
            byte_size_memo: Cell::new(None),
            bounds_memo: Cell::new(None),
        }
    }

    fn lower_bound(&self, eval_ctx: &EvaluationContext) -> i64 {
        self.lower_bound.value(eval_ctx).unwrap_or(0)
    }

    pub fn bounds(&self, eval_ctx: &EvaluationContext) -> Option<(i64, i64)> {
        if self.bounds_memo.get().is_none() {
            let lb = self.lower_bound(eval_ctx);
            let bounds = match self.upper_bound.as_ref()? {
                UpperBound::UpperBound(ub) => (lb, ub.value(eval_ctx).ok()? - lb),
                UpperBound::Count(c) => (lb, c.value(eval_ctx).ok()?),
            };
            self.bounds_memo.set(Some(bounds));
        }
        self.bounds_memo.get()
    }

    pub fn size_in_bytes(&self, eval_ctx: &EvaluationContext) -> Option<u64> {
        if self.byte_size.is_some() {
            return self.byte_size;
        }

        if self.byte_size_memo.get().is_none() {
            let bounds = self.bounds(eval_ctx)?;
            let inner_type = self.element_type.as_ref()?;
            let inner_type_size = inner_type.size_in_bytes(eval_ctx)?;
            self.byte_size_memo
                .set(Some(inner_type_size * (bounds.1 - bounds.0) as u64));
        }

        self.byte_size_memo.get()
    }
}

#[derive(Clone)]
pub struct ScalarType {
    pub name: Option<String>,
    pub byte_size: Option<u64>,
    pub encoding: Option<DwAte>,
}

#[derive(Clone)]
pub enum TypeDeclaration {
    Scalar(ScalarType),
    Array(ArrayType),
    Structure {
        name: Option<String>,
        byte_size: Option<u64>,
        members: Vec<StructureMember>,
        type_params: HashMap<String, Option<TypeDeclaration>>,
    },
    CStyleEnum {
        name: Option<String>,
        byte_size: Option<u64>,
        discr_type: Option<Box<TypeDeclaration>>,
        enumerators: HashMap<i64, String>,
    },
    RustEnum {
        name: Option<String>,
        byte_size: Option<u64>,
        discr_type: Option<Box<StructureMember>>,
        /// key `None` is default enumerator
        enumerators: HashMap<Option<i64>, StructureMember>,
    },
    Pointer {
        name: Option<String>,
        target_type: Option<Box<TypeDeclaration>>,
    },
}

impl TypeDeclaration {
    pub fn name(&self) -> Option<String> {
        match self {
            TypeDeclaration::Scalar(s) => s.name.clone(),
            TypeDeclaration::Structure { name, .. } => name.clone(),
            TypeDeclaration::Array(arr) => Some(format!(
                "[{}]",
                arr.element_type
                    .as_ref()
                    .and_then(|t| { t.name() })
                    .as_deref()
                    .unwrap_or("unknown")
            )),
            TypeDeclaration::CStyleEnum { name, .. } => name.clone(),
            TypeDeclaration::RustEnum { name, .. } => name.clone(),
            TypeDeclaration::Pointer { name, .. } => name.clone(),
        }
    }

    pub fn size_in_bytes(&self, eval_ctx: &EvaluationContext) -> Option<u64> {
        match self {
            TypeDeclaration::Scalar(s) => s.byte_size,
            TypeDeclaration::Structure { byte_size, .. } => *byte_size,
            TypeDeclaration::Array(arr) => arr.size_in_bytes(eval_ctx),
            TypeDeclaration::CStyleEnum { byte_size, .. } => *byte_size,
            TypeDeclaration::RustEnum { byte_size, .. } => *byte_size,
            TypeDeclaration::Pointer { .. } => Some(mem::size_of::<usize>() as u64),
        }
    }

    fn from_type_ref<T>(ctx_die: ContextualDieRef<'_, T>, type_ref: DieRef) -> Option<Self> {
        let mb_type_die = ctx_die.context.deref_die(ctx_die.unit, type_ref);
        mb_type_die.and_then(|entry| match &entry.die {
            DieVariant::BaseType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                context: ctx_die.context,
                unit: ctx_die.unit,
                node: &entry.node,
                die,
            })),
            DieVariant::StructType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                context: ctx_die.context,
                unit: ctx_die.unit,
                node: &entry.node,
                die,
            })),
            DieVariant::ArrayType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                context: ctx_die.context,
                unit: ctx_die.unit,
                node: &entry.node,
                die,
            })),
            DieVariant::EnumType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                context: ctx_die.context,
                unit: ctx_die.unit,
                node: &entry.node,
                die,
            })),
            DieVariant::PointerType(die) => Some(TypeDeclaration::from(ContextualDieRef {
                context: ctx_die.context,
                unit: ctx_die.unit,
                node: &entry.node,
                die,
            })),
            _ => None,
        })
    }

    fn from_struct_type(ctx_die: ContextualDieRef<'_, StructTypeDie>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();
        let members = ctx_die
            .node
            .children
            .iter()
            .filter_map(|child_idx| {
                let entry = &ctx_die.unit.entries[*child_idx];
                if let DieVariant::TypeMember(member) = &entry.die {
                    return Some(StructureMember::from(ContextualDieRef {
                        context: ctx_die.context,
                        unit: ctx_die.unit,
                        node: &entry.node,
                        die: member,
                    }));
                }
                None
            })
            .collect::<Vec<_>>();

        let type_params = ctx_die
            .node
            .children
            .iter()
            .filter_map(|child_idx| {
                let entry = &ctx_die.unit.entries[*child_idx];
                if let DieVariant::TemplateType(param) = &entry.die {
                    let name = param.base_attributes.name.clone()?;
                    let mb_type_decl = TypeDeclaration::from_type_ref(ctx_die, param.type_ref?);
                    return Some((name, mb_type_decl));
                }
                None
            })
            .collect::<HashMap<_, _>>();

        TypeDeclaration::Structure {
            name,
            byte_size: ctx_die.die.byte_size,
            members,
            type_params,
        }
    }

    fn from_struct_enum_type(ctx_die: ContextualDieRef<'_, StructTypeDie>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();

        let variant_part = ctx_die.node.children.iter().find_map(|c_idx| {
            if let DieVariant::VariantPart(ref v) = ctx_die.unit.entries[*c_idx].die {
                return Some((v, &ctx_die.unit.entries[*c_idx].node));
            }
            None
        });

        let member_from_ref = |type_ref: DieRef| -> Option<StructureMember> {
            let entry = ctx_die.context.deref_die(ctx_die.unit, type_ref)?;
            if let DieVariant::TypeMember(ref member) = &entry.die {
                return Some(StructureMember::from(ContextualDieRef {
                    context: ctx_die.context,
                    unit: ctx_die.unit,
                    node: &entry.node,
                    die: member,
                }));
            }
            None
        };

        let discr_type = variant_part.and_then(|vp| {
            let variant = vp.0;
            variant
                .discr_ref
                .and_then(member_from_ref)
                .or_else(|| variant.type_ref.and_then(member_from_ref))
        });

        let variants = variant_part
            .map(|vp| {
                let node = vp.1;
                node.children
                    .iter()
                    .filter_map(|idx| {
                        if let DieVariant::Variant(ref v) = ctx_die.unit.entries[*idx].die {
                            return Some((v, &ctx_die.unit.entries[*idx].node));
                        }
                        None
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let enumerators = variants
            .iter()
            .filter_map(|&(variant, node)| {
                let member = node.children.iter().find_map(|&c_idx| {
                    if let DieVariant::TypeMember(ref member) = ctx_die.unit.entries[c_idx].die {
                        return Some(StructureMember::from(ContextualDieRef {
                            context: ctx_die.context,
                            unit: ctx_die.unit,
                            node: &ctx_die.unit.entries[c_idx].node,
                            die: member,
                        }));
                    }
                    None
                })?;
                Some((variant.discr_value, member))
            })
            .collect::<HashMap<_, _>>();

        TypeDeclaration::RustEnum {
            name,
            byte_size: ctx_die.die.byte_size,
            discr_type: discr_type.map(Box::new),
            enumerators,
        }
    }
}

impl From<ContextualDieRef<'_, BaseTypeDie>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'_, BaseTypeDie>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();
        TypeDeclaration::Scalar(ScalarType {
            name,
            byte_size: ctx_die.die.byte_size,
            encoding: ctx_die.die.encoding,
        })
    }
}

impl<'a> From<ContextualDieRef<'a, StructTypeDie>> for TypeDeclaration {
    /// Convert DW_TAG_structure_type into TypeDeclaration.
    /// For rust DW_TAG_structure_type DIE can be interpreter as enum, see https://github.com/rust-lang/rust/issues/32920
    fn from(ctx_die: ContextualDieRef<'a, StructTypeDie>) -> Self {
        let is_enum =
            ctx_die.node.children.iter().any(|c_idx| {
                matches!(ctx_die.unit.entries[*c_idx].die, DieVariant::VariantPart(_))
            });

        if is_enum {
            TypeDeclaration::from_struct_enum_type(ctx_die)
        } else {
            TypeDeclaration::from_struct_type(ctx_die)
        }
    }
}

impl<'a> From<ContextualDieRef<'a, ArrayDie>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'a, ArrayDie>) -> Self {
        let type_decl = ctx_die
            .die
            .type_ref
            .and_then(|reference| TypeDeclaration::from_type_ref(ctx_die, reference));

        let subrange = ctx_die.node.children.iter().find_map(|&child_idx| {
            let entry = &ctx_die.unit.entries[child_idx];
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
                        ArrayBoundValue::Expr(ArrayBoundValueExpression { expr: expr.clone() })
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
                        ArrayBoundValueExpression { expr: expr.clone() },
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
                        ArrayBoundValueExpression { expr: expr.clone() },
                    )));
                };
            }

            None
        });

        TypeDeclaration::Array(ArrayType::new(
            ctx_die.die.byte_size,
            type_decl.map(Box::new),
            lower_bound,
            upper_bound,
        ))
    }
}

impl<'a> From<ContextualDieRef<'a, EnumTypeDie>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'a, EnumTypeDie>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();

        let discr_type = ctx_die
            .die
            .type_ref
            .and_then(|reference| TypeDeclaration::from_type_ref(ctx_die, reference));

        let enumerators = ctx_die
            .node
            .children
            .iter()
            .filter_map(|&child_idx| {
                let entry = &ctx_die.unit.entries[child_idx];
                if let DieVariant::Enumerator(ref enumerator) = entry.die {
                    Some((
                        enumerator.const_value?,
                        enumerator.base_attributes.name.as_ref()?.to_string(),
                    ))
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();

        TypeDeclaration::CStyleEnum {
            name,
            byte_size: ctx_die.die.byte_size,
            discr_type: discr_type.map(Box::new),
            enumerators,
        }
    }
}

impl<'a> From<ContextualDieRef<'a, PointerType>> for TypeDeclaration {
    fn from(ctx_die: ContextualDieRef<'a, PointerType>) -> Self {
        let name = ctx_die.die.base_attributes.name.clone();
        let type_decl = ctx_die
            .die
            .type_ref
            .and_then(|reference| TypeDeclaration::from_type_ref(ctx_die, reference));

        TypeDeclaration::Pointer {
            name,
            target_type: type_decl.map(Box::new),
        }
    }
}

pub struct EvaluationContext<'a> {
    pub unit: &'a Unit,
    pub pid: Pid,
}
