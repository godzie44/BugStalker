use crate::debugger::variable::SpecializedVariableIR;
use crate::debugger::variable::VariableIR;
use std::borrow::Cow;

#[derive(Debug)]
pub enum ValueLayout<'a> {
    PreRendered(Cow<'a, str>),
    Referential {
        addr: *const (),
    },
    Wrapped(&'a VariableIR),
    Nested {
        members: &'a [VariableIR],
        named: bool,
    },
    Map(&'a [(VariableIR, VariableIR)]),
}

pub trait RenderRepr {
    fn name(&self) -> String;
    fn r#type(&self) -> &str;
    fn value(&self) -> Option<ValueLayout>;
}

impl RenderRepr for VariableIR {
    fn name(&self) -> String {
        self.identity().to_string()
    }

    fn r#type(&self) -> &str {
        let r#type = match self {
            VariableIR::Scalar(s) => &s.type_name,
            VariableIR::Struct(s) => &s.type_name,
            VariableIR::Array(a) => &a.type_name,
            VariableIR::CEnum(e) => &e.type_name,
            VariableIR::RustEnum(e) => &e.type_name,
            VariableIR::Pointer(p) => &p.type_name,
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, original }
                | SpecializedVariableIR::VecDeque { vec, original } => match vec {
                    None => &original.type_name,
                    Some(v) => &v.structure.type_name,
                },
                SpecializedVariableIR::String { .. } => return "String",
                SpecializedVariableIR::Str { .. } => return "&str",
                SpecializedVariableIR::Tls {
                    tls_var: value,
                    original,
                    ..
                } => match value {
                    None => &original.type_name,
                    Some(v) => &v.inner_type,
                },
                SpecializedVariableIR::HashMap { map, original } => match map {
                    None => &original.type_name,
                    Some(map) => &map.type_name,
                },
                SpecializedVariableIR::HashSet { set, original } => match set {
                    None => &original.type_name,
                    Some(set) => &set.type_name,
                },
                SpecializedVariableIR::BTreeMap { map, original } => match map {
                    None => &original.type_name,
                    Some(map) => &map.type_name,
                },
                SpecializedVariableIR::BTreeSet { set, original } => match set {
                    None => &original.type_name,
                    Some(set) => &set.type_name,
                },
                SpecializedVariableIR::Cell { original, .. }
                | SpecializedVariableIR::RefCell { original, .. } => &original.type_name,
                SpecializedVariableIR::Rc { original, .. }
                | SpecializedVariableIR::Arc { original, .. } => &original.type_name,
            },
        };
        r#type.as_deref().unwrap_or("unknown")
    }

    fn value(&self) -> Option<ValueLayout> {
        let value_repr = match self {
            VariableIR::Scalar(scalar) => {
                ValueLayout::PreRendered(Cow::Owned(scalar.value.as_ref()?.to_string()))
            }
            VariableIR::Struct(r#struct) => ValueLayout::Nested {
                members: r#struct.members.as_ref(),
                named: true,
            },
            VariableIR::Array(array) => ValueLayout::Nested {
                members: array.items.as_deref()?,
                named: true,
            },
            VariableIR::CEnum(r#enum) => {
                ValueLayout::PreRendered(Cow::Borrowed(r#enum.value.as_ref()?))
            }
            VariableIR::RustEnum(r#enum) => ValueLayout::Wrapped(r#enum.value.as_ref()?),
            VariableIR::Pointer(pointer) => {
                let ptr = pointer.value?;
                ValueLayout::Referential { addr: ptr }
            }
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, original }
                | SpecializedVariableIR::VecDeque { vec, original } => match vec {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(v) => ValueLayout::Nested {
                        members: v.structure.members.as_ref(),
                        named: true,
                    },
                },
                SpecializedVariableIR::String { string, original } => match string {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(s) => ValueLayout::PreRendered(Cow::Borrowed(&s.value)),
                },
                SpecializedVariableIR::Str { string, original } => match string {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(s) => ValueLayout::PreRendered(Cow::Borrowed(&s.value)),
                },
                SpecializedVariableIR::Tls {
                    tls_var: value,
                    original,
                } => match value {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(ref tls_val) => match tls_val.inner_value.as_ref() {
                        None => ValueLayout::PreRendered(Cow::Borrowed("uninit")),
                        Some(tls_inner_val) => tls_inner_val.value()?,
                    },
                },
                SpecializedVariableIR::HashMap { map, original } => match map {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(map) => ValueLayout::Map(&map.kv_items),
                },
                SpecializedVariableIR::HashSet { set, original } => match set {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(set) => ValueLayout::Nested {
                        members: &set.items,
                        named: false,
                    },
                },
                SpecializedVariableIR::BTreeMap { map, original } => match map {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(map) => ValueLayout::Map(&map.kv_items),
                },
                SpecializedVariableIR::BTreeSet { set, original } => match set {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(set) => ValueLayout::Nested {
                        members: &set.items,
                        named: false,
                    },
                },
                SpecializedVariableIR::Cell { value, original }
                | SpecializedVariableIR::RefCell { value, original } => match value {
                    Some(v) => v.value()?,
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                },
                SpecializedVariableIR::Rc { value, original }
                | SpecializedVariableIR::Arc { value, original } => match value {
                    None => ValueLayout::Nested {
                        members: original.members.as_ref(),
                        named: true,
                    },
                    Some(pointer) => {
                        let ptr = pointer.value?;
                        ValueLayout::Referential { addr: ptr }
                    }
                },
            },
        };
        Some(value_repr)
    }
}
