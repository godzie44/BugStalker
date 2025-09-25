use crate::debugger::TypeDeclaration;
use crate::debugger::debugee::dwarf::r#type::{CModifier, TypeId, TypeIdentity};
use crate::debugger::variable::ObjectBinaryRepr;
use crate::debugger::variable::dqe::{Literal, LiteralOrWildcard};
use crate::debugger::variable::render::RenderValue;
use crate::debugger::variable::value::bfs::BfsIterator;
use crate::debugger::variable::value::bfs::FieldOrIndex;
use crate::debugger::variable::value::parser::{ParseContext, ValueParser};
use crate::debugger::variable::value::specialization::{
    HashSetVariable, StrVariable, StringVariable,
};
use crate::{debugger, weak_error};
use bytes::Bytes;
use indexmap::IndexMap;
use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display, Formatter};
use std::string::FromUtf8Error;
use uuid::Uuid;

mod bfs;
pub(super) mod parser;
pub mod specialization;

pub use crate::debugger::variable::value::specialization::SpecializedValue;

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
    #[error("undefined size of type `{}`", .0.name_fmt())]
    UnknownSize(TypeIdentity),
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
pub struct ScalarValue {
    pub value: Option<SupportedScalar>,
    pub raw_address: Option<usize>,
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
}

impl ScalarValue {
    pub fn try_as_number(&self) -> Option<i64> {
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

/// Structure member representation.
#[derive(Clone, PartialEq, Debug)]
pub struct Member {
    pub field_name: Option<String>,
    pub value: Value,
}

/// Represents structures.
#[derive(Clone, Default, PartialEq)]
pub struct StructValue {
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
    /// Structure members.
    pub members: Vec<Member>,
    /// Map of type parameters of a structure type.
    pub type_params: IndexMap<String, Option<TypeId>>,
    pub raw_address: Option<usize>,
}

impl StructValue {
    pub fn field(self, field_name: &str) -> Option<Value> {
        self.members.into_iter().find_map(|member| {
            if member.field_name.as_deref() == Some(field_name) {
                Some(member.value)
            } else {
                None
            }
        })
    }

    pub fn into_member_n(self, n: usize) -> Option<Value> {
        let mut this = self;
        if this.members.len() > n {
            let m = this.members.swap_remove(n);
            return Some(m.value);
        };
        None
    }
}

/// Array item representation.
#[derive(Clone, PartialEq, Debug)]
pub struct ArrayItem {
    pub index: i64,
    pub value: Value,
}

/// Represents arrays.
#[derive(Clone, PartialEq)]
pub struct ArrayValue {
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
    /// Array items.
    pub items: Option<Vec<ArrayItem>>,
    pub raw_address: Option<usize>,
}

impl ArrayValue {
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
pub struct CEnumValue {
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
    /// String representation of selected variant.
    pub value: Option<String>,
    pub raw_address: Option<usize>,
}

/// Represents all enum's that more complex than c-style enums.
#[derive(Clone, PartialEq)]
pub struct RustEnumValue {
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
    /// Variable IR representation of selected variant.
    pub value: Option<Box<Member>>,
    pub raw_address: Option<usize>,
}

/// Raw pointers, references, Box.
#[derive(Clone, PartialEq)]
pub struct PointerValue {
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
    /// Raw pointer to underline value.
    pub value: Option<*const ()>,
    /// Underline type identity.
    pub target_type: Option<TypeId>,
    pub target_type_size: Option<u64>,
    pub raw_address: Option<usize>,
}

impl PointerValue {
    /// Dereference pointer and return variable IR that represents underline value.
    pub fn deref(&self, pcx: &ParseContext) -> Option<Value> {
        let target_type = self.target_type?;
        let deref_size = self
            .target_type_size
            .or_else(|| pcx.type_graph.type_size_in_bytes(pcx.evcx, target_type));

        let target_type_decl = pcx.type_graph.types.get(&target_type);
        if matches!(target_type_decl, Some(TypeDeclaration::Subroutine { .. })) {
            // this variable is a fn pointer - don't deref it
            return None;
        }

        self.value.and_then(|ptr| {
            let data = deref_size.and_then(|sz| {
                let raw_data = debugger::read_memory_by_pid(
                    pcx.evcx.ecx.pid_on_focus(),
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
            let parser = ValueParser::new();
            parser.parse_inner(pcx, data, target_type)
        })
    }

    /// Interpret a pointer as a pointer on first array element.
    /// Returns variable IR that represents an array.
    pub fn slice(&self, pcx: &ParseContext, left: Option<usize>, right: usize) -> Option<Value> {
        let target_type = self.target_type?;
        let deref_size = pcx.type_graph.type_size_in_bytes(pcx.evcx, target_type)? as usize;

        self.value.and_then(|ptr| {
            let left = left.unwrap_or_default();
            let base_addr = ptr as usize + deref_size * left;
            let raw_data = weak_error!(debugger::read_memory_by_pid(
                pcx.evcx.ecx.pid_on_focus(),
                base_addr,
                deref_size * (right - left)
            ))?;
            let raw_data = bytes::Bytes::from(raw_data);

            let parser = ValueParser::new();
            let items = raw_data
                .chunks(deref_size)
                .enumerate()
                .filter_map(|(i, chunk)| {
                    let data = ObjectBinaryRepr {
                        raw_data: raw_data.slice_ref(chunk),
                        address: Some(base_addr + (i * deref_size)),
                        size: deref_size,
                    };
                    Some(ArrayItem {
                        index: i as i64,
                        value: parser.parse_inner(pcx, Some(data), target_type)?,
                    })
                })
                .collect::<Vec<_>>();

            Some(Value::Array(ArrayValue {
                items: Some(items),
                type_id: None,
                type_ident: pcx.type_graph.identity(target_type).as_array_type(),
                raw_address: Some(base_addr),
            }))
        })
    }
}

/// Represents subroutine.
#[derive(Clone, PartialEq)]
pub struct SubroutineValue {
    pub type_id: Option<TypeId>,
    pub return_type_ident: Option<TypeIdentity>,
    pub address: Option<usize>,
}

/// Represent a variable with C modifiers (volatile, const, typedef, etc.)
#[derive(Clone, PartialEq)]
pub struct CModifiedValue {
    pub type_ident: TypeIdentity,
    pub type_id: Option<TypeId>,
    pub modifier: CModifier,
    pub value: Option<Box<Value>>,
    pub address: Option<usize>,
}

/// Program typed value representation.
#[derive(Clone, PartialEq)]
pub enum Value {
    Scalar(ScalarValue),
    Struct(StructValue),
    Array(ArrayValue),
    CEnum(CEnumValue),
    RustEnum(RustEnumValue),
    Pointer(PointerValue),
    Subroutine(SubroutineValue),
    Specialized {
        value: Option<SpecializedValue>,
        original: StructValue,
    },
    CModifiedVariable(CModifiedValue),
}

// SAFETY: this enum may contain a raw pointers on memory in a debugee process,
// it is safe to dereference it using public API of *Variable structures.
unsafe impl Send for Value {}

impl Debug for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.as_literal() {
            None => Ok(()),
            Some(lit) => f.write_fmt(format_args!("{lit}")),
        }
    }
}

impl Value {
    pub fn into_scalar(self) -> Option<ScalarValue> {
        match self {
            Value::Scalar(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_array(self) -> Option<ArrayValue> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&ArrayValue> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn into_raw_ptr(self) -> Option<PointerValue> {
        match self {
            Value::Pointer(p) => Some(p),
            _ => None,
        }
    }

    /// Return literal equals representation of a value.
    pub fn as_literal(&self) -> Option<Literal> {
        match self {
            Value::Scalar(scalar) => {
                if let Some(i) = scalar.try_as_number() {
                    return Some(Literal::Int(i));
                }

                match scalar.value.as_ref()? {
                    SupportedScalar::F32(f) => Some(Literal::Float(*f as f64)),
                    SupportedScalar::F64(f) => Some(Literal::Float(*f)),
                    SupportedScalar::Bool(b) => Some(Literal::Bool(*b)),
                    SupportedScalar::Char(c) => Some(Literal::String(c.to_string())),
                    SupportedScalar::Empty() => None,
                    _ => None,
                }
            }
            Value::Struct(s) => {
                let mut assoc_array = HashMap::new();
                for member in &s.members {
                    let field = member.field_name.as_ref()?.clone();
                    let literal = member.value.as_literal()?;
                    assoc_array.insert(field, LiteralOrWildcard::Literal(literal));
                }
                Some(Literal::AssocArray(assoc_array))
            }
            Value::Array(arr) => {
                let mut array = vec![];
                for item in arr.items.as_ref()? {
                    array.push(LiteralOrWildcard::Literal(item.value.as_literal()?))
                }
                Some(Literal::Array(array.into_boxed_slice()))
            }
            Value::CEnum(e) => Some(Literal::EnumVariant(e.value.as_ref()?.to_string(), None)),
            Value::RustEnum(e) => {
                let member = e.value.as_ref()?;
                Some(Literal::EnumVariant(
                    member.field_name.as_ref()?.clone(),
                    member.value.as_literal().map(Box::new),
                ))
            }
            Value::Pointer(ptr) => Some(Literal::Address(ptr.value? as usize)),
            Value::Subroutine(_) => None,
            Value::Specialized { value, .. } => match value.as_ref()? {
                SpecializedValue::Vector(vec) | SpecializedValue::VecDeque(vec) => {
                    let member = vec.structure.members.first();
                    let array = member?;
                    array.value.as_literal()
                }
                SpecializedValue::HashMap(map) | SpecializedValue::BTreeMap(map) => {
                    let mut assoc_array = HashMap::new();
                    for (key, val) in &map.kv_items {
                        let Some(Literal::String(key_str)) = key.as_literal() else {
                            return None;
                        };
                        let value = val.as_literal()?;
                        assoc_array.insert(key_str, LiteralOrWildcard::Literal(value));
                    }
                    Some(Literal::AssocArray(assoc_array))
                }
                SpecializedValue::HashSet(set) | SpecializedValue::BTreeSet(set) => {
                    let mut array = vec![];
                    for item in &set.items {
                        array.push(LiteralOrWildcard::Literal(item.as_literal()?))
                    }
                    Some(Literal::Array(array.into_boxed_slice()))
                }
                SpecializedValue::String(str) => Some(Literal::String(str.value.clone())),
                SpecializedValue::Str(str) => Some(Literal::String(str.value.clone())),
                SpecializedValue::Tls(tls) => tls.inner_value.as_ref()?.as_literal(),
                SpecializedValue::Cell(c) => c.as_literal(),
                SpecializedValue::RefCell(c) => c.as_literal(),
                SpecializedValue::Rc(ptr) => Some(Literal::Address(ptr.raw_address?)),
                SpecializedValue::Arc(ptr) => Some(Literal::Address(ptr.raw_address?)),
                SpecializedValue::Uuid(uuid) => {
                    let uuid = Uuid::from_bytes(*uuid);
                    Some(Literal::String(uuid.to_string()))
                }
                SpecializedValue::SystemTime(time) => {
                    let time = chrono::NaiveDateTime::from_timestamp_opt(time.0, time.1)?;
                    Some(Literal::String(
                        time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    ))
                }
                SpecializedValue::Instant(_) => None,
            },
            Value::CModifiedVariable(val) => Some(val.value.as_ref()?.as_literal()?),
        }
    }

    /// Return address in debugee memory for variable data.
    pub fn in_memory_location(&self) -> Option<usize> {
        match self {
            Value::Scalar(s) => s.raw_address,
            Value::Struct(s) => s.raw_address,
            Value::Array(a) => a.raw_address,
            Value::CEnum(ce) => ce.raw_address,
            Value::RustEnum(re) => re.raw_address,
            Value::Pointer(p) => p.raw_address,
            Value::Subroutine(s) => s.address,
            Value::Specialized {
                original: origin, ..
            } => origin.raw_address,
            Value::CModifiedVariable(cmv) => cmv.address,
        }
    }

    pub fn type_id(&self) -> Option<TypeId> {
        match self {
            Value::Scalar(s) => s.type_id,
            Value::Struct(s) => s.type_id,
            Value::Array(a) => a.type_id,
            Value::CEnum(ce) => ce.type_id,
            Value::RustEnum(re) => re.type_id,
            Value::Pointer(p) => p.type_id,
            Value::Subroutine(s) => s.type_id,
            Value::Specialized {
                original: origin, ..
            } => origin.type_id,
            Value::CModifiedVariable(cmv) => cmv.type_id,
        }
    }

    /// Visit variable children in BFS order.
    fn bfs_iterator(&self) -> BfsIterator<'_> {
        BfsIterator {
            queue: VecDeque::from([(FieldOrIndex::Root, self)]),
        }
    }

    /// Returns i64 value representation or error if cast fail.
    fn assume_field_as_scalar_number(&self, field_name: &'static str) -> Result<i64, AssumeError> {
        let val = self
            .bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                (field_or_idx == FieldOrIndex::Field(Some(field_name))).then_some(child)
            })
            .ok_or(AssumeError::FieldNotFound(field_name))?;
        if let Value::Scalar(s) = val {
            Ok(s.try_as_number()
                .ok_or(AssumeError::FieldNotANumber(field_name))?)
        } else {
            Err(AssumeError::FieldNotANumber(field_name))
        }
    }

    /// Returns value as a raw pointer or error if cast fails.
    fn assume_field_as_pointer(&self, field_name: &'static str) -> Result<*const (), AssumeError> {
        self.bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let Value::Pointer(pointer) = child
                    && field_or_idx == FieldOrIndex::Field(Some(field_name))
                {
                    return pointer.value;
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    /// Returns value as enum or error if cast fail.
    fn assume_field_as_rust_enum(
        &self,
        field_name: &'static str,
    ) -> Result<RustEnumValue, AssumeError> {
        self.bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let Value::RustEnum(r_enum) = child
                    && field_or_idx == FieldOrIndex::Field(Some(field_name))
                {
                    return Some(r_enum.clone());
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    /// Returns value as structure or error if cast fail.
    fn assume_field_as_struct(&self, field_name: &'static str) -> Result<StructValue, AssumeError> {
        self.bfs_iterator()
            .find_map(|(field_or_idx, child)| {
                if let Value::Struct(structure) = child
                    && field_or_idx == FieldOrIndex::Field(Some(field_name))
                {
                    return Some(structure.clone());
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("structure"))
    }

    /// Return an underlying structure for specialized values (vectors, strings, etc.).
    pub fn canonic(self) -> Self {
        match self {
            Value::Specialized {
                original: origin, ..
            } => Value::Struct(origin),
            _ => self,
        }
    }

    /// Try to dereference variable and returns underline variable IR.
    /// Return `None` if dereference not allowed.
    pub fn deref(self, pcx: &ParseContext) -> Option<Self> {
        match self {
            Value::Pointer(ptr) => ptr.deref(pcx),
            Value::RustEnum(r_enum) => r_enum.value.and_then(|v| v.value.deref(pcx)),
            Value::Specialized {
                value: Some(SpecializedValue::Rc(ptr)),
                ..
            }
            | Value::Specialized {
                value: Some(SpecializedValue::Arc(ptr)),
                ..
            } => ptr.deref(pcx),
            Value::Specialized {
                value: Some(SpecializedValue::Tls(tls_var)),
                ..
            } => tls_var.inner_value.and_then(|inner| inner.deref(pcx)),
            Value::Specialized {
                value: Some(SpecializedValue::Cell(cell)),
                ..
            }
            | Value::Specialized {
                value: Some(SpecializedValue::RefCell(cell)),
                ..
            } => cell.deref(pcx),
            _ => None,
        }
    }

    /// Return address (as pointer variable) of raw data in debugee memory.
    pub fn address(self, pcx: &ParseContext) -> Option<Self> {
        let addr = self.in_memory_location()?;
        Some(Value::Pointer(PointerValue {
            type_ident: self.r#type().as_address_type(),
            value: Some(addr as *const ()),
            target_type: self.type_id(),
            target_type_size: self
                .type_id()
                .and_then(|t| pcx.type_graph.type_size_in_bytes(pcx.evcx, t)),
            raw_address: None,
            type_id: None,
        }))
    }

    /// Return variable field, `None` if field is not allowed for a variable type.
    /// Supported: structures, rust-style enums, hashmaps, btree-maps.
    pub fn field(self, field_name: &str) -> Option<Self> {
        match self {
            Value::Struct(structure) => structure.field(field_name),
            Value::RustEnum(r_enum) => r_enum.value.and_then(|v| v.value.field(field_name)),
            Value::Specialized {
                value: specialized, ..
            } => match specialized {
                Some(SpecializedValue::HashMap(map)) | Some(SpecializedValue::BTreeMap(map)) => {
                    map.kv_items.into_iter().find_map(|(key, value)| match key {
                        Value::Specialized {
                            value: specialized, ..
                        } => match specialized {
                            Some(SpecializedValue::String(string_key)) => {
                                (string_key.value == field_name).then_some(value)
                            }
                            Some(SpecializedValue::Str(string_key)) => {
                                (string_key.value == field_name).then_some(value)
                            }
                            _ => None,
                        },
                        _ => None,
                    })
                }
                Some(SpecializedValue::Tls(tls_var)) => tls_var
                    .inner_value
                    .and_then(|inner| inner.field(field_name)),
                Some(SpecializedValue::Cell(cell)) | Some(SpecializedValue::RefCell(cell)) => {
                    cell.field(field_name)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Return variable element by its index, `None` if indexing is not allowed for a variable type.
    /// Supported: array, rust-style enums, vector, hashmap, hashset, btreemap, btreeset.
    pub fn index(self, idx: &Literal) -> Option<Self> {
        match self {
            Value::Array(array) => array.items.and_then(|mut items| {
                if let Literal::Int(idx) = idx {
                    let idx = *idx as usize;
                    if idx < items.len() {
                        return Some(items.swap_remove(idx).value);
                    }
                }
                None
            }),
            Value::RustEnum(r_enum) => r_enum.value.and_then(|v| v.value.index(idx)),
            Value::Specialized {
                value: Some(spec_val),
                ..
            } => match spec_val {
                SpecializedValue::Vector(mut vec) | SpecializedValue::VecDeque(mut vec) => {
                    let inner_array = vec.structure.members.swap_remove(0).value;
                    inner_array.index(idx)
                }
                SpecializedValue::Tls(tls_var) => {
                    tls_var.inner_value.and_then(|inner| inner.index(idx))
                }
                SpecializedValue::Cell(cell) | SpecializedValue::RefCell(cell) => cell.index(idx),
                SpecializedValue::BTreeMap(map) | SpecializedValue::HashMap(map) => {
                    for (k, v) in map.kv_items {
                        if k.match_literal(idx) {
                            return Some(v);
                        }
                    }

                    None
                }
                SpecializedValue::BTreeSet(set) | SpecializedValue::HashSet(set) => {
                    let found = set.items.into_iter().any(|it| it.match_literal(idx));

                    Some(Value::Scalar(ScalarValue {
                        type_id: None,
                        type_ident: TypeIdentity::no_namespace("bool"),
                        value: Some(SupportedScalar::Bool(found)),
                        raw_address: None,
                    }))
                }
                _ => None,
            },
            _ => None,
        }
    }

    pub fn slice(
        self,
        pcx: &ParseContext,
        left: Option<usize>,
        right: Option<usize>,
    ) -> Option<Self> {
        match self {
            Value::Array(mut array) => {
                array.slice(left, right);
                Some(Value::Array(array))
            }
            Value::Pointer(ptr) => {
                // for pointer the right bound must always be specified
                let right = right?;
                ptr.slice(pcx, left, right)
            }
            Value::Specialized {
                value: Some(spec_val),
                original,
            } => match spec_val {
                SpecializedValue::Rc(ptr) | SpecializedValue::Arc(ptr) => {
                    // for pointer the right bound must always be specified
                    let right = right?;
                    ptr.slice(pcx, left, right)
                }
                SpecializedValue::Vector(mut vec) => {
                    vec.slice(left, right);
                    Some(Value::Specialized {
                        value: Some(SpecializedValue::Vector(vec)),
                        original,
                    })
                }
                SpecializedValue::VecDeque(mut vec) => {
                    vec.slice(left, right);
                    Some(Value::Specialized {
                        value: Some(SpecializedValue::VecDeque(vec)),
                        original,
                    })
                }
                SpecializedValue::Tls(mut tls_var) => {
                    let inner = tls_var.inner_value.take()?;
                    inner.slice(pcx, left, right)
                }
                SpecializedValue::Cell(cell) | SpecializedValue::RefCell(cell) => {
                    cell.slice(pcx, left, right)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Match variable with a literal object.
    /// Return true if variable matched to literal.
    fn match_literal(self, literal: &Literal) -> bool {
        match self {
            Value::Scalar(ScalarValue {
                value: Some(scalar),
                ..
            }) => scalar.equal_with_literal(literal),
            Value::Pointer(PointerValue {
                value: Some(ptr), ..
            }) => literal.equal_with_address(ptr as usize),
            Value::Array(ArrayValue {
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
                            if !item.value.match_literal(lit) {
                                return false;
                            }
                        }
                        LiteralOrWildcard::Wildcard => continue,
                    }
                }
                true
            }
            Value::Struct(StructValue { members, .. }) => {
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
                                    if !member.value.match_literal(lit) {
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
                            let Some(member_name) = member.field_name else {
                                return false;
                            };

                            let Some(field_literal) = struct_literal.get(&member_name) else {
                                return false;
                            };

                            match field_literal {
                                LiteralOrWildcard::Literal(lit) => {
                                    if !member.value.match_literal(lit) {
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
            Value::Specialized {
                value: Some(spec), ..
            } => match spec {
                SpecializedValue::String(StringVariable { value, .. }) => {
                    literal.equal_with_string(&value)
                }
                SpecializedValue::Str(StrVariable { value, .. }) => {
                    literal.equal_with_string(&value)
                }
                SpecializedValue::Uuid(bytes) => {
                    let uuid = Uuid::from_bytes(bytes);
                    literal.equal_with_string(&uuid.to_string())
                }
                SpecializedValue::Cell(cell) | SpecializedValue::RefCell(cell) => {
                    cell.match_literal(literal)
                }
                SpecializedValue::Rc(PointerValue {
                    value: Some(ptr), ..
                })
                | SpecializedValue::Arc(PointerValue {
                    value: Some(ptr), ..
                }) => literal.equal_with_address(ptr as usize),
                SpecializedValue::Vector(mut v) | SpecializedValue::VecDeque(mut v) => {
                    let inner_array = v.structure.members.swap_remove(0).value;
                    debug_assert!(matches!(inner_array, Value::Array(_)));
                    inner_array.match_literal(literal)
                }
                SpecializedValue::HashSet(HashSetVariable { items, .. })
                | SpecializedValue::BTreeSet(HashSetVariable { items, .. }) => {
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
                SpecializedValue::Tls(inner) => inner
                    .inner_value
                    .map(|v| v.match_literal(literal))
                    .unwrap_or_default(),
                SpecializedValue::SystemTime(time) => {
                    let Some(time) = chrono::NaiveDateTime::from_timestamp_opt(time.0, time.1)
                    else {
                        return false;
                    };

                    literal.equal_with_string(&time.format("%Y-%m-%d %H:%M:%S").to_string())
                }
                _ => false,
            },
            Value::CEnum(CEnumValue {
                value: Some(ref value),
                ..
            }) => {
                let Literal::EnumVariant(variant, None) = literal else {
                    return false;
                };
                value == variant
            }
            Value::RustEnum(RustEnumValue {
                value: Some(value), ..
            }) => {
                let Literal::EnumVariant(variant, variant_value) = literal else {
                    return false;
                };

                if value.field_name.as_ref() != Some(variant) {
                    return false;
                }

                match variant_value {
                    None => true,
                    Some(lit) => value.value.match_literal(lit),
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::debugger::variable::value::specialization::VecValue;

    // test helpers --------------------------------------------------------------------------------
    //
    fn make_scalar_val(type_name: &str, scalar: SupportedScalar) -> Value {
        Value::Scalar(ScalarValue {
            type_id: None,
            type_ident: TypeIdentity::no_namespace(type_name),
            value: Some(scalar),
            raw_address: None,
        })
    }

    fn make_str_val(val: &str) -> Value {
        Value::Specialized {
            value: Some(SpecializedValue::Str(StrVariable {
                value: val.to_string(),
            })),
            original: StructValue {
                ..Default::default()
            },
        }
    }

    fn make_string_val(val: &str) -> Value {
        Value::Specialized {
            value: Some(SpecializedValue::String(StringVariable {
                value: val.to_string(),
            })),
            original: StructValue {
                ..Default::default()
            },
        }
    }

    fn make_vec_val(items: Vec<ArrayItem>) -> VecValue {
        let items_len = items.len();
        VecValue {
            structure: StructValue {
                type_id: None,
                type_ident: TypeIdentity::no_namespace("vec"),
                members: vec![
                    Member {
                        field_name: None,
                        value: Value::Array(ArrayValue {
                            type_id: None,
                            type_ident: TypeIdentity::no_namespace("[item]"),
                            items: Some(items),
                            raw_address: None,
                        }),
                    },
                    Member {
                        field_name: Some("cap".to_string()),
                        value: Value::Scalar(ScalarValue {
                            type_id: None,
                            type_ident: TypeIdentity::no_namespace("usize"),
                            value: Some(SupportedScalar::Usize(items_len)),
                            raw_address: None,
                        }),
                    },
                ],
                type_params: IndexMap::default(),
                raw_address: None,
            },
        }
    }

    fn make_vector_val(items: Vec<ArrayItem>) -> Value {
        Value::Specialized {
            value: Some(SpecializedValue::Vector(make_vec_val(items))),
            original: StructValue {
                ..Default::default()
            },
        }
    }

    fn make_vecdeque_val(items: Vec<ArrayItem>) -> Value {
        Value::Specialized {
            value: Some(SpecializedValue::VecDeque(make_vec_val(items))),
            original: StructValue {
                ..Default::default()
            },
        }
    }

    fn make_hashset_val(items: Vec<Value>) -> Value {
        Value::Specialized {
            value: Some(SpecializedValue::HashSet(HashSetVariable {
                type_ident: TypeIdentity::no_namespace("hashset"),
                items,
            })),
            original: StructValue {
                ..Default::default()
            },
        }
    }

    fn make_btreeset_var_val(items: Vec<Value>) -> Value {
        Value::Specialized {
            value: Some(SpecializedValue::BTreeSet(HashSetVariable {
                type_ident: TypeIdentity::no_namespace("btreeset"),
                items,
            })),
            original: StructValue {
                ..Default::default()
            },
        }
    }
    //----------------------------------------------------------------------------------------------

    #[test]
    fn test_equal_with_literal() {
        struct TestCase {
            variable: Value,
            eq_literal: Literal,
            neq_literals: Vec<Literal>,
        }

        let test_cases = [
            TestCase {
                variable: make_scalar_val("i8", SupportedScalar::I8(8)),
                eq_literal: Literal::Int(8),
                neq_literals: vec![Literal::Int(9)],
            },
            TestCase {
                variable: make_scalar_val("i32", SupportedScalar::I32(32)),
                eq_literal: Literal::Int(32),
                neq_literals: vec![Literal::Int(33)],
            },
            TestCase {
                variable: make_scalar_val("isize", SupportedScalar::Isize(-1234)),
                eq_literal: Literal::Int(-1234),
                neq_literals: vec![Literal::Int(-1233)],
            },
            TestCase {
                variable: make_scalar_val("u8", SupportedScalar::U8(8)),
                eq_literal: Literal::Int(8),
                neq_literals: vec![Literal::Int(9)],
            },
            TestCase {
                variable: make_scalar_val("u32", SupportedScalar::U32(32)),
                eq_literal: Literal::Int(32),
                neq_literals: vec![Literal::Int(33)],
            },
            TestCase {
                variable: make_scalar_val("usize", SupportedScalar::Usize(1234)),
                eq_literal: Literal::Int(1234),
                neq_literals: vec![Literal::Int(1235)],
            },
            TestCase {
                variable: make_scalar_val("f32", SupportedScalar::F32(1.1)),
                eq_literal: Literal::Float(1.1),
                neq_literals: vec![Literal::Float(1.2)],
            },
            TestCase {
                variable: make_scalar_val("f64", SupportedScalar::F64(-2.2)),
                eq_literal: Literal::Float(-2.2),
                neq_literals: vec![Literal::Float(2.2)],
            },
            TestCase {
                variable: make_scalar_val("bool", SupportedScalar::Bool(true)),
                eq_literal: Literal::Bool(true),
                neq_literals: vec![Literal::Bool(false)],
            },
            TestCase {
                variable: make_scalar_val("char", SupportedScalar::Char('b')),
                eq_literal: Literal::String("b".into()),
                neq_literals: vec![Literal::String("c".into())],
            },
            TestCase {
                variable: Value::Pointer(PointerValue {
                    target_type: None,
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("ptr"),
                    value: Some(123usize as *const ()),
                    raw_address: None,
                    target_type_size: None,
                }),
                eq_literal: Literal::Address(123),
                neq_literals: vec![Literal::Address(124), Literal::Int(123)],
            },
            TestCase {
                variable: Value::Pointer(PointerValue {
                    target_type: None,
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("MyPtr"),
                    value: Some(123usize as *const ()),
                    raw_address: None,
                    target_type_size: None,
                }),
                eq_literal: Literal::Address(123),
                neq_literals: vec![Literal::Address(124), Literal::Int(123)],
            },
            TestCase {
                variable: Value::CEnum(CEnumValue {
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("MyEnum"),
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
                variable: Value::RustEnum(RustEnumValue {
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("MyEnum"),
                    value: Some(Box::new(Member {
                        field_name: Some("Variant1".to_string()),
                        value: Value::Struct(StructValue {
                            type_id: None,
                            type_ident: TypeIdentity::unknown(),
                            members: vec![Member {
                                field_name: Some("Variant1".to_string()),
                                value: Value::Scalar(ScalarValue {
                                    type_id: None,
                                    type_ident: TypeIdentity::no_namespace("int"),
                                    value: Some(SupportedScalar::I64(100)),
                                    raw_address: None,
                                }),
                            }],
                            type_params: Default::default(),
                            raw_address: None,
                        }),
                    })),
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
            variable: Value,
            eq_literals: Vec<Literal>,
            neq_literals: Vec<Literal>,
        }

        let test_cases = [
            TestCase {
                variable: make_str_val("str1"),
                eq_literals: vec![Literal::String("str1".to_string())],
                neq_literals: vec![Literal::String("str2".to_string()), Literal::Int(1)],
            },
            TestCase {
                variable: make_string_val("string1"),
                eq_literals: vec![Literal::String("string1".to_string())],
                neq_literals: vec![Literal::String("string2".to_string()), Literal::Int(1)],
            },
            TestCase {
                variable: Value::Specialized {
                    value: Some(SpecializedValue::Uuid([
                        0xd0, 0x60, 0x66, 0x29, 0x78, 0x6a, 0x44, 0xbe, 0x9d, 0x49, 0xb7, 0x02,
                        0x0f, 0x3e, 0xb0, 0x5a,
                    ])),
                    original: StructValue::default(),
                },
                eq_literals: vec![Literal::String(
                    "d0606629-786a-44be-9d49-b7020f3eb05a".to_string(),
                )],
                neq_literals: vec![Literal::String(
                    "d0606629-786a-44be-9d49-b7020f3eb05b".to_string(),
                )],
            },
            TestCase {
                variable: make_vector_val(vec![
                    ArrayItem {
                        index: 0,
                        value: make_scalar_val("char", SupportedScalar::Char('a')),
                    },
                    ArrayItem {
                        index: 1,
                        value: make_scalar_val("char", SupportedScalar::Char('b')),
                    },
                    ArrayItem {
                        index: 2,
                        value: make_scalar_val("char", SupportedScalar::Char('c')),
                    },
                    ArrayItem {
                        index: 3,
                        value: make_scalar_val("char", SupportedScalar::Char('c')),
                    },
                ]),
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
                variable: make_vecdeque_val(vec![
                    ArrayItem {
                        index: 0,
                        value: make_scalar_val("char", SupportedScalar::Char('a')),
                    },
                    ArrayItem {
                        index: 1,
                        value: make_scalar_val("char", SupportedScalar::Char('b')),
                    },
                    ArrayItem {
                        index: 2,
                        value: make_scalar_val("char", SupportedScalar::Char('c')),
                    },
                    ArrayItem {
                        index: 3,
                        value: make_scalar_val("char", SupportedScalar::Char('c')),
                    },
                ]),
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
                variable: make_hashset_val(vec![
                    make_scalar_val("char", SupportedScalar::Char('a')),
                    make_scalar_val("char", SupportedScalar::Char('b')),
                    make_scalar_val("char", SupportedScalar::Char('c')),
                ]),
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
                variable: make_btreeset_var_val(vec![
                    make_scalar_val("char", SupportedScalar::Char('a')),
                    make_scalar_val("char", SupportedScalar::Char('b')),
                    make_scalar_val("char", SupportedScalar::Char('c')),
                ]),
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
                variable: Value::Specialized {
                    value: Some(SpecializedValue::Cell(Box::new(make_scalar_val(
                        "int",
                        SupportedScalar::I64(100),
                    )))),
                    original: StructValue::default(),
                },
                eq_literals: vec![Literal::Int(100)],
                neq_literals: vec![Literal::Int(101), Literal::Float(100.1)],
            },
            TestCase {
                variable: Value::Array(ArrayValue {
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("array_str"),
                    items: Some(vec![
                        ArrayItem {
                            index: 0,
                            value: make_str_val("ab"),
                        },
                        ArrayItem {
                            index: 1,
                            value: make_str_val("cd"),
                        },
                        ArrayItem {
                            index: 2,
                            value: make_str_val("ef"),
                        },
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
                variable: Value::Struct(StructValue {
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("MyStruct"),
                    members: vec![
                        Member {
                            field_name: Some("str_field".to_string()),
                            value: make_str_val("str1"),
                        },
                        Member {
                            field_name: Some("vec_field".to_string()),
                            value: make_vector_val(vec![
                                ArrayItem {
                                    index: 0,
                                    value: make_scalar_val("", SupportedScalar::I8(1)),
                                },
                                ArrayItem {
                                    index: 1,
                                    value: make_scalar_val("", SupportedScalar::I8(2)),
                                },
                            ]),
                        },
                        Member {
                            field_name: Some("bool_field".to_string()),
                            value: make_scalar_val("", SupportedScalar::Bool(true)),
                        },
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
                variable: Value::Struct(StructValue {
                    type_id: None,
                    type_ident: TypeIdentity::no_namespace("MyTuple"),
                    members: vec![
                        Member {
                            field_name: None,
                            value: make_str_val("str1"),
                        },
                        Member {
                            field_name: None,
                            value: make_vector_val(vec![
                                ArrayItem {
                                    index: 0,
                                    value: make_scalar_val("", SupportedScalar::I8(1)),
                                },
                                ArrayItem {
                                    index: 1,
                                    value: make_scalar_val("", SupportedScalar::I8(2)),
                                },
                            ]),
                        },
                        Member {
                            field_name: None,
                            value: make_scalar_val("", SupportedScalar::Bool(true)),
                        },
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
