use crate::debugger::variable::SpecializedVariableIR;
use crate::debugger::variable::VariableIR;
use std::borrow::Cow;

#[derive(Debug)]
pub enum ValueLayout<'a> {
    PreRendered(Cow<'a, str>),
    Referential {
        addr: *const (),
        val: &'a VariableIR,
    },
    Wrapped(&'a VariableIR),
    Nested(&'a [VariableIR]),
    Map(&'a [(VariableIR, VariableIR)]),
}

pub trait RenderRepr {
    fn name(&self) -> &str;
    fn r#type(&self) -> &str;
    fn value(&self) -> Option<ValueLayout>;
}

impl RenderRepr for VariableIR {
    fn name(&self) -> &str {
        let name = match self {
            VariableIR::Scalar(s) => &s.identity.name,
            VariableIR::Struct(s) => &s.identity.name,
            VariableIR::Array(a) => &a.identity.name,
            VariableIR::CEnum(e) => &e.identity.name,
            VariableIR::RustEnum(e) => &e.identity.name,
            VariableIR::Pointer(p) => return p.identity.name.as_deref().unwrap_or("anon"),
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, original } => match vec {
                    None => &original.identity.name,
                    Some(v) => &v.structure.identity.name,
                },
                SpecializedVariableIR::String { string, original } => match string {
                    None => &original.identity.name,
                    Some(s) => &s.identity.name,
                },
                SpecializedVariableIR::Str { string, original } => match string {
                    None => &original.identity.name,
                    Some(s) => &s.identity.name,
                },
                SpecializedVariableIR::Tls {
                    tls_var, original, ..
                } => match tls_var {
                    None => &original.identity.name,
                    Some(tls) => &tls.identity.name,
                },
                SpecializedVariableIR::HashMap { map, original } => match map {
                    None => &original.identity.name,
                    Some(map) => &map.identity.name,
                },
            },
        };

        let name = name.as_deref().unwrap_or("unknown");
        if name.starts_with("__") {
            let mb_num = name.trim_start_matches('_');
            if mb_num.parse::<u32>().is_ok() {
                return mb_num;
            }
        }
        name
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
                SpecializedVariableIR::Vector { vec, original } => match vec {
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
            },
        };
        r#type.as_deref().unwrap_or("unknown")
    }

    fn value(&self) -> Option<ValueLayout> {
        let value_repr = match self {
            VariableIR::Scalar(scalar) => {
                ValueLayout::PreRendered(Cow::Owned(scalar.value.as_ref()?.to_string()))
            }
            VariableIR::Struct(r#struct) => ValueLayout::Nested(r#struct.members.as_ref()),
            VariableIR::Array(array) => ValueLayout::Nested(array.items.as_deref()?),
            VariableIR::CEnum(r#enum) => {
                ValueLayout::PreRendered(Cow::Borrowed(r#enum.value.as_ref()?))
            }
            VariableIR::RustEnum(r#enum) => ValueLayout::Wrapped(r#enum.value.as_ref()?),
            VariableIR::Pointer(pointer) => {
                let ptr = pointer.value?;
                let val = pointer.deref.as_ref()?;
                ValueLayout::Referential { addr: ptr, val }
            }
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, original } => match vec {
                    None => ValueLayout::Nested(original.members.as_ref()),
                    Some(v) => ValueLayout::Nested(v.structure.members.as_ref()),
                },
                SpecializedVariableIR::String { string, original } => match string {
                    None => ValueLayout::Nested(original.members.as_ref()),
                    Some(s) => ValueLayout::PreRendered(Cow::Borrowed(&s.value)),
                },
                SpecializedVariableIR::Str { string, original } => match string {
                    None => ValueLayout::Nested(original.members.as_ref()),
                    Some(s) => ValueLayout::PreRendered(Cow::Borrowed(&s.value)),
                },
                SpecializedVariableIR::Tls {
                    tls_var: value,
                    original,
                } => match value {
                    None => ValueLayout::Nested(original.members.as_ref()),
                    Some(ref tls_val) => match tls_val.inner_value.as_ref() {
                        None => ValueLayout::PreRendered(Cow::Borrowed("uninit")),
                        Some(tls_inner_val) => tls_inner_val.value()?,
                    },
                },
                SpecializedVariableIR::HashMap { map, original } => match map {
                    None => ValueLayout::Nested(original.members.as_ref()),
                    Some(map) => ValueLayout::Map(&map.kv_items),
                },
            },
        };
        Some(value_repr)
    }
}
