use crate::debugger::debugee::dwarf::r#type::{
    ArrayType, CModifier, EvaluationContext, ScalarType, StructureMember, TypeIdentity,
};
use crate::debugger::debugee::dwarf::r#type::{ComplexType, TypeDeclaration};
use crate::debugger::debugee::dwarf::{AsAllocatedData, ContextualDieRef, NamespaceHierarchy};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::select::{Literal, LiteralOrWildcard, ObjectBinaryRepr};
use crate::debugger::variable::specialization::{
    HashSetVariable, StrVariable, StringVariable, VariableParserExtension,
};
use crate::{debugger, version_switch, weak_error};
use bytes::Bytes;
use gimli::{
    DW_ATE_ASCII, DW_ATE_UTF, DW_ATE_address, DW_ATE_boolean, DW_ATE_float, DW_ATE_signed,
    DW_ATE_signed_char, DW_ATE_unsigned, DW_ATE_unsigned_char,
};
use log::warn;
pub use specialization::SpecializedVariableIR;
use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display, Formatter};
use std::string::FromUtf8Error;
use uuid::Uuid;

pub mod render;
pub mod select;
mod specialization;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AssumeError {
    #[error("field `{0}` not found")]
    FieldNotFound(&'static str),
    #[error("field `{0}` not a number")]
    FieldNotANumber(&'static str),
    #[error("incomplete interpretation of `{0}`")]
    IncompleteInterp(&'static str),
    #[error("not data for {0}")]
    NoData(&'static str),
    #[error("not type for {0}")]
    NoType(&'static str),
    #[error("underline data not a string")]
    DataNotAString(#[from] FromUtf8Error),
    #[error("undefined size of type `{0}`")]
    UnknownSize(String),
    #[error("type parameter `{0}` not found")]
    TypeParameterNotFound(&'static str),
    #[error("unknown type for type parameter `{0}`")]
    TypeParameterTypeNotFound(&'static str),
    #[error("unexpected type for {0}")]
    UnexpectedType(&'static str),
    #[error("unexpected binary representation of {0}, expect {1} got {2} bytes")]
    UnexpectedBinaryRepr(&'static str, usize, usize),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParsingError {
    #[error(transparent)]
    Assume(#[from] AssumeError),
    #[error("unsupported language version")]
    UnsupportedVersion,
    #[error("error while reading from debugee memory: {0}")]
    ReadDebugeeMemory(#[from] nix::Error),
}

/// Identifier of debugee variables.
/// Consists of the name and namespace of the variable.
#[derive(Clone, Default, PartialEq)]
pub struct VariableIdentity {
    namespace: NamespaceHierarchy,
    pub name: Option<String>,
}

impl VariableIdentity {
    pub fn new(namespace: NamespaceHierarchy, name: Option<String>) -> Self {
        Self { namespace, name }
    }

    pub fn from_variable_die(var: &ContextualDieRef<impl AsAllocatedData>) -> Self {
        Self::new(var.namespaces(), var.die.name().map(String::from))
    }

    pub fn no_namespace(name: Option<String>) -> Self {
        Self {
            namespace: NamespaceHierarchy::default(),
            name,
        }
    }
}

impl Display for VariableIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let namespaces = if self.namespace.is_empty() {
            String::default()
        } else {
            self.namespace.join("::") + "::"
        };

        match self.name.as_deref() {
            None => f.write_fmt(format_args!("{namespaces}{{unknown}}")),
            Some(mut name) => {
                if name.starts_with("__") {
                    let mb_num = name.trim_start_matches('_');
                    if mb_num.parse::<u64>().is_ok() {
                        name = mb_num
                    }
                }

                f.write_fmt(format_args!("{namespaces}{name}"))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SupportedScalar {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    Isize(isize),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    Usize(usize),
    F32(f32),
    F64(f64),
    Bool(bool),
    Char(char),
    Empty(),
}

impl Display for SupportedScalar {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedScalar::I8(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I16(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I32(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I64(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I128(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Isize(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U8(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U16(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U32(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U64(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U128(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Usize(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::F32(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::F64(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Bool(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Char(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Empty() => f.write_str("()"),
        }
    }
}

impl SupportedScalar {
    fn equal_with_literal(&self, lhs: &Literal) -> bool {
        match self {
            SupportedScalar::I8(i) => lhs.equal_with_int(*i as i64),
            SupportedScalar::I16(i) => lhs.equal_with_int(*i as i64),
            SupportedScalar::I32(i) => lhs.equal_with_int(*i as i64),
            SupportedScalar::I64(i) => lhs.equal_with_int(*i),
            SupportedScalar::I128(i) => lhs.equal_with_int(*i as i64),
            SupportedScalar::Isize(i) => lhs.equal_with_int(*i as i64),
            SupportedScalar::U8(u) => lhs.equal_with_int(*u as i64),
            SupportedScalar::U16(u) => lhs.equal_with_int(*u as i64),
            SupportedScalar::U32(u) => lhs.equal_with_int(*u as i64),
            SupportedScalar::U64(u) => lhs.equal_with_int(*u as i64),
            SupportedScalar::U128(u) => lhs.equal_with_int(*u as i64),
            SupportedScalar::Usize(u) => lhs.equal_with_int(*u as i64),
            SupportedScalar::F32(f) => lhs.equal_with_float(*f as f64),
            SupportedScalar::F64(f) => lhs.equal_with_float(*f),
            SupportedScalar::Bool(b) => lhs.equal_with_bool(*b),
            SupportedScalar::Char(c) => lhs.equal_with_string(&c.to_string()),
            SupportedScalar::Empty() => false,
        }
    }
}

/// Represents scalars: integer's, float's, bool, char and () types.
#[derive(Clone, PartialEq)]
pub struct ScalarVariable {
    pub identity: VariableIdentity,
    pub value: Option<SupportedScalar>,
    pub raw_address: Option<usize>,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
}

impl ScalarVariable {
    fn try_as_number(&self) -> Option<i64> {
        match self.value {
            Some(SupportedScalar::I8(num)) => Some(num as i64),
            Some(SupportedScalar::I16(num)) => Some(num as i64),
            Some(SupportedScalar::I32(num)) => Some(num as i64),
            Some(SupportedScalar::I64(num)) => Some(num),
            Some(SupportedScalar::Isize(num)) => Some(num as i64),
            Some(SupportedScalar::U8(num)) => Some(num as i64),
            Some(SupportedScalar::U16(num)) => Some(num as i64),
            Some(SupportedScalar::U32(num)) => Some(num as i64),
            Some(SupportedScalar::U64(num)) => Some(num as i64),
            Some(SupportedScalar::Usize(num)) => Some(num as i64),
            _ => None,
        }
    }
}

/// Represents structures.
#[derive(Clone, Default, PartialEq)]
pub struct StructVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
    /// Structure members. Each represents by variable IR.
    pub members: Vec<VariableIR>,
    /// Map of type parameters of a structure type.
    pub type_params: HashMap<String, Option<TypeIdentity>>,
    pub raw_address: Option<usize>,
}

/// Represents arrays.
#[derive(Clone, PartialEq)]
pub struct ArrayVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
    /// Array items. Each represents by variable IR.
    pub items: Option<Vec<VariableIR>>,
    pub raw_address: Option<usize>,
}

impl ArrayVariable {
    fn slice(&mut self, left: Option<usize>, right: Option<usize>) {
        if let Some(items) = self.items.as_mut() {
            if let Some(left) = left {
                items.drain(..left);
            }

            if let Some(right) = right {
                let remove_range = right - left.unwrap_or_default()..;
                if remove_range.start < items.len() {
                    items.drain(remove_range);
                };
            }
        }
    }
}

/// Simple c-style enums (each option in which does not contain the underlying values).
#[derive(Clone, PartialEq)]
pub struct CEnumVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
    /// String representation of selected variant.
    pub value: Option<String>,
    pub raw_address: Option<usize>,
}

/// Represents all enum's that more complex then c-style enums.
#[derive(Clone, PartialEq)]
pub struct RustEnumVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
    /// Variable IR representation of selected variant.
    pub value: Option<Box<VariableIR>>,
    pub raw_address: Option<usize>,
}

/// Raw pointers, references, Box.
#[derive(Clone, PartialEq)]
pub struct PointerVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
    /// Raw pointer to underline value.
    pub value: Option<*const ()>,
    /// Underline type identity.
    pub target_type: Option<TypeIdentity>,
    pub target_type_size: Option<u64>,
    pub raw_address: Option<usize>,
}

impl PointerVariable {
    /// Dereference pointer and return variable IR that represents underline value.
    pub fn deref(
        &self,
        eval_ctx: &EvaluationContext,
        parser: &VariableParser,
    ) -> Option<VariableIR> {
        let target_type = self.target_type?;
        let deref_size = self
            .target_type_size
            .or_else(|| parser.r#type.type_size_in_bytes(eval_ctx, target_type));

        let target_type_decl = parser.r#type.types.get(&target_type);
        if matches!(target_type_decl, Some(TypeDeclaration::Subroutine { .. })) {
            // this variable is a fn pointer - don't deref it
            return None;
        }

        self.value.and_then(|ptr| {
            let data = deref_size.and_then(|sz| {
                let raw_data = debugger::read_memory_by_pid(
                    eval_ctx.expl_ctx.pid_on_focus(),
                    ptr as usize,
                    sz as usize,
                )
                .ok()?;

                Some(ObjectBinaryRepr {
                    raw_data: Bytes::from(raw_data),
                    address: Some(ptr as usize),
                    size: sz as usize,
                })
            });
            let mut identity = self.identity.clone();
            identity.name = identity.name.map(|n| format!("*{n}"));
            parser.parse_inner(eval_ctx, identity, data, target_type)
        })
    }

    /// Interpret pointer as a pointer on first array element. Returns variable IR that represents
    /// an array.
    pub fn slice(
        &self,
        eval_ctx: &EvaluationContext,
        parser: &VariableParser,
        left: Option<usize>,
        right: usize,
    ) -> Option<VariableIR> {
        let target_type = self.target_type?;
        let deref_size = parser.r#type.type_size_in_bytes(eval_ctx, target_type)? as usize;

        self.value.and_then(|ptr| {
            let left = left.unwrap_or_default();
            let base_addr = ptr as usize + deref_size * left;
            let raw_data = weak_error!(debugger::read_memory_by_pid(
                eval_ctx.expl_ctx.pid_on_focus(),
                base_addr,
                deref_size * (right - left)
            ))?;
            let raw_data = bytes::Bytes::from(raw_data);
            let mut identity = self.identity.clone();
            identity.name = identity.name.map(|n| format!("[*{n}]"));

            let items = raw_data
                .chunks(deref_size)
                .enumerate()
                .filter_map(|(i, chunk)| {
                    let data = ObjectBinaryRepr {
                        raw_data: raw_data.slice_ref(chunk),
                        address: Some(base_addr + (i * deref_size)),
                        size: deref_size,
                    };
                    parser.parse_inner(
                        eval_ctx,
                        VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                        Some(data),
                        target_type,
                    )
                })
                .collect::<Vec<_>>();

            Some(VariableIR::Array(ArrayVariable {
                identity,
                items: Some(items),
                type_id: None,
                type_name: parser
                    .r#type
                    .type_name(target_type)
                    .map(|t| format!("[{t}]")),
                raw_address: Some(base_addr),
            }))
        })
    }
}

/// Represents subroutine.
#[derive(Clone, PartialEq)]
pub struct SubroutineVariable {
    pub identity: VariableIdentity,
    pub type_id: Option<TypeIdentity>,
    pub return_type_name: Option<String>,
    pub address: Option<usize>,
}

/// Represent a variable with C modifiers (volatile, const, typedef, etc)
#[derive(Clone, PartialEq)]
pub struct CModifiedVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub type_id: Option<TypeIdentity>,
    pub modifier: CModifier,
    pub value: Option<Box<VariableIR>>,
    pub address: Option<usize>,
}

/// Variable intermediate representation.
#[derive(Clone, PartialEq)]
pub enum VariableIR {
    Scalar(ScalarVariable),
    Struct(StructVariable),
    Array(ArrayVariable),
    CEnum(CEnumVariable),
    RustEnum(RustEnumVariable),
    Pointer(PointerVariable),
    Subroutine(SubroutineVariable),
    Specialized(SpecializedVariableIR),
    CModifiedVariable(CModifiedVariable),
}

// SAFETY: this enum may contain a raw pointers on memory in a debugee process,
// it is safe to dereference it using public API of *Variable structures.
unsafe impl Send for VariableIR {}

impl Debug for VariableIR {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name())
    }
}

impl VariableIR {
    /// Return address in debugee memory for variable data.
    pub fn in_memory_location(&self) -> Option<usize> {
        match self {
            VariableIR::Scalar(s) => s.raw_address,
            VariableIR::Struct(s) => s.raw_address,
            VariableIR::Array(a) => a.raw_address,
            VariableIR::CEnum(ce) => ce.raw_address,
            VariableIR::RustEnum(re) => re.raw_address,
            VariableIR::Pointer(p) => p.raw_address,
            VariableIR::Subroutine(s) => s.address,
            VariableIR::Specialized(s) => s.in_memory_location(),
            VariableIR::CModifiedVariable(cmv) => cmv.address,
        }
    }

    pub fn type_identity(&self) -> Option<TypeIdentity> {
        match self {
            VariableIR::Scalar(s) => s.type_id,
            VariableIR::Struct(s) => s.type_id,
            VariableIR::Array(a) => a.type_id,
            VariableIR::CEnum(ce) => ce.type_id,
            VariableIR::RustEnum(re) => re.type_id,
            VariableIR::Pointer(p) => p.type_id,
            VariableIR::Subroutine(s) => s.type_id,
            VariableIR::Specialized(s) => s.type_id(),
            VariableIR::CModifiedVariable(cmv) => cmv.type_id,
        }
    }

    /// Visit variable children in BFS order.
    fn bfs_iterator(&self) -> BfsIterator {
        BfsIterator {
            queue: VecDeque::from([self]),
        }
    }

    /// Returns i64 value representation or error if cast fail.
    fn assume_field_as_scalar_number(&self, field_name: &'static str) -> Result<i64, AssumeError> {
        let ir = self
            .bfs_iterator()
            .find(|child| child.name() == field_name)
            .ok_or(AssumeError::FieldNotFound(field_name))?;
        if let VariableIR::Scalar(s) = ir {
            Ok(s.try_as_number()
                .ok_or(AssumeError::FieldNotANumber(field_name))?)
        } else {
            Err(AssumeError::FieldNotANumber(field_name))
        }
    }

    /// Returns value as raw pointer or error if cast fail.
    fn assume_field_as_pointer(&self, field_name: &'static str) -> Result<*const (), AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Pointer(pointer) = child {
                    if pointer.identity.name.as_deref()? == field_name {
                        return pointer.value;
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    /// Returns value as enum or error if cast fail.
    fn assume_field_as_rust_enum(
        &self,
        field_name: &'static str,
    ) -> Result<RustEnumVariable, AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::RustEnum(r_enum) = child {
                    if r_enum.identity.name.as_deref()? == field_name {
                        return Some(r_enum.clone());
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    /// Returns value as structure or error if cast fail.
    fn assume_field_as_struct(
        &self,
        field_name: &'static str,
    ) -> Result<StructVariable, AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Struct(structure) = child {
                    if structure.identity.name.as_deref()? == field_name {
                        return Some(structure.clone());
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("structure"))
    }

    /// Returns variable identity.
    fn identity(&self) -> &VariableIdentity {
        match self {
            VariableIR::Scalar(s) => &s.identity,
            VariableIR::Struct(s) => &s.identity,
            VariableIR::Array(a) => &a.identity,
            VariableIR::CEnum(e) => &e.identity,
            VariableIR::RustEnum(e) => &e.identity,
            VariableIR::Pointer(p) => &p.identity,
            VariableIR::Specialized(s) => match s {
                SpecializedVariableIR::Vector { original, .. } => &original.identity,
                SpecializedVariableIR::VecDeque { original, .. } => &original.identity,
                SpecializedVariableIR::String { original, .. } => &original.identity,
                SpecializedVariableIR::Str { original, .. } => &original.identity,
                SpecializedVariableIR::Tls { original, .. } => &original.identity,
                SpecializedVariableIR::HashMap { original, .. } => &original.identity,
                SpecializedVariableIR::HashSet { original, .. } => &original.identity,
                SpecializedVariableIR::BTreeMap { original, .. } => &original.identity,
                SpecializedVariableIR::BTreeSet { original, .. } => &original.identity,
                SpecializedVariableIR::Cell { original, .. } => &original.identity,
                SpecializedVariableIR::RefCell { original, .. } => &original.identity,
                SpecializedVariableIR::Rc { original, .. } => &original.identity,
                SpecializedVariableIR::Arc { original, .. } => &original.identity,
                SpecializedVariableIR::Uuid { original, .. } => &original.identity,
            },
            VariableIR::Subroutine(s) => &s.identity,
            VariableIR::CModifiedVariable(v) => &v.identity,
        }
    }

    pub fn identity_mut(&mut self) -> &mut VariableIdentity {
        match self {
            VariableIR::Scalar(s) => &mut s.identity,
            VariableIR::Struct(s) => &mut s.identity,
            VariableIR::Array(a) => &mut a.identity,
            VariableIR::CEnum(e) => &mut e.identity,
            VariableIR::RustEnum(e) => &mut e.identity,
            VariableIR::Pointer(p) => &mut p.identity,
            VariableIR::Specialized(s) => match s {
                SpecializedVariableIR::Vector { original, .. } => &mut original.identity,
                SpecializedVariableIR::VecDeque { original, .. } => &mut original.identity,
                SpecializedVariableIR::String { original, .. } => &mut original.identity,
                SpecializedVariableIR::Str { original, .. } => &mut original.identity,
                SpecializedVariableIR::Tls { original, .. } => &mut original.identity,
                SpecializedVariableIR::HashMap { original, .. } => &mut original.identity,
                SpecializedVariableIR::HashSet { original, .. } => &mut original.identity,
                SpecializedVariableIR::BTreeMap { original, .. } => &mut original.identity,
                SpecializedVariableIR::BTreeSet { original, .. } => &mut original.identity,
                SpecializedVariableIR::Cell { original, .. } => &mut original.identity,
                SpecializedVariableIR::RefCell { original, .. } => &mut original.identity,
                SpecializedVariableIR::Rc { original, .. } => &mut original.identity,
                SpecializedVariableIR::Arc { original, .. } => &mut original.identity,
                SpecializedVariableIR::Uuid { original, .. } => &mut original.identity,
            },
            VariableIR::Subroutine(s) => &mut s.identity,
            VariableIR::CModifiedVariable(v) => &mut v.identity,
        }
    }

    /// If a variable has a specialized ir (vectors, strings, etc.) then return
    /// an underlying structure
    fn canonic(self) -> Self {
        let VariableIR::Specialized(spec) = self else {
            return self;
        };

        match spec {
            SpecializedVariableIR::Vector { original, .. }
            | SpecializedVariableIR::VecDeque { original, .. }
            | SpecializedVariableIR::HashMap { original, .. }
            | SpecializedVariableIR::HashSet { original, .. }
            | SpecializedVariableIR::BTreeMap { original, .. }
            | SpecializedVariableIR::BTreeSet { original, .. }
            | SpecializedVariableIR::String { original, .. }
            | SpecializedVariableIR::Str { original, .. }
            | SpecializedVariableIR::Tls { original, .. }
            | SpecializedVariableIR::Cell { original, .. }
            | SpecializedVariableIR::RefCell { original, .. }
            | SpecializedVariableIR::Rc { original, .. }
            | SpecializedVariableIR::Arc { original, .. }
            | SpecializedVariableIR::Uuid { original, .. } => VariableIR::Struct(original),
        }
    }

    /// Try to dereference variable and returns underline variable IR.
    /// Return `None` if dereference not allowed.
    fn deref(self, eval_ctx: &EvaluationContext, variable_parser: &VariableParser) -> Option<Self> {
        match self {
            VariableIR::Pointer(ptr) => ptr.deref(eval_ctx, variable_parser),
            VariableIR::RustEnum(r_enum) => r_enum
                .value
                .and_then(|v| v.deref(eval_ctx, variable_parser)),
            VariableIR::Specialized(SpecializedVariableIR::Rc { value, .. })
            | VariableIR::Specialized(SpecializedVariableIR::Arc { value, .. }) => {
                value.and_then(|var| var.deref(eval_ctx, variable_parser))
            }
            VariableIR::Specialized(SpecializedVariableIR::Tls { tls_var, .. }) => tls_var
                .and_then(|var| {
                    var.inner_value
                        .and_then(|inner| inner.deref(eval_ctx, variable_parser))
                }),
            _ => None,
        }
    }

    /// Return address (as pointer variable) of raw data in debugee memory.
    fn address(
        self,
        eval_ctx: &EvaluationContext,
        variable_parser: &VariableParser,
    ) -> Option<Self> {
        let addr = self.in_memory_location()?;
        Some(VariableIR::Pointer(PointerVariable {
            identity: VariableIdentity::no_namespace(None),
            type_name: Some(format!("&{}", self.r#type())),
            value: Some(addr as *const ()),
            target_type: self.type_identity(),
            target_type_size: self
                .type_identity()
                .and_then(|t| variable_parser.r#type.type_size_in_bytes(eval_ctx, t)),
            raw_address: None,
            type_id: None,
        }))
    }

    /// Return variable field, `None` if get field is not allowed for variable type.
    /// Supported: structures, rust-style enums, hashmaps, btree-maps.
    fn field(self, field_name: &str) -> Option<Self> {
        match self {
            VariableIR::Struct(structure) => structure
                .members
                .into_iter()
                .find(|member| field_name == member.name()),
            VariableIR::RustEnum(r_enum) => r_enum.value.and_then(|v| v.field(field_name)),
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::HashMap { map, .. } => map.and_then(|map| {
                    map.kv_items.into_iter().find_map(|(key, value)| match key {
                        VariableIR::Specialized(spec) => match spec {
                            SpecializedVariableIR::String {
                                string: string_key, ..
                            } => string_key.and_then(|string| {
                                (string.value == field_name)
                                    .then(|| value.clone_and_rename(&string.value))
                            }),
                            SpecializedVariableIR::Str {
                                string: string_key, ..
                            } => string_key.and_then(|str| {
                                (str.value == field_name)
                                    .then(|| value.clone_and_rename(&str.value))
                            }),
                            _ => None,
                        },
                        _ => None,
                    })
                }),
                SpecializedVariableIR::Tls { tls_var, .. } => tls_var
                    .and_then(|var| var.inner_value.and_then(|inner| inner.field(field_name))),
                SpecializedVariableIR::Cell { value, .. }
                | SpecializedVariableIR::RefCell { value, .. } => {
                    value.and_then(|var| var.field(field_name))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Return variable element by its index, `None` if indexing is not allowed for a variable type.
    /// Supported: array, rust-style enums, vector, hashmap, hashset, btreemap, btreeset.
    fn index(self, idx: &Literal) -> Option<Self> {
        match self {
            VariableIR::Array(array) => array.items.and_then(|mut items| {
                if let Literal::Int(idx) = idx {
                    let idx = *idx as usize;
                    if idx < items.len() {
                        return Some(items.swap_remove(idx));
                    }
                }
                None
            }),
            VariableIR::RustEnum(r_enum) => r_enum.value.and_then(|v| v.index(idx)),
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, .. }
                | SpecializedVariableIR::VecDeque { vec, .. } => vec.and_then(|mut v| {
                    let inner_array = v.structure.members.swap_remove(0);
                    inner_array.index(idx)
                }),
                SpecializedVariableIR::Tls { tls_var, .. } => {
                    tls_var.and_then(|var| var.inner_value.and_then(|inner| inner.index(idx)))
                }
                SpecializedVariableIR::Cell { value, .. }
                | SpecializedVariableIR::RefCell { value, .. } => {
                    value.and_then(|var| var.index(idx))
                }
                SpecializedVariableIR::BTreeMap { map: Some(map), .. }
                | SpecializedVariableIR::HashMap { map: Some(map), .. } => {
                    for (k, mut v) in map.kv_items {
                        if k.match_literal(idx) {
                            let identity = v.identity_mut();
                            identity.name = Some("value".to_string());
                            return Some(v);
                        }
                    }

                    None
                }
                SpecializedVariableIR::BTreeSet { set: Some(set), .. }
                | SpecializedVariableIR::HashSet { set: Some(set), .. } => {
                    let found = set.items.into_iter().any(|it| it.match_literal(idx));

                    Some(VariableIR::Scalar(ScalarVariable {
                        identity: VariableIdentity::no_namespace(Some("contains".to_string())),
                        type_id: None,
                        type_name: Some("bool".to_string()),
                        value: Some(SupportedScalar::Bool(found)),
                        raw_address: None,
                    }))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn slice(
        mut self,
        eval_ctx: &EvaluationContext,
        variable_parser: &VariableParser,
        left: Option<usize>,
        right: Option<usize>,
    ) -> Option<Self> {
        match &mut self {
            VariableIR::Array(array) => {
                array.slice(left, right);
                Some(self)
            }
            VariableIR::Pointer(ptr) => {
                // for pointer the right bound must always be specified
                let right = right?;
                ptr.slice(eval_ctx, variable_parser, left, right)
            }
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Rc { value, .. }
                | SpecializedVariableIR::Arc { value, .. } => {
                    let ptr = value.as_ref()?;
                    // for pointer the right bound must always be specified
                    let right = right?;
                    ptr.slice(eval_ctx, variable_parser, left, right)
                }
                SpecializedVariableIR::Vector { vec, .. }
                | SpecializedVariableIR::VecDeque { vec, .. } => {
                    let vec = vec.as_mut()?;
                    vec.slice(left, right);
                    Some(self)
                }
                SpecializedVariableIR::Tls { tls_var, .. } => {
                    let tls_var = tls_var.take()?;
                    let inner = tls_var.inner_value?;
                    inner.slice(eval_ctx, variable_parser, left, right)
                }
                SpecializedVariableIR::Cell { value, .. }
                | SpecializedVariableIR::RefCell { value, .. } => {
                    let inner = value.take()?;
                    inner.slice(eval_ctx, variable_parser, left, right)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn clone_and_rename(&self, new_name: &str) -> Self {
        let mut clone = self.clone();
        let identity = clone.identity_mut();
        identity.name = Some(new_name.to_string());
        clone
    }

    /// Match variable with a literal object.
    /// Return true if variable matched to literal.
    fn match_literal(self, literal: &Literal) -> bool {
        match self {
            VariableIR::Scalar(ScalarVariable {
                value: Some(scalar),
                ..
            }) => scalar.equal_with_literal(literal),
            VariableIR::Pointer(PointerVariable {
                value: Some(ptr), ..
            }) => literal.equal_with_address(ptr as usize),
            VariableIR::Array(ArrayVariable {
                items: Some(items), ..
            }) => {
                let Literal::Array(arr_literal) = literal else {
                    return false;
                };
                if arr_literal.len() != items.len() {
                    return false;
                }

                for (i, item) in items.into_iter().enumerate() {
                    match &arr_literal[i] {
                        LiteralOrWildcard::Literal(lit) => {
                            if !item.match_literal(lit) {
                                return false;
                            }
                        }
                        LiteralOrWildcard::Wildcard => continue,
                    }
                }
                true
            }
            VariableIR::Struct(StructVariable { members, .. }) => {
                match literal {
                    Literal::Array(array_literal) => {
                        // structure must be a tuple
                        if array_literal.len() != members.len() {
                            return false;
                        }

                        for (i, member) in members.into_iter().enumerate() {
                            let field_literal = &array_literal[i];
                            match field_literal {
                                LiteralOrWildcard::Literal(lit) => {
                                    if !member.match_literal(lit) {
                                        return false;
                                    }
                                }
                                LiteralOrWildcard::Wildcard => continue,
                            }
                        }

                        true
                    }
                    Literal::AssocArray(struct_literal) => {
                        // default structure
                        if struct_literal.len() != members.len() {
                            return false;
                        }

                        for member in members {
                            let Some(member_name) = member.identity().name.as_ref() else {
                                return false;
                            };

                            let Some(field_literal) = struct_literal.get(member_name) else {
                                return false;
                            };

                            match field_literal {
                                LiteralOrWildcard::Literal(lit) => {
                                    if !member.match_literal(lit) {
                                        return false;
                                    }
                                }
                                LiteralOrWildcard::Wildcard => continue,
                            }
                        }
                        true
                    }
                    _ => false,
                }
            }
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::String {
                    string: Some(StringVariable { value, .. }),
                    ..
                } => literal.equal_with_string(&value),
                SpecializedVariableIR::Str {
                    string: Some(StrVariable { value, .. }),
                    ..
                } => literal.equal_with_string(&value),
                SpecializedVariableIR::Uuid {
                    value: Some(bytes), ..
                } => {
                    let uuid = Uuid::from_bytes(bytes);
                    literal.equal_with_string(&uuid.to_string())
                }
                SpecializedVariableIR::Cell { mut value, .. }
                | SpecializedVariableIR::RefCell { mut value, .. } => {
                    let Some(inner) = value.take() else {
                        return false;
                    };
                    inner.match_literal(literal)
                }
                SpecializedVariableIR::Rc {
                    value:
                        Some(PointerVariable {
                            value: Some(ptr), ..
                        }),
                    ..
                }
                | SpecializedVariableIR::Arc {
                    value:
                        Some(PointerVariable {
                            value: Some(ptr), ..
                        }),
                    ..
                } => literal.equal_with_address(ptr as usize),
                SpecializedVariableIR::Vector {
                    vec: Some(mut v), ..
                }
                | SpecializedVariableIR::VecDeque {
                    vec: Some(mut v), ..
                } => {
                    let inner_array = v.structure.members.swap_remove(0);
                    debug_assert!(matches!(inner_array, VariableIR::Array(_)));
                    inner_array.match_literal(literal)
                }
                SpecializedVariableIR::HashSet {
                    set: Some(HashSetVariable { items, .. }),
                    ..
                }
                | SpecializedVariableIR::BTreeSet {
                    set: Some(HashSetVariable { items, .. }),
                    ..
                } => {
                    let Literal::Array(arr_literal) = literal else {
                        return false;
                    };
                    if arr_literal.len() != items.len() {
                        return false;
                    }
                    let mut arr_literal = arr_literal.to_vec();

                    for item in items {
                        let mut item_found = false;

                        // try to find equals item
                        let mb_literal_idx = arr_literal.iter().position(|lit| {
                            if let LiteralOrWildcard::Literal(lit) = lit {
                                item.clone().match_literal(lit)
                            } else {
                                false
                            }
                        });
                        if let Some(literal_idx) = mb_literal_idx {
                            arr_literal.swap_remove(literal_idx);
                            item_found = true;
                        }

                        // try to find wildcard
                        if !item_found {
                            let mb_wildcard_idx = arr_literal
                                .iter()
                                .position(|lit| matches!(lit, LiteralOrWildcard::Wildcard));
                            if let Some(wildcard_idx) = mb_wildcard_idx {
                                arr_literal.swap_remove(wildcard_idx);
                                item_found = true;
                            }
                        }

                        // still not found - set aren't equal
                        if !item_found {
                            return false;
                        }
                    }
                    true
                }
                _ => false,
            },
            VariableIR::CEnum(CEnumVariable {
                value: Some(ref value),
                ..
            }) => {
                let Literal::EnumVariant(variant, None) = literal else {
                    return false;
                };
                value == variant
            }
            VariableIR::RustEnum(RustEnumVariable {
                value: Some(value), ..
            }) => {
                let Literal::EnumVariant(variant, variant_value) = literal else {
                    return false;
                };

                if value.identity().name.as_ref() != Some(variant) {
                    return false;
                }

                match variant_value {
                    None => true,
                    Some(lit) => value.match_literal(lit),
                }
            }
            _ => false,
        }
    }
}

pub struct VariableParser<'a> {
    r#type: &'a ComplexType,
}

impl<'a> VariableParser<'a> {
    pub fn new(r#type: &'a ComplexType) -> Self {
        Self { r#type }
    }

    fn parse_scalar(
        &self,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
        r#type: &ScalarType,
    ) -> ScalarVariable {
        fn render_scalar<S: Copy + Display>(data: Option<ObjectBinaryRepr>) -> Option<S> {
            data.as_ref().map(|v| scalar_from_bytes::<S>(&v.raw_data))
        }
        let in_debugee_loc = data.as_ref().and_then(|d| d.address);
        #[allow(non_upper_case_globals)]
        let value_view = r#type.encoding.and_then(|encoding| match encoding {
            DW_ATE_address => render_scalar::<usize>(data).map(SupportedScalar::Usize),
            DW_ATE_signed_char => render_scalar::<i8>(data).map(SupportedScalar::I8),
            DW_ATE_unsigned_char => render_scalar::<u8>(data).map(SupportedScalar::U8),
            DW_ATE_signed => match r#type.byte_size.unwrap_or(0) {
                0 => Some(SupportedScalar::Empty()),
                1 => render_scalar::<i8>(data).map(SupportedScalar::I8),
                2 => render_scalar::<i16>(data).map(SupportedScalar::I16),
                4 => render_scalar::<i32>(data).map(SupportedScalar::I32),
                8 => {
                    if r#type.name.as_deref() == Some("isize") {
                        render_scalar::<isize>(data).map(SupportedScalar::Isize)
                    } else {
                        render_scalar::<i64>(data).map(SupportedScalar::I64)
                    }
                }
                16 => render_scalar::<i128>(data).map(SupportedScalar::I128),
                _ => {
                    warn!(
                        "parse scalar: unexpected signed size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_unsigned => match r#type.byte_size.unwrap_or(0) {
                0 => Some(SupportedScalar::Empty()),
                1 => render_scalar::<u8>(data).map(SupportedScalar::U8),
                2 => render_scalar::<u16>(data).map(SupportedScalar::U16),
                4 => render_scalar::<u32>(data).map(SupportedScalar::U32),
                8 => {
                    if r#type.name.as_deref() == Some("usize") {
                        render_scalar::<usize>(data).map(SupportedScalar::Usize)
                    } else {
                        render_scalar::<u64>(data).map(SupportedScalar::U64)
                    }
                }
                16 => render_scalar::<u128>(data).map(SupportedScalar::U128),
                _ => {
                    warn!(
                        "parse scalar: unexpected unsigned size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_float => match r#type.byte_size.unwrap_or(0) {
                4 => render_scalar::<f32>(data).map(SupportedScalar::F32),
                8 => render_scalar::<f64>(data).map(SupportedScalar::F64),
                _ => {
                    warn!(
                        "parse scalar: unexpected float size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_boolean => render_scalar::<bool>(data).map(SupportedScalar::Bool),
            DW_ATE_UTF => render_scalar::<char>(data).map(|char| {
                // WAITFORFIX: https://github.com/rust-lang/rust/issues/113819
                // this check is meaningfully here cause in case above there is a random bytes here,
                // and it may lead to panic in other places
                // (specially when someone tries to render this char)
                if String::from_utf8(char.to_string().into_bytes()).is_err() {
                    SupportedScalar::Char('?')
                } else {
                    SupportedScalar::Char(char)
                }
            }),
            DW_ATE_ASCII => render_scalar::<char>(data).map(SupportedScalar::Char),
            _ => {
                warn!("parse scalar: unexpected base type encoding: {encoding}");
                None
            }
        });

        ScalarVariable {
            identity,
            type_name: r#type.name.clone(),
            type_id: Some(type_id),
            value: value_view,
            raw_address: in_debugee_loc,
        }
    }

    fn parse_struct_variable(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
        type_params: HashMap<String, Option<TypeIdentity>>,
        members: &[StructureMember],
    ) -> StructVariable {
        let children = members
            .iter()
            .filter_map(|member| self.parse_struct_member(eval_ctx, member, data.as_ref()))
            .collect();

        StructVariable {
            identity,
            type_id: Some(type_id),
            type_name: self.r#type.type_name(type_id),
            members: children,
            type_params,
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_struct_member(
        &self,
        eval_ctx: &EvaluationContext,
        member: &StructureMember,
        parent_data: Option<&ObjectBinaryRepr>,
    ) -> Option<VariableIR> {
        let name = member.name.clone();
        let Some(type_ref) = member.type_ref else {
            warn!(
                "parse structure: unknown type for member {}",
                name.as_deref().unwrap_or_default()
            );
            return None;
        };
        let member_val = parent_data.and_then(|data| member.value(eval_ctx, self.r#type, data));

        self.parse_inner(
            eval_ctx,
            VariableIdentity::no_namespace(member.name.clone()),
            member_val,
            type_ref,
        )
    }

    fn parse_array(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
        array_decl: &ArrayType,
    ) -> ArrayVariable {
        let items = array_decl.bounds(eval_ctx).and_then(|bounds| {
            let len = bounds.1 - bounds.0;
            let data = data.as_ref()?;
            let el_size = (array_decl.size_in_bytes(eval_ctx, self.r#type)? / len as u64) as usize;
            let bytes = &data.raw_data;
            let el_type_id = array_decl.element_type?;

            let (mut bytes_chunks, mut empty_chunks);
            let raw_items_iter: &mut dyn Iterator<Item = (usize, &[u8])> = if el_size != 0 {
                bytes_chunks = bytes.chunks(el_size).enumerate();
                &mut bytes_chunks
            } else {
                // if an item type is zst
                let v: Vec<&[u8]> = vec![&[]; len as usize];
                empty_chunks = v.into_iter().enumerate();
                &mut empty_chunks
            };

            Some(
                raw_items_iter
                    .filter_map(|(i, chunk)| {
                        let offset = i * el_size;
                        let data = ObjectBinaryRepr {
                            raw_data: bytes.slice_ref(chunk),
                            address: data.address.map(|addr| addr + offset),
                            size: el_size,
                        };

                        self.parse_inner(
                            eval_ctx,
                            VariableIdentity::no_namespace(Some(format!(
                                "{index}",
                                index = bounds.0 + i as i64
                            ))),
                            Some(data),
                            el_type_id,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        });

        ArrayVariable {
            identity,
            items,
            type_id: Some(type_id),
            type_name: self.r#type.type_name(type_id),
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_c_enum(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
        discr_type: Option<TypeIdentity>,
        enumerators: &HashMap<i64, String>,
    ) -> CEnumVariable {
        let in_debugee_loc = data.as_ref().and_then(|d| d.address);
        let mb_discr = discr_type.and_then(|type_id| {
            self.parse_inner(
                eval_ctx,
                VariableIdentity::no_namespace(None),
                data,
                type_id,
            )
        });

        let value = mb_discr.and_then(|discr| {
            if let VariableIR::Scalar(scalar) = discr {
                scalar.try_as_number()
            } else {
                None
            }
        });

        CEnumVariable {
            identity,
            type_name: self.r#type.type_name(type_id),
            type_id: Some(type_id),
            value: value.and_then(|val| enumerators.get(&val).cloned()),
            raw_address: in_debugee_loc,
        }
    }

    fn parse_rust_enum(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
        discr_member: Option<&StructureMember>,
        enumerators: &HashMap<Option<i64>, StructureMember>,
    ) -> RustEnumVariable {
        let discr_value = discr_member.and_then(|member| {
            let discr = self.parse_struct_member(eval_ctx, member, data.as_ref())?;
            if let VariableIR::Scalar(scalar) = discr {
                return scalar.try_as_number();
            }
            None
        });

        let enumerator =
            discr_value.and_then(|v| enumerators.get(&Some(v)).or_else(|| enumerators.get(&None)));

        let enumerator = enumerator.and_then(|member| {
            Some(Box::new(self.parse_struct_member(
                eval_ctx,
                member,
                data.as_ref(),
            )?))
        });

        RustEnumVariable {
            identity,
            type_id: Some(type_id),
            type_name: self.r#type.type_name(type_id),
            value: enumerator,
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_pointer(
        &self,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
        target_type: Option<TypeIdentity>,
    ) -> PointerVariable {
        let mb_ptr = data
            .as_ref()
            .map(|v| scalar_from_bytes::<*const ()>(&v.raw_data));

        PointerVariable {
            identity,
            type_id: Some(type_id),
            type_name: self.r#type.type_name(type_id).or_else(|| {
                Some(format!(
                    "*{deref_type}",
                    deref_type = self.r#type.type_name(target_type?)?
                ))
            }),
            value: mb_ptr,
            target_type,
            target_type_size: None,
            raw_address: data.and_then(|d| d.address),
        }
    }

    fn parse_inner(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        data: Option<ObjectBinaryRepr>,
        type_id: TypeIdentity,
    ) -> Option<VariableIR> {
        match &self.r#type.types[&type_id] {
            TypeDeclaration::Scalar(scalar_type) => Some(VariableIR::Scalar(self.parse_scalar(
                identity,
                data,
                type_id,
                scalar_type,
            ))),
            TypeDeclaration::Structure {
                namespaces: type_ns_h,
                members,
                type_params,
                name: struct_name,
                ..
            } => {
                let struct_var = self.parse_struct_variable(
                    eval_ctx,
                    identity,
                    data,
                    type_id,
                    type_params.clone(),
                    members,
                );

                let parser_ext = VariableParserExtension::new(self);
                // Reinterpret structure if underline data type is:
                // - Vector
                // - String
                // - &str
                // - tls variable
                // - hashmaps
                // - hashset
                // - btree map
                // - btree set
                // - vecdeque
                // - cell/refcell
                // - rc/arc
                if struct_name.as_deref() == Some("&str") {
                    return Some(VariableIR::Specialized(
                        parser_ext.parse_str(eval_ctx, struct_var),
                    ));
                };

                if struct_name.as_deref() == Some("String") {
                    return Some(VariableIR::Specialized(
                        parser_ext.parse_string(eval_ctx, struct_var),
                    ));
                };

                if struct_name.as_ref().map(|name| name.starts_with("Vec")) == Some(true)
                    && type_ns_h.contains(&["vec"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_vector(
                        eval_ctx,
                        struct_var,
                        type_params,
                    )));
                };

                let rust_version = eval_ctx.rustc_version().unwrap_or_default();
                let is_tls_type = version_switch!(
                    rust_version,
                    (1, 0, 0) ..= (1, 76, u32::MAX) => type_ns_h.contains(&["std", "sys", "common", "thread_local", "fast_local"]),
                    (1, 77, 0) ..= (1, 77, u32::MAX) => type_ns_h.contains(&["std", "sys", "pal", "common", "thread_local", "fast_local"]),
                    (1, 78, 0) ..= (1, 80, u32::MAX) => type_ns_h.contains(&["std", "sys", "thread_local", "fast_local"]),
                    (1, 81, 0) ..= (1, u32::MAX, u32::MAX) => type_ns_h.contains(&["std", "sys", "thread_local"]),
                );
                if is_tls_type == Some(true) {
                    return version_switch!(
                        rust_version,
                        (1, 0, 0) ..= (1, 79, u32::MAX) => Some(VariableIR::Specialized(parser_ext.parse_tls_old(struct_var, type_params))),
                        (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => Some(VariableIR::Specialized(parser_ext.parse_tls(struct_var, type_params)?)),
                    )
                        .expect("infallible: all rustc versions are covered");
                }

                if struct_name.as_ref().map(|name| name.starts_with("HashMap")) == Some(true)
                    && type_ns_h.contains(&["collections", "hash", "map"])
                {
                    return Some(VariableIR::Specialized(
                        parser_ext.parse_hashmap(eval_ctx, struct_var),
                    ));
                };

                if struct_name.as_ref().map(|name| name.starts_with("HashSet")) == Some(true)
                    && type_ns_h.contains(&["collections", "hash", "set"])
                {
                    return Some(VariableIR::Specialized(
                        parser_ext.parse_hashset(eval_ctx, struct_var),
                    ));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("BTreeMap"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "btree", "map"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_btree_map(
                        eval_ctx,
                        struct_var,
                        type_id,
                        type_params,
                    )));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("BTreeSet"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "btree", "set"])
                {
                    return Some(VariableIR::Specialized(
                        parser_ext.parse_btree_set(struct_var),
                    ));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("VecDeque"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "vec_deque"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_vec_dequeue(
                        eval_ctx,
                        struct_var,
                        type_params,
                    )));
                };

                if struct_name.as_ref().map(|name| name.starts_with("Cell")) == Some(true)
                    && type_ns_h.contains(&["cell"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_cell(struct_var)));
                };

                if struct_name.as_ref().map(|name| name.starts_with("RefCell")) == Some(true)
                    && type_ns_h.contains(&["cell"])
                {
                    return Some(VariableIR::Specialized(
                        parser_ext.parse_refcell(struct_var),
                    ));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("Rc<") | name.starts_with("Weak<"))
                    == Some(true)
                    && type_ns_h.contains(&["rc"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_rc(struct_var)));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("Arc<") | name.starts_with("Weak<"))
                    == Some(true)
                    && type_ns_h.contains(&["sync"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_arc(struct_var)));
                };

                if struct_name.as_ref().map(|name| name == "Uuid") == Some(true)
                    && type_ns_h.contains(&["uuid"])
                {
                    return Some(VariableIR::Specialized(parser_ext.parse_uuid(struct_var)));
                };

                Some(VariableIR::Struct(struct_var))
            }
            TypeDeclaration::Array(decl) => Some(VariableIR::Array(
                self.parse_array(eval_ctx, identity, data, type_id, decl),
            )),
            TypeDeclaration::CStyleEnum {
                discr_type,
                enumerators,
                ..
            } => Some(VariableIR::CEnum(self.parse_c_enum(
                eval_ctx,
                identity,
                data,
                type_id,
                *discr_type,
                enumerators,
            ))),
            TypeDeclaration::RustEnum {
                discr_type,
                enumerators,
                ..
            } => Some(VariableIR::RustEnum(self.parse_rust_enum(
                eval_ctx,
                identity,
                data,
                type_id,
                discr_type.as_ref().map(|t| t.as_ref()),
                enumerators,
            ))),
            TypeDeclaration::Pointer { target_type, .. } => Some(VariableIR::Pointer(
                self.parse_pointer(identity, data, type_id, *target_type),
            )),
            TypeDeclaration::Union { members, .. } => {
                let struct_var = self.parse_struct_variable(
                    eval_ctx,
                    identity,
                    data,
                    type_id,
                    HashMap::new(),
                    members,
                );
                Some(VariableIR::Struct(struct_var))
            }
            TypeDeclaration::Subroutine { return_type, .. } => {
                let ret_type = return_type.and_then(|t_id| self.r#type.type_name(t_id));
                let fn_var = SubroutineVariable {
                    identity,
                    type_id: Some(type_id),
                    return_type_name: ret_type,
                    address: data.and_then(|d| d.address),
                };
                Some(VariableIR::Subroutine(fn_var))
            }
            TypeDeclaration::ModifiedType {
                inner, modifier, ..
            } => {
                let in_debugee_loc = data.as_ref().and_then(|d| d.address);
                Some(VariableIR::CModifiedVariable(CModifiedVariable {
                    identity: identity.clone(),
                    type_id: Some(type_id),
                    type_name: self.r#type.type_name(type_id),
                    modifier: *modifier,
                    value: inner.and_then(|inner_type| {
                        Some(Box::new(
                            self.parse_inner(eval_ctx, identity, data, inner_type)?,
                        ))
                    }),
                    address: in_debugee_loc,
                }))
            }
        }
    }

    pub fn parse(
        self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<ObjectBinaryRepr>,
    ) -> Option<VariableIR> {
        self.parse_inner(eval_ctx, identity, value, self.r#type.root)
    }
}

/// Iterator for visits underline values in BFS order.
struct BfsIterator<'a> {
    queue: VecDeque<&'a VariableIR>,
}

impl<'a> Iterator for BfsIterator<'a> {
    type Item = &'a VariableIR;

    fn next(&mut self) -> Option<Self::Item> {
        let next_item = self.queue.pop_front()?;

        match next_item {
            VariableIR::Struct(r#struct) => {
                r#struct
                    .members
                    .iter()
                    .for_each(|member| self.queue.push_back(member));
            }
            VariableIR::Array(array) => {
                if let Some(items) = array.items.as_ref() {
                    items.iter().for_each(|item| self.queue.push_back(item))
                }
            }
            VariableIR::RustEnum(r#enum) => {
                if let Some(enumerator) = r#enum.value.as_ref() {
                    self.queue.push_back(enumerator)
                }
            }
            VariableIR::Pointer(_) => {}
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { original, .. }
                | SpecializedVariableIR::VecDeque { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::String { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Str { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Tls { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::HashMap { original, .. }
                | SpecializedVariableIR::BTreeMap { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::HashSet { original, .. }
                | SpecializedVariableIR::BTreeSet { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Cell { original, .. }
                | SpecializedVariableIR::RefCell { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Rc { .. } | SpecializedVariableIR::Arc { .. } => {}
                SpecializedVariableIR::Uuid { .. } => {}
            },
            _ => {}
        }

        Some(next_item)
    }
}

#[inline(never)]
fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> T {
    let ptr = bytes.as_ptr();
    unsafe { std::ptr::read_unaligned::<T>(ptr as *const T) }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::debugger::variable::specialization::VecVariable;

    #[test]
    fn test_bfs_iterator() {
        struct TestCase {
            variable: VariableIR,
            expected_order: Vec<&'static str>,
        }

        let test_cases = vec![
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: VariableIdentity::no_namespace(Some("struct_1".to_owned())),
                    type_name: None,
                    type_id: None,
                    members: vec![
                        VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::no_namespace(Some("array_1".to_owned())),
                            type_id: None,
                            type_name: None,
                            items: Some(vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                    raw_address: None,
                                    type_id: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_2".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                    raw_address: None,
                                    type_id: None,
                                }),
                            ]),
                            raw_address: None,
                        }),
                        VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::no_namespace(Some("array_2".to_owned())),
                            type_name: None,
                            type_id: None,
                            items: Some(vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_3".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                    raw_address: None,
                                    type_id: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_4".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                    raw_address: None,
                                    type_id: None,
                                }),
                            ]),
                            raw_address: None,
                        }),
                    ],
                    type_params: Default::default(),
                    raw_address: None,
                }),
                expected_order: vec![
                    "struct_1", "array_1", "array_2", "scalar_1", "scalar_2", "scalar_3",
                    "scalar_4",
                ],
            },
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: VariableIdentity::no_namespace(Some("struct_1".to_owned())),
                    type_id: None,
                    type_name: None,
                    members: vec![
                        VariableIR::Struct(StructVariable {
                            identity: VariableIdentity::no_namespace(Some("struct_2".to_owned())),
                            type_id: None,
                            type_name: None,
                            members: vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_1".to_owned(),
                                    )),
                                    type_id: None,
                                    type_name: None,
                                    value: None,
                                    raw_address: None,
                                }),
                                VariableIR::RustEnum(RustEnumVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "enum_1".to_owned(),
                                    )),
                                    type_id: None,
                                    type_name: None,
                                    value: Some(Box::new(VariableIR::Scalar(ScalarVariable {
                                        identity: VariableIdentity::no_namespace(Some(
                                            "scalar_2".to_owned(),
                                        )),
                                        type_id: None,
                                        type_name: None,
                                        value: None,
                                        raw_address: None,
                                    }))),
                                    raw_address: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_3".to_owned(),
                                    )),
                                    type_id: None,
                                    type_name: None,
                                    value: None,
                                    raw_address: None,
                                }),
                            ],
                            type_params: Default::default(),
                            raw_address: None,
                        }),
                        VariableIR::Pointer(PointerVariable {
                            identity: VariableIdentity::no_namespace(Some("pointer_1".to_owned())),
                            type_id: None,
                            type_name: None,
                            value: None,
                            target_type: None,
                            target_type_size: None,
                            raw_address: None,
                        }),
                    ],
                    type_params: Default::default(),
                    raw_address: None,
                }),
                expected_order: vec![
                    "struct_1",
                    "struct_2",
                    "pointer_1",
                    "scalar_1",
                    "enum_1",
                    "scalar_3",
                    "scalar_2",
                ],
            },
        ];

        for tc in test_cases {
            let iter = tc.variable.bfs_iterator();
            let names: Vec<_> = iter
                .map(|g| match g {
                    VariableIR::Scalar(s) => s.identity.name.as_deref().unwrap(),
                    VariableIR::Struct(s) => s.identity.name.as_deref().unwrap(),
                    VariableIR::Array(a) => a.identity.name.as_deref().unwrap(),
                    VariableIR::CEnum(e) => e.identity.name.as_deref().unwrap(),
                    VariableIR::RustEnum(e) => e.identity.name.as_deref().unwrap(),
                    VariableIR::Pointer(p) => p.identity.name.as_deref().unwrap(),
                    _ => {
                        unreachable!()
                    }
                })
                .collect();
            assert_eq!(tc.expected_order, names);
        }
    }

    // test helpers --------------------------------------------------------------------------------
    //
    fn make_scalar_var_ir(
        name: Option<&str>,
        type_name: &str,
        scalar: SupportedScalar,
    ) -> VariableIR {
        VariableIR::Scalar(ScalarVariable {
            identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
            type_id: None,
            type_name: Some(type_name.into()),
            value: Some(scalar),
            raw_address: None,
        })
    }

    fn make_str_var_ir(name: Option<&str>, val: &str) -> VariableIR {
        VariableIR::Specialized(SpecializedVariableIR::Str {
            string: Some(StrVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                value: val.to_string(),
            }),
            original: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                ..Default::default()
            },
        })
    }

    fn make_string_var_ir(name: Option<&str>, val: &str) -> VariableIR {
        VariableIR::Specialized(SpecializedVariableIR::String {
            string: Some(StringVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                value: val.to_string(),
            }),
            original: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                ..Default::default()
            },
        })
    }

    fn make_vec_var_ir(name: Option<&str>, items: Vec<VariableIR>) -> VecVariable {
        let items_len = items.len();
        VecVariable {
            structure: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                type_id: None,
                type_name: Some("vec".to_string()),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::default(),
                        type_id: None,
                        type_name: Some("[item]".to_string()),
                        items: Some(items),
                        raw_address: None,
                    }),
                    VariableIR::Scalar(ScalarVariable {
                        identity: VariableIdentity::no_namespace(Some("cap".to_string())),
                        type_id: None,
                        type_name: Some("usize".to_owned()),
                        value: Some(SupportedScalar::Usize(items_len)),
                        raw_address: None,
                    }),
                ],
                type_params: HashMap::default(),
                raw_address: None,
            },
        }
    }

    fn make_vector_var_ir(name: Option<&str>, items: Vec<VariableIR>) -> VariableIR {
        VariableIR::Specialized(SpecializedVariableIR::Vector {
            vec: Some(make_vec_var_ir(name, items)),
            original: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                ..Default::default()
            },
        })
    }

    fn make_vecdeque_var_ir(name: Option<&str>, items: Vec<VariableIR>) -> VariableIR {
        VariableIR::Specialized(SpecializedVariableIR::VecDeque {
            vec: Some(make_vec_var_ir(name, items)),
            original: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                ..Default::default()
            },
        })
    }

    fn make_hashset_var_ir(name: Option<&str>, items: Vec<VariableIR>) -> VariableIR {
        VariableIR::Specialized(SpecializedVariableIR::HashSet {
            set: Some(HashSetVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                type_name: Some("hashset".to_string()),
                items,
            }),
            original: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                ..Default::default()
            },
        })
    }

    fn make_btreeset_var_ir(name: Option<&str>, items: Vec<VariableIR>) -> VariableIR {
        VariableIR::Specialized(SpecializedVariableIR::BTreeSet {
            set: Some(HashSetVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                type_name: Some("btreeset".to_string()),
                items,
            }),
            original: StructVariable {
                identity: VariableIdentity::no_namespace(name.map(ToString::to_string)),
                ..Default::default()
            },
        })
    }
    //----------------------------------------------------------------------------------------------

    #[test]
    fn test_equal_with_literal() {
        struct TestCase {
            variable: VariableIR,
            eq_literal: Literal,
            neq_literals: Vec<Literal>,
        }

        let test_cases = [
            TestCase {
                variable: make_scalar_var_ir(None, "i8", SupportedScalar::I8(8)),
                eq_literal: Literal::Int(8),
                neq_literals: vec![Literal::Int(9)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "i32", SupportedScalar::I32(32)),
                eq_literal: Literal::Int(32),
                neq_literals: vec![Literal::Int(33)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "isize", SupportedScalar::Isize(-1234)),
                eq_literal: Literal::Int(-1234),
                neq_literals: vec![Literal::Int(-1233)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "u8", SupportedScalar::U8(8)),
                eq_literal: Literal::Int(8),
                neq_literals: vec![Literal::Int(9)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "u32", SupportedScalar::U32(32)),
                eq_literal: Literal::Int(32),
                neq_literals: vec![Literal::Int(33)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "usize", SupportedScalar::Usize(1234)),
                eq_literal: Literal::Int(1234),
                neq_literals: vec![Literal::Int(1235)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "f32", SupportedScalar::F32(1.1)),
                eq_literal: Literal::Float(1.1),
                neq_literals: vec![Literal::Float(1.2)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "f64", SupportedScalar::F64(-2.2)),
                eq_literal: Literal::Float(-2.2),
                neq_literals: vec![Literal::Float(2.2)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "bool", SupportedScalar::Bool(true)),
                eq_literal: Literal::Bool(true),
                neq_literals: vec![Literal::Bool(false)],
            },
            TestCase {
                variable: make_scalar_var_ir(None, "char", SupportedScalar::Char('b')),
                eq_literal: Literal::String("b".into()),
                neq_literals: vec![Literal::String("c".into())],
            },
            TestCase {
                variable: VariableIR::Pointer(PointerVariable {
                    identity: VariableIdentity::default(),
                    target_type: None,
                    type_id: None,
                    type_name: Some("ptr".into()),
                    value: Some(123usize as *const ()),
                    raw_address: None,
                    target_type_size: None,
                }),
                eq_literal: Literal::Address(123),
                neq_literals: vec![Literal::Address(124), Literal::Int(123)],
            },
            TestCase {
                variable: VariableIR::Pointer(PointerVariable {
                    identity: VariableIdentity::default(),
                    target_type: None,
                    type_id: None,
                    type_name: Some("MyPtr".into()),
                    value: Some(123usize as *const ()),
                    raw_address: None,
                    target_type_size: None,
                }),
                eq_literal: Literal::Address(123),
                neq_literals: vec![Literal::Address(124), Literal::Int(123)],
            },
            TestCase {
                variable: VariableIR::CEnum(CEnumVariable {
                    identity: VariableIdentity::default(),
                    type_id: None,
                    type_name: Some("MyEnum".into()),
                    value: Some("Variant1".into()),
                    raw_address: None,
                }),
                eq_literal: Literal::EnumVariant("Variant1".to_string(), None),
                neq_literals: vec![
                    Literal::EnumVariant("Variant2".to_string(), None),
                    Literal::String("Variant1".to_string()),
                ],
            },
            TestCase {
                variable: VariableIR::RustEnum(RustEnumVariable {
                    identity: VariableIdentity::default(),
                    type_id: None,
                    type_name: Some("MyEnum".into()),
                    value: Some(Box::new(VariableIR::Struct(StructVariable {
                        identity: VariableIdentity::no_namespace(Some("Variant1".to_string())),
                        type_id: None,
                        type_name: None,
                        members: vec![VariableIR::Scalar(ScalarVariable {
                            identity: VariableIdentity::no_namespace(Some("Variant1".to_string())),
                            type_id: None,
                            type_name: Some("int".into()),
                            value: Some(SupportedScalar::I64(100)),
                            raw_address: None,
                        })],
                        type_params: Default::default(),
                        raw_address: None,
                    }))),
                    raw_address: None,
                }),
                eq_literal: Literal::EnumVariant(
                    "Variant1".to_string(),
                    Some(Box::new(Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::Int(100)),
                    ])))),
                ),
                neq_literals: vec![
                    Literal::EnumVariant("Variant1".to_string(), Some(Box::new(Literal::Int(101)))),
                    Literal::EnumVariant("Variant2".to_string(), Some(Box::new(Literal::Int(100)))),
                    Literal::String("Variant1".to_string()),
                ],
            },
        ];

        for tc in test_cases {
            assert!(tc.variable.clone().match_literal(&tc.eq_literal));
            for neq_lit in tc.neq_literals {
                assert!(!tc.variable.clone().match_literal(&neq_lit));
            }
        }
    }

    #[test]
    fn test_equal_with_complex_literal() {
        struct TestCase {
            variable: VariableIR,
            eq_literals: Vec<Literal>,
            neq_literals: Vec<Literal>,
        }

        let test_cases = [
            TestCase {
                variable: make_str_var_ir(None, "str1"),
                eq_literals: vec![Literal::String("str1".to_string())],
                neq_literals: vec![Literal::String("str2".to_string()), Literal::Int(1)],
            },
            TestCase {
                variable: make_string_var_ir(None, "string1"),
                eq_literals: vec![Literal::String("string1".to_string())],
                neq_literals: vec![Literal::String("string2".to_string()), Literal::Int(1)],
            },
            TestCase {
                variable: VariableIR::Specialized(SpecializedVariableIR::Uuid {
                    value: Some([
                        0xd0, 0x60, 0x66, 0x29, 0x78, 0x6a, 0x44, 0xbe, 0x9d, 0x49, 0xb7, 0x02,
                        0x0f, 0x3e, 0xb0, 0x5a,
                    ]),
                    original: StructVariable::default(),
                }),
                eq_literals: vec![Literal::String(
                    "d0606629-786a-44be-9d49-b7020f3eb05a".to_string(),
                )],
                neq_literals: vec![Literal::String(
                    "d0606629-786a-44be-9d49-b7020f3eb05b".to_string(),
                )],
            },
            TestCase {
                variable: make_vector_var_ir(
                    None,
                    vec![
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('a')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('b')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                    ],
                ),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
            },
            TestCase {
                variable: make_vector_var_ir(
                    None,
                    vec![
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('a')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('b')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                    ],
                ),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
            },
            TestCase {
                variable: make_vecdeque_var_ir(
                    None,
                    vec![
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('a')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('b')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                    ],
                ),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
            },
            TestCase {
                variable: make_hashset_var_ir(
                    None,
                    vec![
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('a')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('b')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                    ],
                ),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
            },
            TestCase {
                variable: make_btreeset_var_ir(
                    None,
                    vec![
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('a')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('b')),
                        make_scalar_var_ir(None, "char", SupportedScalar::Char('c')),
                    ],
                ),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("b".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
            },
            TestCase {
                variable: VariableIR::Specialized(SpecializedVariableIR::Cell {
                    value: Some(Box::new(make_scalar_var_ir(
                        None,
                        "int",
                        SupportedScalar::I64(100),
                    ))),
                    original: StructVariable::default(),
                }),
                eq_literals: vec![Literal::Int(100)],
                neq_literals: vec![Literal::Int(101), Literal::Float(100.1)],
            },
            TestCase {
                variable: VariableIR::Array(ArrayVariable {
                    identity: Default::default(),
                    type_id: None,
                    type_name: Some("array_str".to_string()),
                    items: Some(vec![
                        make_str_var_ir(None, "ab"),
                        make_str_var_ir(None, "cd"),
                        make_str_var_ir(None, "ef"),
                    ]),
                    raw_address: None,
                }),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("ab".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("cd".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("ef".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("ab".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("cd".to_string())),
                        LiteralOrWildcard::Wildcard,
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("ab".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("cd".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("gj".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("ab".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("cd".to_string())),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("ab".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("cd".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("ef".to_string())),
                        LiteralOrWildcard::Literal(Literal::String("gj".to_string())),
                    ])),
                ],
            },
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: Default::default(),
                    type_id: None,
                    type_name: Some("MyStruct".to_string()),
                    members: vec![
                        make_str_var_ir(Some("str_field"), "str1"),
                        make_vector_var_ir(
                            Some("vec_field"),
                            vec![
                                make_scalar_var_ir(None, "", SupportedScalar::I8(1)),
                                make_scalar_var_ir(None, "", SupportedScalar::I8(2)),
                            ],
                        ),
                        make_scalar_var_ir(Some("bool_field"), "", SupportedScalar::Bool(true)),
                    ],
                    type_params: Default::default(),
                    raw_address: None,
                }),
                eq_literals: vec![
                    Literal::AssocArray(HashMap::from([
                        (
                            "str_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        ),
                        (
                            "vec_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Array(Box::new([
                                LiteralOrWildcard::Literal(Literal::Int(1)),
                                LiteralOrWildcard::Literal(Literal::Int(2)),
                            ]))),
                        ),
                        (
                            "bool_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Bool(true)),
                        ),
                    ])),
                    Literal::AssocArray(HashMap::from([
                        (
                            "str_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        ),
                        (
                            "vec_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Array(Box::new([
                                LiteralOrWildcard::Literal(Literal::Int(1)),
                                LiteralOrWildcard::Wildcard,
                            ]))),
                        ),
                        ("bool_field".to_string(), LiteralOrWildcard::Wildcard),
                    ])),
                ],
                neq_literals: vec![
                    Literal::AssocArray(HashMap::from([
                        (
                            "str_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::String("str2".to_string())),
                        ),
                        (
                            "vec_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Array(Box::new([
                                LiteralOrWildcard::Literal(Literal::Int(1)),
                                LiteralOrWildcard::Literal(Literal::Int(2)),
                            ]))),
                        ),
                        (
                            "bool_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Bool(true)),
                        ),
                    ])),
                    Literal::AssocArray(HashMap::from([
                        (
                            "str_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        ),
                        (
                            "vec_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Array(Box::new([
                                LiteralOrWildcard::Literal(Literal::Int(1)),
                            ]))),
                        ),
                        (
                            "bool_field".to_string(),
                            LiteralOrWildcard::Literal(Literal::Bool(true)),
                        ),
                    ])),
                ],
            },
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: Default::default(),
                    type_id: None,
                    type_name: Some("MyTuple".to_string()),
                    members: vec![
                        make_str_var_ir(None, "str1"),
                        make_vector_var_ir(
                            None,
                            vec![
                                make_scalar_var_ir(None, "", SupportedScalar::I8(1)),
                                make_scalar_var_ir(None, "", SupportedScalar::I8(2)),
                            ],
                        ),
                        make_scalar_var_ir(None, "", SupportedScalar::Bool(true)),
                    ],
                    type_params: Default::default(),
                    raw_address: None,
                }),
                eq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        LiteralOrWildcard::Literal(Literal::Array(Box::new([
                            LiteralOrWildcard::Literal(Literal::Int(1)),
                            LiteralOrWildcard::Literal(Literal::Int(2)),
                        ]))),
                        LiteralOrWildcard::Literal(Literal::Bool(true)),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        LiteralOrWildcard::Literal(Literal::Array(Box::new([
                            LiteralOrWildcard::Literal(Literal::Int(1)),
                            LiteralOrWildcard::Wildcard,
                        ]))),
                        LiteralOrWildcard::Wildcard,
                    ])),
                ],
                neq_literals: vec![
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        LiteralOrWildcard::Literal(Literal::Array(Box::new([
                            LiteralOrWildcard::Literal(Literal::Int(1)),
                            LiteralOrWildcard::Literal(Literal::Int(2)),
                        ]))),
                        LiteralOrWildcard::Literal(Literal::Bool(false)),
                    ])),
                    Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::String("str1".to_string())),
                        LiteralOrWildcard::Literal(Literal::Array(Box::new([
                            LiteralOrWildcard::Literal(Literal::Int(1)),
                        ]))),
                        LiteralOrWildcard::Literal(Literal::Bool(true)),
                    ])),
                ],
            },
        ];

        for tc in test_cases {
            for eq_lit in tc.eq_literals {
                assert!(tc.variable.clone().match_literal(&eq_lit));
            }
            for neq_lit in tc.neq_literals {
                assert!(!tc.variable.clone().match_literal(&neq_lit));
            }
        }
    }
}
