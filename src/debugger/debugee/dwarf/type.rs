use crate::debugger::debugee::dwarf::eval::{AddressKind, EvaluationContext};
use crate::debugger::debugee::dwarf::unit::{
    ArrayDie, AtomicDie, BaseTypeDie, ConstTypeDie, DieRef, DieVariant, EnumTypeDie, PointerType,
    RestrictDie, StructTypeDie, SubroutineDie, TypeDefDie, TypeMemberDie, UnionTypeDie,
    VolatileDie,
};
use crate::debugger::debugee::dwarf::{eval, ContextualDieRef, EndianArcSlice, NamespaceHierarchy};
use crate::debugger::error::Error;
use crate::debugger::variable::ObjectBinaryRepr;
use crate::{ctx_resolve_unit_call, weak_error};
use bytes::Bytes;
use gimli::{AttributeValue, DwAte, Expression};
use log::warn;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::rc::Rc;
use strum_macros::Display;
use uuid::Uuid;

/// Type identifier.
pub type TypeId = DieRef;

/// Type name with namespace.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct TypeIdentity {
    namespace: NamespaceHierarchy,
    name: Option<String>,
}

impl TypeIdentity {
    /// Create type identity with empty namespace.
    #[inline(always)]
    pub fn no_namespace(name: impl ToString) -> Self {
        Self {
            namespace: Default::default(),
            name: Some(name.to_string()),
        }
    }

    /// Create type identity for an unknown type.
    #[inline(always)]
    pub fn unknown() -> Self {
        Self {
            namespace: Default::default(),
            name: None,
        }
    }

    /// True whether a type is unknown.
    #[inline(always)]
    pub fn is_unknown(&self) -> bool {
        self.name.is_none()
    }

    #[inline(always)]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Return formatted type name.  
    #[inline(always)]
    pub fn name_fmt(&self) -> &str {
        self.name().unwrap_or("unknown")
    }

    /// Create address type name.
    #[inline(always)]
    pub fn as_address_type(&self) -> TypeIdentity {
        TypeIdentity {
            namespace: self.namespace.clone(),
            name: Some(format!("&{}", self.name_fmt())),
        }
    }

    /// Create dereferenced type name.
    #[inline(always)]
    pub fn as_deref_type(&self) -> TypeIdentity {
        TypeIdentity {
            namespace: self.namespace.clone(),
            name: Some(format!("*{}", self.name_fmt())),
        }
    }

    /// Create array type name from element type name.
    #[inline(always)]
    pub fn as_array_type(&self) -> TypeIdentity {
        TypeIdentity::no_namespace(format!("[{}]", self.name_fmt()))
    }
}

#[derive(Clone, Debug)]
pub struct MemberLocationExpression {
    expr: Expression<EndianArcSlice>,
}

impl MemberLocationExpression {
    fn base_addr(&self, eval_ctx: &EvaluationContext, entity_addr: usize) -> Result<usize, Error> {
        eval_ctx
            .evaluator
            .evaluate_with_resolver(
                eval::ExternalRequirementsResolver::new()
                    .with_at_location(entity_addr.to_ne_bytes()),
                eval_ctx.expl_ctx,
                self.expr.clone(),
            )?
            .into_scalar::<usize>(AddressKind::Value)
    }
}

#[derive(Clone, Debug)]
pub enum MemberLocation {
    Offset(i64),
    Expr(MemberLocationExpression),
}

#[derive(Clone, Debug)]
pub struct StructureMember {
    pub in_struct_location: Option<MemberLocation>,
    pub name: Option<String>,
    pub type_ref: Option<TypeId>,
}

impl StructureMember {
    /// Return binary representation of structure member data.
    ///
    /// # Arguments
    ///
    /// * `eval_ctx`: evaluation context
    /// * `r#type`: member data type
    /// * `base_data`: binary representation of the parent structure
    pub fn value(
        &self,
        eval_ctx: &EvaluationContext,
        r#type: &ComplexType,
        base_data: &ObjectBinaryRepr,
    ) -> Option<ObjectBinaryRepr> {
        let type_size = r#type.type_size_in_bytes(eval_ctx, self.type_ref?)? as usize;

        let base_entity_addr = base_data.raw_data.as_ptr() as usize;
        let addr = match self.in_struct_location.as_ref()? {
            MemberLocation::Offset(offset) => {
                Some((base_entity_addr as isize + (*offset as isize)) as usize)
            }
            MemberLocation::Expr(expr) => {
                weak_error!(expr.base_addr(eval_ctx, base_entity_addr))
            }
        }? as *const u8;

        let offset = addr as isize - base_entity_addr as isize;
        let new_in_debugee_addr = base_data
            .address
            .map(|addr| (addr as isize + offset) as usize);

        let raw_data = Bytes::from(unsafe { std::slice::from_raw_parts(addr, type_size) });

        Some(ObjectBinaryRepr {
            raw_data,
            address: new_in_debugee_addr,
            size: type_size,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ArrayBoundValueExpression {
    expr: Expression<EndianArcSlice>,
}

impl ArrayBoundValueExpression {
    fn bound(&self, eval_ctx: &EvaluationContext) -> Result<i64, Error> {
        eval_ctx
            .evaluator
            .evaluate(eval_ctx.expl_ctx, self.expr.clone())?
            .into_scalar::<i64>(AddressKind::MemoryAddress)
    }
}

#[derive(Clone, Debug)]
pub enum ArrayBoundValue {
    Const(i64),
    Expr(ArrayBoundValueExpression),
}

impl ArrayBoundValue {
    pub fn value(&self, eval_ctx: &EvaluationContext) -> Result<i64, Error> {
        match self {
            ArrayBoundValue::Const(v) => Ok(*v),
            ArrayBoundValue::Expr(e) => e.bound(eval_ctx),
        }
    }
}

#[derive(Clone, Debug)]
pub enum UpperBound {
    UpperBound(ArrayBoundValue),
    Count(ArrayBoundValue),
}

#[derive(Clone, Debug)]
pub struct ArrayType {
    pub namespaces: NamespaceHierarchy,
    byte_size: Option<u64>,
    pub element_type: Option<TypeId>,
    lower_bound: ArrayBoundValue,
    upper_bound: Option<UpperBound>,
    byte_size_memo: Cell<Option<u64>>,
    bounds_memo: Cell<Option<(i64, i64)>>,
}

impl ArrayType {
    pub fn new(
        namespaces: NamespaceHierarchy,
        byte_size: Option<u64>,
        element_type: Option<TypeId>,
        lower_bound: ArrayBoundValue,
        upper_bound: Option<UpperBound>,
    ) -> Self {
        Self {
            namespaces,
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

    pub fn size_in_bytes(
        &self,
        eval_ctx: &EvaluationContext,
        type_graph: &ComplexType,
    ) -> Option<u64> {
        if self.byte_size.is_some() {
            return self.byte_size;
        }

        if self.byte_size_memo.get().is_none() {
            let bounds = self.bounds(eval_ctx)?;
            let inner_type_size = type_graph.type_size_in_bytes(eval_ctx, self.element_type?)?;
            self.byte_size_memo
                .set(Some(inner_type_size * (bounds.1 - bounds.0) as u64));
        }

        self.byte_size_memo.get()
    }
}

#[derive(Clone, Debug)]
pub struct ScalarType {
    pub namespaces: NamespaceHierarchy,
    pub name: Option<String>,
    pub byte_size: Option<u64>,
    pub encoding: Option<DwAte>,
}

impl ScalarType {
    pub fn identity(&self) -> TypeIdentity {
        TypeIdentity {
            namespace: self.namespaces.clone(),
            name: self.name.clone(),
        }
    }
}

/// List of type modifiers
#[derive(Display, Clone, Copy, PartialEq, Debug)]
#[strum(serialize_all = "snake_case")]
pub enum CModifier {
    TypeDef,
    Const,
    Volatile,
    Atomic,
    Restrict,
}

#[derive(Clone, Debug)]
pub enum TypeDeclaration {
    Scalar(ScalarType),
    Array(ArrayType),
    CStyleEnum {
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        byte_size: Option<u64>,
        discr_type: Option<TypeId>,
        enumerators: HashMap<i64, String>,
    },
    Pointer {
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        target_type: Option<TypeId>,
    },
    Structure {
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        byte_size: Option<u64>,
        members: Vec<StructureMember>,
        type_params: HashMap<String, Option<TypeId>>,
    },
    Union {
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        byte_size: Option<u64>,
        members: Vec<StructureMember>,
    },
    RustEnum {
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        byte_size: Option<u64>,
        discr_type: Option<Box<StructureMember>>,
        /// key `None` is default enumerator
        enumerators: HashMap<Option<i64>, StructureMember>,
    },
    Subroutine {
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        return_type: Option<TypeId>,
    },
    ModifiedType {
        modifier: CModifier,
        namespaces: NamespaceHierarchy,
        name: Option<String>,
        inner: Option<TypeId>,
    },
}

/// Type representation. This is a graph of types where vertexes is a type declaration and edges
/// is a dependencies between types. Type linking implemented by `TypeId` references.
/// Root is id of a main type.
#[derive(Clone, Debug)]
pub struct ComplexType {
    pub types: HashMap<TypeId, TypeDeclaration>,
    root: TypeId,
}

impl ComplexType {
    /// Return root type id.
    #[inline(always)]
    pub fn root(&self) -> TypeId {
        self.root
    }

    /// Return name of some of a type existed in a complex type.
    pub fn identity(&self, typ: TypeId) -> TypeIdentity {
        let Some(r#type) = self.types.get(&typ) else {
            return TypeIdentity::unknown();
        };

        match r#type {
            TypeDeclaration::Scalar(s) => s.identity(),
            TypeDeclaration::Structure {
                name, namespaces, ..
            } => TypeIdentity {
                namespace: namespaces.clone(),
                name: name.as_ref().cloned(),
            },
            TypeDeclaration::Array(arr) => {
                let el_ident = arr.element_type.map(|t| self.identity(t));
                let name = format!(
                    "[{}]",
                    el_ident
                        .as_ref()
                        .map(|ident| ident.name_fmt())
                        .unwrap_or("unknown")
                );

                TypeIdentity {
                    namespace: arr.namespaces.clone(),
                    name: Some(name.to_string()),
                }
            }
            TypeDeclaration::CStyleEnum {
                name, namespaces, ..
            } => TypeIdentity {
                namespace: namespaces.clone(),
                name: name.as_ref().cloned(),
            },
            TypeDeclaration::RustEnum {
                name, namespaces, ..
            } => TypeIdentity {
                namespace: namespaces.clone(),
                name: name.as_ref().cloned(),
            },
            TypeDeclaration::Pointer {
                name, namespaces, ..
            } => TypeIdentity {
                namespace: namespaces.clone(),
                name: name.as_ref().cloned(),
            },
            TypeDeclaration::Union {
                name, namespaces, ..
            } => TypeIdentity {
                namespace: namespaces.clone(),
                name: name.as_ref().cloned(),
            },
            TypeDeclaration::Subroutine {
                name, namespaces, ..
            } => TypeIdentity {
                namespace: namespaces.clone(),
                name: name.as_ref().cloned(),
            },
            TypeDeclaration::ModifiedType {
                modifier,
                namespaces,
                name,
                inner,
                ..
            } => match name {
                None => {
                    let name = inner.map(|inner_id| {
                        let ident = self.identity(inner_id);
                        format!("{modifier} {name}", name = ident.name_fmt())
                    });

                    TypeIdentity {
                        namespace: namespaces.clone(),
                        name,
                    }
                }
                Some(n) => TypeIdentity {
                    namespace: namespaces.clone(),
                    name: Some(n.to_string()),
                },
            },
        }
    }

    /// Return size of a type existed from a complex type.
    pub fn type_size_in_bytes(&self, eval_ctx: &EvaluationContext, typ: TypeId) -> Option<u64> {
        match &self.types.get(&typ)? {
            TypeDeclaration::Scalar(s) => s.byte_size,
            TypeDeclaration::Structure { byte_size, .. } => *byte_size,
            TypeDeclaration::Array(arr) => arr.size_in_bytes(eval_ctx, self),
            TypeDeclaration::CStyleEnum { byte_size, .. } => *byte_size,
            TypeDeclaration::RustEnum { byte_size, .. } => *byte_size,
            TypeDeclaration::Pointer { .. } => Some(mem::size_of::<usize>() as u64),
            TypeDeclaration::Union { byte_size, .. } => *byte_size,
            TypeDeclaration::Subroutine { .. } => Some(mem::size_of::<usize>() as u64),
            TypeDeclaration::ModifiedType { inner, .. } => {
                inner.and_then(|inner_id| self.type_size_in_bytes(eval_ctx, inner_id))
            }
        }
    }

    /// Visit type children in bfs order, `start_at` - identity of root type.
    pub fn bfs_iterator(&self, start_at: TypeId) -> BfsIterator {
        BfsIterator {
            complex_type: self,
            queue: VecDeque::from([start_at]),
        }
    }
}

/// Bfs iterator over related types.
/// Note that this iterator may be infinite.
pub struct BfsIterator<'a> {
    complex_type: &'a ComplexType,
    queue: VecDeque<TypeId>,
}

impl<'a> Iterator for BfsIterator<'a> {
    type Item = &'a TypeDeclaration;

    fn next(&mut self) -> Option<Self::Item> {
        let el_id = self.queue.pop_back()?;
        let type_decl = &self.complex_type.types[&el_id];
        match type_decl {
            TypeDeclaration::Scalar(_) => {}
            TypeDeclaration::Array(arr) => {
                if let Some(el) = arr.element_type {
                    self.queue.push_front(el);
                }
            }
            TypeDeclaration::CStyleEnum { discr_type, .. } => {
                if let Some(el) = discr_type {
                    self.queue.push_front(*el);
                }
            }
            TypeDeclaration::Pointer { target_type, .. } => {
                if let Some(el) = target_type {
                    self.queue.push_front(*el);
                }
            }
            TypeDeclaration::Structure {
                members,
                type_params,
                ..
            } => {
                members.iter().for_each(|m| {
                    if let Some(el) = m.type_ref {
                        self.queue.push_front(el);
                    }
                });
                type_params.values().for_each(|t| {
                    if let Some(el) = t {
                        self.queue.push_front(*el);
                    }
                });
            }
            TypeDeclaration::Union { members, .. } => {
                members.iter().for_each(|m| {
                    if let Some(el) = m.type_ref {
                        self.queue.push_front(el);
                    }
                });
            }
            TypeDeclaration::RustEnum {
                discr_type,
                enumerators,
                ..
            } => {
                if let Some(el) = discr_type.as_ref().and_then(|member| member.type_ref) {
                    self.queue.push_front(el);
                }
                enumerators.values().for_each(|member| {
                    if let Some(el) = member.type_ref {
                        self.queue.push_front(el);
                    }
                });
            }
            TypeDeclaration::Subroutine { .. } => {}
            TypeDeclaration::ModifiedType { inner, .. } => {
                if let Some(type_id) = inner {
                    self.queue.push_front(*type_id);
                }
            }
        }

        Some(type_decl)
    }
}

/// Dwarf DIE parser.
pub struct TypeParser {
    known_type_ids: HashSet<TypeId>,
    processed_types: HashMap<TypeId, TypeDeclaration>,
}

impl TypeParser {
    /// Creates new type parser.
    pub fn new() -> Self {
        Self {
            known_type_ids: HashSet::new(),
            processed_types: HashMap::new(),
        }
    }

    /// Parse a `ComplexType` from a DIEs.
    pub fn parse<'dbg, T>(
        self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, T>,
        root_id: TypeId,
    ) -> ComplexType {
        let mut this = self;
        this.parse_inner(ctx_die, root_id);
        ComplexType {
            types: this.processed_types,
            root: root_id,
        }
    }

    fn parse_inner<T>(&mut self, ctx_die: ContextualDieRef<'_, '_, T>, type_ref: DieRef) {
        // guard from recursion types parsing
        if self.known_type_ids.contains(&type_ref) {
            return;
        }
        self.known_type_ids.insert(type_ref);

        let mb_type_die = ctx_die.debug_info.deref_die(ctx_die.unit(), type_ref);

        let type_decl = mb_type_die.and_then(|(entry, unit)| match &entry.die {
            DieVariant::BaseType(die) => Some(self.parse_base_type(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::StructType(die) => Some(self.parse_struct(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::ArrayType(die) => Some(self.parse_array(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::EnumType(die) => Some(self.parse_enum(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::PointerType(die) => Some(self.parse_pointer(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::UnionTypeDie(die) => Some(self.parse_union(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::Subroutine(die) => Some(self.parse_subroutine(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::TypeDef(die) => Some(self.parse_typedef(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::ConstType(die) => Some(self.parse_const_type(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::Atomic(die) => Some(self.parse_atomic(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::Volatile(die) => Some(self.parse_volatile(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            DieVariant::Restrict(die) => Some(self.parse_restrict(ContextualDieRef {
                debug_info: ctx_die.debug_info,
                unit_idx: unit.idx(),
                node: &entry.node,
                die,
            })),
            _ => {
                warn!("unsupported type die: {:?}", entry.die);
                None
            }
        });
        if let Some(type_decl) = type_decl {
            self.processed_types.insert(type_ref, type_decl);
        }
    }

    fn parse_base_type(
        &mut self,
        ctx_die: ContextualDieRef<'_, '_, BaseTypeDie>,
    ) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();
        TypeDeclaration::Scalar(ScalarType {
            namespaces: ctx_die.namespaces(),
            name,
            byte_size: ctx_die.die.byte_size,
            encoding: ctx_die.die.encoding,
        })
    }

    fn parse_array<'dbg>(
        &mut self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, ArrayDie>,
    ) -> TypeDeclaration {
        let mb_type_ref = ctx_die.die.type_ref;
        if let Some(reference) = mb_type_ref {
            self.parse_inner(ctx_die, reference);
        }

        let subrange = ctx_die.node.children.iter().find_map(|&child_idx| {
            let entry = ctx_resolve_unit_call!(ctx_die, entry, child_idx);
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
            ctx_die.namespaces(),
            ctx_die.die.byte_size,
            mb_type_ref,
            lower_bound,
            upper_bound,
        ))
    }

    /// Convert DW_TAG_structure_type into TypeDeclaration.
    /// In rust DW_TAG_structure_type DIE can be interpreter as enum, see https://github.com/rust-lang/rust/issues/32920
    fn parse_struct<'dbg>(
        &mut self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, StructTypeDie>,
    ) -> TypeDeclaration {
        let is_enum = ctx_die.node.children.iter().any(|c_idx| {
            let entry = ctx_resolve_unit_call!(ctx_die, entry, *c_idx);
            matches!(entry.die, DieVariant::VariantPart(_))
        });

        if is_enum {
            self.parse_struct_enum(ctx_die)
        } else {
            self.parse_struct_struct(ctx_die)
        }
    }

    fn parse_struct_struct(
        &mut self,
        ctx_die: ContextualDieRef<'_, '_, StructTypeDie>,
    ) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();
        let members = ctx_die
            .node
            .children
            .iter()
            .filter_map(|child_idx| {
                let entry = ctx_resolve_unit_call!(ctx_die, entry, *child_idx);
                if let DieVariant::TypeMember(member) = &entry.die {
                    return Some(self.parse_member(ContextualDieRef {
                        debug_info: ctx_die.debug_info,
                        unit_idx: ctx_die.unit_idx,
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
                let entry = ctx_resolve_unit_call!(ctx_die, entry, *child_idx);
                if let DieVariant::TemplateType(param) = &entry.die {
                    let name = param.base_attributes.name.clone()?;
                    self.parse_inner(ctx_die, param.type_ref?);
                    return Some((name, param.type_ref));
                }
                None
            })
            .collect::<HashMap<_, _>>();

        TypeDeclaration::Structure {
            namespaces: ctx_die.namespaces(),
            name,
            byte_size: ctx_die.die.byte_size,
            members,
            type_params,
        }
    }

    fn parse_member<'dbg>(
        &mut self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, TypeMemberDie>,
    ) -> StructureMember {
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

        let mb_type_ref = ctx_die.die.type_ref;
        if let Some(reference) = mb_type_ref {
            self.parse_inner(ctx_die, reference);
        }

        StructureMember {
            in_struct_location,
            name: ctx_die.die.base_attributes.name.clone(),
            type_ref: mb_type_ref,
        }
    }

    fn parse_struct_enum(
        &mut self,
        ctx_die: ContextualDieRef<'_, '_, StructTypeDie>,
    ) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();

        let variant_part = ctx_die.node.children.iter().find_map(|c_idx| {
            let entry = ctx_resolve_unit_call!(ctx_die, entry, *c_idx);
            if let DieVariant::VariantPart(ref v) = entry.die {
                return Some((v, &entry.node));
            }
            None
        });

        let mut member_from_ref = |type_ref: DieRef| -> Option<StructureMember> {
            let (entry, unit) = ctx_die.debug_info.deref_die(ctx_die.unit(), type_ref)?;

            if let DieVariant::TypeMember(ref member) = &entry.die {
                return Some(self.parse_member(ContextualDieRef {
                    debug_info: ctx_die.debug_info,
                    unit_idx: unit.idx(),
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
                .and_then(&mut member_from_ref)
                .or_else(|| variant.type_ref.and_then(&mut member_from_ref))
        });

        let variants = variant_part
            .map(|vp| {
                let node = vp.1;
                node.children
                    .iter()
                    .filter_map(|idx| {
                        let entry = ctx_resolve_unit_call!(ctx_die, entry, *idx);
                        if let DieVariant::Variant(ref v) = entry.die {
                            return Some((v, &entry.node));
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
                    let entry = ctx_resolve_unit_call!(ctx_die, entry, c_idx);
                    if let DieVariant::TypeMember(ref member) = entry.die {
                        return Some(self.parse_member(ContextualDieRef {
                            debug_info: ctx_die.debug_info,
                            unit_idx: ctx_die.unit_idx,
                            node: &entry.node,
                            die: member,
                        }));
                    }
                    None
                })?;
                Some((variant.discr_value, member))
            })
            .collect::<HashMap<_, _>>();

        TypeDeclaration::RustEnum {
            namespaces: ctx_die.namespaces(),
            name,
            byte_size: ctx_die.die.byte_size,
            discr_type: discr_type.map(Box::new),
            enumerators,
        }
    }

    fn parse_enum<'dbg>(
        &mut self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, EnumTypeDie>,
    ) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();

        let mb_discr_type = ctx_die.die.type_ref;
        if let Some(reference) = mb_discr_type {
            self.parse_inner(ctx_die, reference);
        }

        let enumerators = ctx_die
            .node
            .children
            .iter()
            .filter_map(|&child_idx| {
                let entry = ctx_resolve_unit_call!(ctx_die, entry, child_idx);
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
            namespaces: ctx_die.namespaces(),
            name,
            byte_size: ctx_die.die.byte_size,
            discr_type: mb_discr_type,
            enumerators,
        }
    }

    fn parse_union(&mut self, ctx_die: ContextualDieRef<'_, '_, UnionTypeDie>) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();
        let members = ctx_die
            .node
            .children
            .iter()
            .filter_map(|child_idx| {
                let entry = ctx_resolve_unit_call!(ctx_die, entry, *child_idx);
                if let DieVariant::TypeMember(member) = &entry.die {
                    return Some(self.parse_member(ContextualDieRef {
                        debug_info: ctx_die.debug_info,
                        unit_idx: ctx_die.unit_idx,
                        node: &entry.node,
                        die: member,
                    }));
                }
                None
            })
            .collect::<Vec<_>>();

        TypeDeclaration::Union {
            namespaces: ctx_die.namespaces(),
            name,
            byte_size: ctx_die.die.byte_size,
            members,
        }
    }

    fn parse_pointer<'dbg>(
        &mut self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, PointerType>,
    ) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();

        let mb_type_ref = ctx_die.die.type_ref;
        if let Some(reference) = mb_type_ref {
            self.parse_inner(ctx_die, reference);
        }

        TypeDeclaration::Pointer {
            namespaces: ctx_die.namespaces(),
            name,
            target_type: mb_type_ref,
        }
    }

    fn parse_subroutine<'dbg>(
        &mut self,
        ctx_die: ContextualDieRef<'dbg, 'dbg, SubroutineDie>,
    ) -> TypeDeclaration {
        let name = ctx_die.die.base_attributes.name.clone();
        let mb_ret_type_ref = ctx_die.die.return_type_ref;
        if let Some(reference) = mb_ret_type_ref {
            self.parse_inner(ctx_die, reference);
        }

        TypeDeclaration::Subroutine {
            namespaces: ctx_die.namespaces(),
            name,
            return_type: mb_ret_type_ref,
        }
    }
}

macro_rules! parse_modifier_fn {
    ($fn_name: ident, $die: ty, $modifier: expr) => {
        fn $fn_name<'dbg>(
            &mut self,
            ctx_die: ContextualDieRef<'dbg, 'dbg, $die>,
        ) -> TypeDeclaration {
            let name = ctx_die.die.base_attributes.name.clone();
            let mb_type_ref = ctx_die.die.type_ref;
            if let Some(inner_type) = mb_type_ref {
                self.parse_inner(ctx_die, inner_type);
            }

            TypeDeclaration::ModifiedType {
                namespaces: ctx_die.namespaces(),
                modifier: $modifier,
                name,
                inner: mb_type_ref,
            }
        }
    };
}

// create parsers for type modifiers
impl TypeParser {
    parse_modifier_fn!(parse_typedef, TypeDefDie, CModifier::TypeDef);
    parse_modifier_fn!(parse_const_type, ConstTypeDie, CModifier::Const);
    parse_modifier_fn!(parse_atomic, AtomicDie, CModifier::Atomic);
    parse_modifier_fn!(parse_volatile, VolatileDie, CModifier::Volatile);
    parse_modifier_fn!(parse_restrict, RestrictDie, CModifier::Restrict);
}

/// A cache structure for types.
/// Every type identified by its `TypeId` and DWARF unit uuid.
pub type TypeCache = HashMap<(Uuid, TypeId), Rc<ComplexType>>;
