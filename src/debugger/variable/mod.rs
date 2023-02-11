use crate::debugger::debugee::dwarf::r#type::{
    ArrayType, EvaluationContext, ScalarType, StructureMember,
};
use crate::debugger::debugee::dwarf::NamespaceHierarchy;
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::specialized::{
    StrVariable, StringVariable, TlsVariable, VecVariable,
};
use crate::debugger::TypeDeclaration;
use crate::{debugger, weak_error};
use anyhow::Context;
use bytes::Bytes;
use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display, Formatter};
use std::mem;

pub mod render;
pub mod specialized;

pub use specialized::SpecializedVariableIR;

#[derive(Clone)]
pub struct VariableIdentity {
    namespace: NamespaceHierarchy,
    pub name: Option<String>,
}

impl VariableIdentity {
    pub fn new(namespace: NamespaceHierarchy, name: Option<String>) -> Self {
        Self { namespace, name }
    }

    fn no_namespace(name: Option<String>) -> Self {
        Self {
            namespace: NamespaceHierarchy::default(),
            name,
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

#[derive(Clone)]
pub struct ScalarVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<SupportedScalar>,
}

impl ScalarVariable {
    fn new(identity: VariableIdentity, value: Option<Bytes>, r#type: &ScalarType) -> Self {
        fn render_scalar<S: Copy + Display>(data: Option<Bytes>) -> Option<S> {
            data.as_ref().map(|v| *scalar_from_bytes::<S>(v))
        }

        let value_view = match r#type.name.as_deref() {
            Some("i8") => render_scalar::<i8>(value).map(SupportedScalar::I8),
            Some("i16") => render_scalar::<i16>(value).map(SupportedScalar::I16),
            Some("i32") => render_scalar::<i32>(value).map(SupportedScalar::I32),
            Some("i64") => render_scalar::<i64>(value).map(SupportedScalar::I64),
            Some("i128") => render_scalar::<i128>(value).map(SupportedScalar::I128),
            Some("isize") => render_scalar::<isize>(value).map(SupportedScalar::Isize),
            Some("u8") => render_scalar::<u8>(value).map(SupportedScalar::U8),
            Some("u16") => render_scalar::<u16>(value).map(SupportedScalar::U16),
            Some("u32") => render_scalar::<u32>(value).map(SupportedScalar::U32),
            Some("u64") => render_scalar::<u64>(value).map(SupportedScalar::U64),
            Some("u128") => render_scalar::<u128>(value).map(SupportedScalar::U128),
            Some("usize") => render_scalar::<usize>(value).map(SupportedScalar::Usize),
            Some("f32") => render_scalar::<f32>(value).map(SupportedScalar::F32),
            Some("f64") => render_scalar::<f64>(value).map(SupportedScalar::F64),
            Some("bool") => render_scalar::<bool>(value).map(SupportedScalar::Bool),
            Some("char") => render_scalar::<char>(value).map(SupportedScalar::Char),
            Some("()") => Some(SupportedScalar::Empty()),
            _ => None,
        };

        ScalarVariable {
            identity,
            type_name: r#type.name.clone(),
            value: value_view,
        }
    }

    fn try_as_number(&self) -> Option<i64> {
        match self.value {
            Some(SupportedScalar::I8(num)) => Some(num as i64),
            Some(SupportedScalar::I16(num)) => Some(num as i64),
            Some(SupportedScalar::I32(num)) => Some(num as i64),
            Some(SupportedScalar::I64(num)) => Some(num as i64),
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

#[derive(Clone)]
pub struct StructVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub members: Vec<VariableIR>,
}

impl StructVariable {
    fn new(
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        members: &[StructureMember],
    ) -> Self {
        let children = members
            .iter()
            .map(|member| VariableIR::from_member(eval_ctx, member, value.as_ref()))
            .collect();

        StructVariable {
            identity,
            type_name,
            members: children,
        }
    }
}

#[derive(Clone)]
pub struct ArrayVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub items: Option<Vec<VariableIR>>,
}

impl ArrayVariable {
    fn new(
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        array_decl: &ArrayType,
    ) -> Self {
        let items = array_decl.bounds(eval_ctx).and_then(|bounds| {
            let len = bounds.1 - bounds.0;
            let el_size = array_decl.size_in_bytes(eval_ctx)? / len as u64;
            let bytes = value.as_ref()?;
            Some(
                bytes
                    .chunks(el_size as usize)
                    .enumerate()
                    .map(|(i, chunk)| {
                        VariableIR::new(
                            eval_ctx,
                            VariableIdentity::no_namespace(Some(format!(
                                "{}",
                                bounds.0 + i as i64
                            ))),
                            Some(bytes.slice_ref(chunk)),
                            array_decl.element_type.as_ref().map(|t| t.as_ref()),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        });

        ArrayVariable {
            identity,
            items,
            type_name,
        }
    }
}

#[derive(Clone)]
pub struct CEnumVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<String>,
}

impl CEnumVariable {
    fn new(
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        discr_type: Option<&TypeDeclaration>,
        enumerators: &HashMap<i64, String>,
    ) -> Self {
        let discr = VariableIR::new(
            eval_ctx,
            VariableIdentity::no_namespace(None),
            value,
            discr_type,
        );
        let value = if let VariableIR::Scalar(scalar) = discr {
            scalar.try_as_number()
        } else {
            None
        };

        CEnumVariable {
            identity,
            type_name,
            value: value.and_then(|val| enumerators.get(&(val as i64)).cloned()),
        }
    }
}

#[derive(Clone)]
pub struct RustEnumVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<Box<VariableIR>>,
}

impl RustEnumVariable {
    fn new(
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        discr_member: Option<&StructureMember>,
        enumerators: &HashMap<Option<i64>, StructureMember>,
    ) -> Self {
        let discr_value = discr_member.and_then(|member| {
            let discr = VariableIR::from_member(eval_ctx, member, value.as_ref());
            if let VariableIR::Scalar(scalar) = discr {
                return scalar.try_as_number();
            }
            None
        });

        let enumerator =
            discr_value.and_then(|v| enumerators.get(&Some(v)).or_else(|| enumerators.get(&None)));

        let enumerator = enumerator
            .map(|member| Box::new(VariableIR::from_member(eval_ctx, member, value.as_ref())));

        RustEnumVariable {
            identity,
            type_name,
            value: enumerator,
        }
    }
}

#[derive(Clone)]
pub struct PointerVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<*const ()>,
    pub deref: Option<Box<VariableIR>>,
}

impl PointerVariable {
    fn new(
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        target_type: Option<&TypeDeclaration>,
    ) -> Self {
        let mb_ptr = value.as_ref().map(scalar_from_bytes::<*const ()>).copied();
        let deref_size = target_type
            .as_ref()
            .and_then(|&t| t.size_in_bytes(eval_ctx));

        let deref_var = mb_ptr.map(|ptr| {
            let val = deref_size.and_then(|sz| {
                debugger::read_memory_by_pid(eval_ctx.pid, ptr as usize, sz as usize).ok()
            });

            Box::new(VariableIR::new(
                eval_ctx,
                VariableIdentity::no_namespace(Some(String::from("*"))),
                val.map(Bytes::from),
                target_type,
            ))
        });

        PointerVariable {
            identity,
            type_name,
            value: mb_ptr,
            deref: deref_var,
        }
    }
}

#[derive(Clone)]
pub enum VariableIR {
    Scalar(ScalarVariable),
    Struct(StructVariable),
    Array(ArrayVariable),
    CEnum(CEnumVariable),
    RustEnum(RustEnumVariable),
    Pointer(PointerVariable),
    Specialized(SpecializedVariableIR),
}

impl Debug for VariableIR {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl VariableIR {
    pub fn new(
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        r#type: Option<&TypeDeclaration>,
    ) -> Self {
        let type_name = r#type.as_ref().and_then(|t| t.name());

        match r#type {
            Some(type_decl) => match type_decl {
                TypeDeclaration::Scalar(scalar_type) => {
                    VariableIR::Scalar(ScalarVariable::new(identity, value, scalar_type))
                }
                TypeDeclaration::Structure {
                    namespaces: type_ns_h,
                    members,
                    type_params,
                    name: struct_name,
                    ..
                } => {
                    let struct_var =
                        StructVariable::new(eval_ctx, identity, value, type_name, members);

                    // Reinterpret structure if underline data type is:
                    // - Vector
                    // - String
                    // - &str
                    // - tls variable
                    if struct_name.as_deref() == Some("&str") {
                        return VariableIR::Specialized(SpecializedVariableIR::Str {
                            string: weak_error!(StrVariable::from_struct_ir(
                                eval_ctx,
                                VariableIR::Struct(struct_var.clone()),
                            )
                            .context("&str interpretation")),
                            original: struct_var,
                        });
                    };

                    if struct_name.as_deref() == Some("String") {
                        return VariableIR::Specialized(SpecializedVariableIR::String {
                            string: weak_error!(StringVariable::from_struct_ir(
                                eval_ctx,
                                VariableIR::Struct(struct_var.clone()),
                            )
                            .context("string interpretation")),
                            original: struct_var,
                        });
                    };

                    if struct_name.as_ref().map(|name| name.starts_with("Vec")) == Some(true) {
                        return VariableIR::Specialized(SpecializedVariableIR::Vector {
                            vec: weak_error!(VecVariable::from_struct_ir(
                                eval_ctx,
                                VariableIR::Struct(struct_var.clone()),
                                type_params,
                            )
                            .context("vec interpretation")),
                            original: struct_var,
                        });
                    };

                    if type_ns_h.contains(&["std", "thread", "local", "fast"]) {
                        return VariableIR::Specialized(SpecializedVariableIR::Tls {
                            tls_var: weak_error!(TlsVariable::from_struct_ir(
                                VariableIR::Struct(struct_var.clone()),
                                type_params,
                            )
                            .context("tls interpretation")),
                            original: struct_var,
                        });
                    }

                    VariableIR::Struct(struct_var)
                }
                TypeDeclaration::Array(decl) => VariableIR::Array(ArrayVariable::new(
                    eval_ctx, identity, value, type_name, decl,
                )),
                TypeDeclaration::CStyleEnum {
                    discr_type,
                    enumerators,
                    ..
                } => VariableIR::CEnum(CEnumVariable::new(
                    eval_ctx,
                    identity,
                    value,
                    type_name,
                    discr_type.as_ref().map(|t| t.as_ref()),
                    enumerators,
                )),
                TypeDeclaration::RustEnum {
                    discr_type,
                    enumerators,
                    ..
                } => VariableIR::RustEnum(RustEnumVariable::new(
                    eval_ctx,
                    identity,
                    value,
                    type_name,
                    discr_type.as_ref().map(|t| t.as_ref()),
                    enumerators,
                )),
                TypeDeclaration::Pointer { target_type, .. } => {
                    VariableIR::Pointer(PointerVariable::new(
                        eval_ctx,
                        identity,
                        value,
                        type_name,
                        target_type.as_ref().map(|t| t.as_ref()),
                    ))
                }
                TypeDeclaration::Union { members, .. } => {
                    let struct_var =
                        StructVariable::new(eval_ctx, identity, value, type_name, members);
                    VariableIR::Struct(struct_var)
                }
            },
            _ => {
                todo!()
            }
        }
    }

    fn from_member(
        eval_ctx: &EvaluationContext,
        member: &StructureMember,
        parent_value: Option<&Bytes>,
    ) -> Self {
        let member_val = parent_value.and_then(|val| member.value(eval_ctx, val.as_ptr() as usize));
        Self::new(
            eval_ctx,
            VariableIdentity::no_namespace(member.name.clone()),
            member_val,
            member.r#type.as_ref(),
        )
    }

    /// Visit variable children in bfs order.
    fn bfs_iterator(&self) -> BfsIterator {
        BfsIterator {
            queue: VecDeque::from([self]),
        }
    }

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

    fn assume_field_as_pointer(&self, field_name: &'static str) -> Result<*const (), AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Pointer(pointer) = child {
                    if pointer.identity.name.as_deref() != Some(field_name) {
                        return None;
                    }

                    return pointer.value;
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    fn assume_field_as_rust_enum(
        &self,
        field_name: &'static str,
    ) -> Result<RustEnumVariable, AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::RustEnum(r_enum) = child {
                    if r_enum.identity.name.as_deref() != Some(field_name) {
                        return None;
                    }

                    return Some(r_enum.clone());
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

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
                SpecializedVariableIR::String { original, .. } => &original.identity,
                SpecializedVariableIR::Str { original, .. } => &original.identity,
                SpecializedVariableIR::Tls { original, .. } => &original.identity,
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum AssumeError {
    #[error("field `{0}` not found")]
    FieldNotFound(&'static str),
    #[error("field `{0}` not a number")]
    FieldNotANumber(&'static str),
    #[error("incomplete interpretation of `{0}`")]
    IncompleteInterp(&'static str),
}

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
            VariableIR::Pointer(pointer) => {
                if let Some(val) = pointer.deref.as_ref() {
                    self.queue.push_back(val)
                }
            }
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { original, .. } => {
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
            },
            _ => {}
        }

        Some(next_item)
    }
}

fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> &T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { &*ptr.cast() }
}

#[cfg(test)]
mod test {
    use super::*;

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
                    members: vec![
                        VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::no_namespace(Some("array_1".to_owned())),
                            type_name: None,
                            items: Some(vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_2".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                            ]),
                        }),
                        VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::no_namespace(Some("array_2".to_owned())),
                            type_name: None,
                            items: Some(vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_3".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_4".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                            ]),
                        }),
                    ],
                }),
                expected_order: vec![
                    "struct_1", "array_1", "array_2", "scalar_1", "scalar_2", "scalar_3",
                    "scalar_4",
                ],
            },
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: VariableIdentity::no_namespace(Some("struct_1".to_owned())),
                    type_name: None,
                    members: vec![
                        VariableIR::Struct(StructVariable {
                            identity: VariableIdentity::no_namespace(Some("struct_2".to_owned())),
                            type_name: None,
                            members: vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                                VariableIR::RustEnum(RustEnumVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "enum_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: Some(Box::new(VariableIR::Scalar(ScalarVariable {
                                        identity: VariableIdentity::no_namespace(Some(
                                            "scalar_2".to_owned(),
                                        )),
                                        type_name: None,
                                        value: None,
                                    }))),
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_3".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                            ],
                        }),
                        VariableIR::Pointer(PointerVariable {
                            identity: VariableIdentity::no_namespace(Some("pointer_1".to_owned())),
                            type_name: None,
                            value: None,
                            deref: Some(Box::new(VariableIR::Scalar(ScalarVariable {
                                identity: VariableIdentity::no_namespace(Some(
                                    "scalar_4".to_owned(),
                                )),
                                type_name: None,
                                value: None,
                            }))),
                        }),
                    ],
                }),
                expected_order: vec![
                    "struct_1",
                    "struct_2",
                    "pointer_1",
                    "scalar_1",
                    "enum_1",
                    "scalar_3",
                    "scalar_4",
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
}
