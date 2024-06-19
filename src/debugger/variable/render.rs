use crate::debugger::debugee::dwarf::r#type::TypeIdentity;
use crate::debugger::variable::SpecializedVariableIR;
use crate::debugger::variable::VariableIR;
use nix::errno::Errno;
use nix::libc;
use nix::sys::time::TimeSpec;
use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::mem::MaybeUninit;
use std::ops::Sub;
use std::time::Duration;

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

impl Debug for ValueLayout<'_> {
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
    fn r#type(&self) -> &TypeIdentity;
    fn value(&self) -> Option<ValueLayout>;
}

impl RenderRepr for VariableIR {
    fn name(&self) -> String {
        self.identity().to_string()
    }

    fn r#type(&self) -> &TypeIdentity {
        static STRING_TYPE: Lazy<TypeIdentity> = Lazy::new(|| TypeIdentity::no_namespace("String"));
        static STR_TYPE: Lazy<TypeIdentity> = Lazy::new(|| TypeIdentity::no_namespace("&str"));
        static UNKNOWN_TYPE: Lazy<TypeIdentity> = Lazy::new(TypeIdentity::unknown);

        match self {
            VariableIR::Scalar(s) => &s.type_ident,
            VariableIR::Struct(s) => &s.type_ident,
            VariableIR::Array(a) => &a.type_ident,
            VariableIR::CEnum(e) => &e.type_ident,
            VariableIR::RustEnum(e) => &e.type_ident,
            VariableIR::Pointer(p) => &p.type_ident,
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, original }
                | SpecializedVariableIR::VecDeque { vec, original } => match vec {
                    None => &original.type_ident,
                    Some(v) => &v.structure.type_ident,
                },
                SpecializedVariableIR::String { .. } => &STRING_TYPE,
                SpecializedVariableIR::Str { .. } => &STR_TYPE,
                SpecializedVariableIR::Tls {
                    tls_var: value,
                    original,
                    ..
                } => match value {
                    None => &original.type_ident,
                    Some(v) => &v.inner_type,
                },
                SpecializedVariableIR::HashMap { map, original } => match map {
                    None => &original.type_ident,
                    Some(map) => &map.type_ident,
                },
                SpecializedVariableIR::HashSet { set, original } => match set {
                    None => &original.type_ident,
                    Some(set) => &set.type_ident,
                },
                SpecializedVariableIR::BTreeMap { map, original } => match map {
                    None => &original.type_ident,
                    Some(map) => &map.type_ident,
                },
                SpecializedVariableIR::BTreeSet { set, original } => match set {
                    None => &original.type_ident,
                    Some(set) => &set.type_ident,
                },
                SpecializedVariableIR::Cell { original, .. }
                | SpecializedVariableIR::RefCell { original, .. } => &original.type_ident,
                SpecializedVariableIR::Rc { original, .. }
                | SpecializedVariableIR::Arc { original, .. } => &original.type_ident,
                SpecializedVariableIR::Uuid { original, .. } => &original.type_ident,
                SpecializedVariableIR::SystemTime { original, .. } => &original.type_ident,
                SpecializedVariableIR::Instant { original, .. } => &original.type_ident,
            },
            VariableIR::Subroutine(_) => {
                // currently this line is unreachable cause dereference fn pointer is forbidden
                &UNKNOWN_TYPE
            }
            VariableIR::CModifiedVariable(v) => &v.type_ident,
        }
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
                    Some(tls_val) => match tls_val.inner_value.as_ref() {
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
                SpecializedVariableIR::Uuid { value, original } => match value {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some(array) => {
                        let uuid = uuid::Uuid::from_slice(array).expect("infallible");
                        ValueLayout::PreRendered(Cow::Owned(uuid.to_string()))
                    }
                },
                SpecializedVariableIR::SystemTime { value, original } => match value {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some((sec, n_sec)) => {
                        let mb_dt = chrono::NaiveDateTime::from_timestamp_opt(*sec, *n_sec);
                        let dt_rendered = mb_dt
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or("Broken date time".to_string());
                        ValueLayout::PreRendered(Cow::Owned(dt_rendered))
                    }
                },
                SpecializedVariableIR::Instant { value, original } => match value {
                    None => ValueLayout::Structure {
                        members: original.members.as_ref(),
                    },
                    Some((sec, n_sec)) => {
                        let now = now_timespec().expect("broken system clock");
                        let instant = TimeSpec::new(*sec, *n_sec as i64);
                        let render = if now > instant {
                            let from_instant = Duration::from(now.sub(instant));
                            format!("already happened {} seconds ago ", from_instant.as_secs())
                        } else {
                            let from_now = Duration::from(instant.sub(now));
                            format!("{} seconds from now", from_now.as_secs())
                        };

                        ValueLayout::PreRendered(Cow::Owned(render))
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

fn now_timespec() -> Result<TimeSpec, Errno> {
    let mut t = MaybeUninit::uninit();
    let res = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, t.as_mut_ptr()) };
    if res == -1 {
        return Err(Errno::last());
    }
    let t = unsafe { t.assume_init() };
    Ok(TimeSpec::new(t.tv_sec, t.tv_nsec))
}
