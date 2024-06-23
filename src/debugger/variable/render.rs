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
            VariableIR::Specialized {
                value: Some(spec_val),
                original,
            } => match spec_val {
                SpecializedVariableIR::Vector(vec) | SpecializedVariableIR::VecDeque(vec) => {
                    &vec.structure.type_ident
                }
                SpecializedVariableIR::String { .. } => &STRING_TYPE,
                SpecializedVariableIR::Str { .. } => &STR_TYPE,
                SpecializedVariableIR::Tls(value) => &value.inner_type,
                SpecializedVariableIR::HashMap(map) => &map.type_ident,
                SpecializedVariableIR::HashSet(set) => &set.type_ident,
                SpecializedVariableIR::BTreeMap(map) => &map.type_ident,
                SpecializedVariableIR::BTreeSet(set) => &set.type_ident,
                SpecializedVariableIR::Cell(_) | SpecializedVariableIR::RefCell(_) => {
                    &original.type_ident
                }
                SpecializedVariableIR::Rc(_) | SpecializedVariableIR::Arc(_) => {
                    &original.type_ident
                }
                SpecializedVariableIR::Uuid(_) => &original.type_ident,
                SpecializedVariableIR::SystemTime(_) => &original.type_ident,
                SpecializedVariableIR::Instant(_) => &original.type_ident,
            },
            VariableIR::Specialized { original, .. } => &original.type_ident,

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
            VariableIR::Specialized {
                value: Some(spec_val),
                ..
            } => match spec_val {
                SpecializedVariableIR::Vector(vec) | SpecializedVariableIR::VecDeque(vec) => {
                    ValueLayout::List {
                        members: vec.structure.members.as_ref(),
                        indexed: true,
                    }
                }
                SpecializedVariableIR::String(string) => {
                    ValueLayout::PreRendered(Cow::Borrowed(&string.value))
                }
                SpecializedVariableIR::Str(string) => {
                    ValueLayout::PreRendered(Cow::Borrowed(&string.value))
                }
                SpecializedVariableIR::Tls(tls_value) => match tls_value.inner_value.as_ref() {
                    None => ValueLayout::PreRendered(Cow::Borrowed("uninit")),
                    Some(tls_inner_val) => tls_inner_val.value()?,
                },
                SpecializedVariableIR::HashMap(map) => ValueLayout::Map(&map.kv_items),
                SpecializedVariableIR::HashSet(set) => ValueLayout::List {
                    members: &set.items,
                    indexed: false,
                },
                SpecializedVariableIR::BTreeMap(map) => ValueLayout::Map(&map.kv_items),
                SpecializedVariableIR::BTreeSet(set) => ValueLayout::List {
                    members: &set.items,
                    indexed: false,
                },
                SpecializedVariableIR::Cell(cell) | SpecializedVariableIR::RefCell(cell) => {
                    cell.value()?
                }
                SpecializedVariableIR::Rc(ptr) | SpecializedVariableIR::Arc(ptr) => {
                    let ptr = ptr.value?;
                    ValueLayout::Referential { addr: ptr }
                }
                SpecializedVariableIR::Uuid(bytes) => {
                    let uuid = uuid::Uuid::from_slice(bytes).expect("infallible");
                    ValueLayout::PreRendered(Cow::Owned(uuid.to_string()))
                }
                SpecializedVariableIR::SystemTime((sec, n_sec)) => {
                    let mb_dt = chrono::NaiveDateTime::from_timestamp_opt(*sec, *n_sec);
                    let dt_rendered = mb_dt
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or("Broken date time".to_string());
                    ValueLayout::PreRendered(Cow::Owned(dt_rendered))
                }
                SpecializedVariableIR::Instant((sec, n_sec)) => {
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
            VariableIR::Specialized { original, .. } => ValueLayout::Structure {
                members: original.members.as_ref(),
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
