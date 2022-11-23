use crate::debugger::dwarf::eval::ExpressionEvaluator;
use crate::debugger::dwarf::parse::{BaseTypeDie, ContextualDieRef, DieVariant, StructTypeDie};
use crate::debugger::dwarf::{eval, EndianRcSlice};
use crate::weak_error;
use bytes::Bytes;
use gimli::{AttributeValue, DwAte, Expression};
use nix::unistd::Pid;
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
        let type_size = self.r#type.as_ref()?.size_in_bytes()? as usize;

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
pub enum TypeDeclaration {
    Scalar {
        name: Option<String>,
        byte_size: Option<u64>,
        encoding: Option<DwAte>,
    },
    Structure {
        name: Option<String>,
        byte_size: Option<u64>,
        members: Vec<StructureMember>,
    },
}

impl TypeDeclaration {
    pub fn size_in_bytes(&self) -> Option<u64> {
        match self {
            TypeDeclaration::Scalar { byte_size, .. } => *byte_size,
            TypeDeclaration::Structure { byte_size, .. } => *byte_size,
        }
    }

    pub fn name(&self) -> Option<String> {
        match self {
            TypeDeclaration::Scalar { name, .. } => name.clone(),
            TypeDeclaration::Structure { name, .. } => name.clone(),
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

                    let type_decl = member.type_addr.as_ref().and_then(|addr| {
                        if let gimli::AttributeValue::UnitRef(unit_offset) = addr.value() {
                            let mb_type_die = ctx_die.context.find_die(unit_offset);
                            mb_type_die.and_then(|entry| match &entry.die {
                                DieVariant::BaseType(die) => {
                                    Some(TypeDeclaration::from(ContextualDieRef {
                                        context: ctx_die.context,
                                        node: &entry.node,
                                        die,
                                    }))
                                }
                                DieVariant::StructType(die) => {
                                    Some(TypeDeclaration::from(ContextualDieRef {
                                        context: ctx_die.context,
                                        node: &entry.node,
                                        die,
                                    }))
                                }
                                _ => None,
                            })
                        } else {
                            None
                        }
                    });

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
