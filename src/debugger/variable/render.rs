use crate::debugger::variable::SpecializedVariableIR;
use crate::debugger::variable::VariableIR;
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};

pub enum ValueLayout<'a> {
    PreRendered(Cow<'a, str>),
    Referential {
        addr: *const (),
    },
    Wrapped(&'a VariableIR),
    Structure {
        members: &'a [VariableIR],
    },
    List {
        members: &'a [VariableIR],
        indexed: bool,
    },
    Map(&'a [(VariableIR, VariableIR)]),
}

impl<'a> Debug for ValueLayout<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueLayout::PreRendered(s) => f.debug_tuple("PreRendered").field(s).finish(),
            ValueLayout::Referential { addr, .. } => {
                f.debug_tuple("Referential").field(addr).finish()
            }
            ValueLayout::Wrapped(v) => f.debug_tuple("Wrapped").field(v).finish(),
            ValueLayout::Structure { members } => {
                f.debug_struct("Nested").field("members", members).finish()
            }
            ValueLayout::Map(kvs) => {
                let mut list = f.debug_list();
                for kv in kvs.iter() {
                    list.entry(kv);
                }
                list.finish()
            }
            ValueLayout::List { members, indexed } => f
                .debug_struct("List")
                .field("members", members)
                .field("indexed", indexed)
                .finish(),
        }
    }
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
            VariableIR::Subroutine(_) => {
                // currently this line is unreachable cause dereference fn pointer is forbidden
                &None
            }
            VariableIR::CModifiedVariable(v) => &v.type_name,
        };
        r#type.as_deref().unwrap_or("unknown")
    }

    fn value(&self) -> Option<ValueLayout> {
        let value_repr = match self {
            VariableIR::Scalar(scalar) => {
                ValueLayout::PreRendered(Cow::Owned(scalar.value.as_ref()?.to_string()))
            }
            VariableIR::Struct(r#struct) => ValueLayout::Structure {
                members: r#struct.members.as_ref(),
            },
            VariableIR::Array(array) => ValueLayout::List {
                members: array.items.as_deref()?,
                indexed: true,
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
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(v) => ValueLayout::List {
                        members: v.structure.members.as_ref(),
                        indexed: true,
                    },
                },
                SpecializedVariableIR::String { string, original } => match string {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(s) => ValueLayout::PreRendered(Cow::Borrowed(&s.value)),
                },
                SpecializedVariableIR::Str { string, original } => match string {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(s) => ValueLayout::PreRendered(Cow::Borrowed(&s.value)),
                },
                SpecializedVariableIR::Tls {
                    tls_var: value,
                    original,
                } => match value {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(ref tls_val) => match tls_val.inner_value.as_ref() {
                        None => ValueLayout::PreRendered(Cow::Borrowed("uninit")),
                        Some(tls_inner_val) => tls_inner_val.value()?,
                    },
                },
                SpecializedVariableIR::HashMap { map, original } => match map {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(map) => ValueLayout::Map(&map.kv_items),
                },
                SpecializedVariableIR::HashSet { set, original } => match set {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(set) => ValueLayout::List {
                        members: &set.items,
                        indexed: false,
                    },
                },
                SpecializedVariableIR::BTreeMap { map, original } => match map {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(map) => ValueLayout::Map(&map.kv_items),
                },
                SpecializedVariableIR::BTreeSet { set, original } => match set {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(set) => ValueLayout::List {
                        members: &set.items,
                        indexed: false,
                    },
                },
                SpecializedVariableIR::Cell { value, original }
                | SpecializedVariableIR::RefCell { value, original } => match value {
                    Some(v) => v.value()?,
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                },
                SpecializedVariableIR::Rc { value, original }
                | SpecializedVariableIR::Arc { value, original } => match value {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(pointer) => {
                        let ptr = pointer.value?;
                        ValueLayout::Referential { addr: ptr }
                    }
                },
            },
            VariableIR::Subroutine(_) => {
                // currently this line is unreachable a cause dereference fn pointer is forbidden
                return None;
            }
            VariableIR::CModifiedVariable(v) => ValueLayout::Wrapped(v.value.as_ref()?),
        };
        Some(value_repr)
    }
}
