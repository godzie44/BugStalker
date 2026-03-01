use crate::debugger::debugee::dwarf::r#type::TypeIdentity;
use crate::debugger::variable::value::{ArrayItem, Member, SpecializedValue, Value};
use nix::errno::Errno;
use nix::libc;
use nix::sys::time::TimeSpec;
use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};
use std::mem::MaybeUninit;
use std::ops::Sub;
use std::time::Duration;

/// Layout of a value from debugee program.
/// Used by UI for representing value to a user.
pub enum ValueLayout<'a> {
    /// Value already rendered, just print it!
    PreRendered(Cow<'a, str>),
    /// Value is an address in debugee memory.
    Referential(*const ()),
    /// Value wraps another value.
    Wrapped(&'a Value),
    /// Value is a structure.
    Structure(&'a [Member]),
    /// Value is a list with indexed elements.
    IndexedList(&'a [ArrayItem]),
    /// Value is an unordered list.
    NonIndexedList(&'a [Value]),
    /// Value is a map where keys and values are values too.
    Map(&'a [(Value, Value)]),
}

impl Debug for ValueLayout<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueLayout::PreRendered(s) => f.debug_tuple("PreRendered").field(s).finish(),
            ValueLayout::Referential(addr) => f.debug_tuple("Referential").field(addr).finish(),
            ValueLayout::Wrapped(v) => f.debug_tuple("Wrapped").field(&v).finish(),
            ValueLayout::Structure(members) => {
                f.debug_struct("Nested").field("members", members).finish()
            }
            ValueLayout::Map(kvs) => {
                let mut list = f.debug_list();
                for kv in kvs.iter() {
                    list.entry(&kv);
                }
                list.finish()
            }
            ValueLayout::IndexedList(items) => {
                f.debug_struct("List").field("items", items).finish()
            }
            ValueLayout::NonIndexedList(items) => {
                f.debug_struct("List").field("items", items).finish()
            }
        }
    }
}

pub trait RenderValue {
    /// Return type identity for rendering.
    fn r#type(&self) -> &TypeIdentity;

    /// Return value layout for rendering.
    fn value_layout(&self) -> Option<ValueLayout<'_>>;
}

impl RenderValue for Value {
    fn r#type(&self) -> &TypeIdentity {
        static STRING_TYPE: Lazy<TypeIdentity> = Lazy::new(|| TypeIdentity::no_namespace("String"));
        static STR_TYPE: Lazy<TypeIdentity> = Lazy::new(|| TypeIdentity::no_namespace("&str"));
        static UNKNOWN_TYPE: Lazy<TypeIdentity> = Lazy::new(TypeIdentity::unknown);

        match self {
            Value::Scalar(s) => &s.type_ident,
            Value::Struct(s) => &s.type_ident,
            Value::Array(a) => &a.type_ident,
            Value::CEnum(e) => &e.type_ident,
            Value::RustEnum(e) => &e.type_ident,
            Value::Pointer(p) => &p.type_ident,
            Value::Specialized {
                value: Some(spec_val),
                original,
            } => match spec_val {
                SpecializedValue::Vector(vec) | SpecializedValue::VecDeque(vec) => {
                    &vec.structure.type_ident
                }
                SpecializedValue::String { .. } => &STRING_TYPE,
                SpecializedValue::Str { .. } => &STR_TYPE,
                SpecializedValue::Tls(value) => &value.inner_type,
                SpecializedValue::HashMap(map) => &map.type_ident,
                SpecializedValue::HashSet(set) => &set.type_ident,
                SpecializedValue::BTreeMap(map) => &map.type_ident,
                SpecializedValue::BTreeSet(set) => &set.type_ident,
                SpecializedValue::Cell(_) | SpecializedValue::RefCell(_) => &original.type_ident,
                SpecializedValue::Rc(_) | SpecializedValue::Arc(_) => &original.type_ident,
                SpecializedValue::Uuid(_) => &original.type_ident,
                SpecializedValue::SystemTime(_) => &original.type_ident,
                SpecializedValue::Instant(_) => &original.type_ident,
            },
            Value::Specialized { original, .. } => &original.type_ident,
            Value::Subroutine(_) => {
                // currently this line is unreachable because dereference of fn pointer is forbidden
                &UNKNOWN_TYPE
            }
            Value::CModifiedVariable(v) => &v.type_ident,
        }
    }

    fn value_layout(&self) -> Option<ValueLayout<'_>> {
        let value_repr = match self {
            Value::Scalar(scalar) => {
                ValueLayout::PreRendered(Cow::Owned(scalar.value.as_ref()?.to_string()))
            }
            Value::Struct(r#struct) => ValueLayout::Structure(r#struct.members.as_ref()),
            Value::Array(array) => ValueLayout::IndexedList(array.items.as_deref()?),
            Value::CEnum(r#enum) => ValueLayout::PreRendered(Cow::Borrowed(r#enum.value.as_ref()?)),
            Value::RustEnum(r#enum) => {
                let enum_val = &r#enum.value.as_ref()?.value;
                ValueLayout::Wrapped(enum_val)
            }
            Value::Pointer(pointer) => {
                let ptr = pointer.value?;
                ValueLayout::Referential(ptr)
            }
            Value::Specialized {
                value: Some(spec_val),
                ..
            } => match spec_val {
                SpecializedValue::Vector(vec) | SpecializedValue::VecDeque(vec) => {
                    ValueLayout::Structure(vec.structure.members.as_ref())
                }
                SpecializedValue::String(string) => {
                    ValueLayout::PreRendered(Cow::Borrowed(&string.value))
                }
                SpecializedValue::Str(string) => {
                    ValueLayout::PreRendered(Cow::Borrowed(&string.value))
                }
                SpecializedValue::Tls(tls_value) => match tls_value.inner_value.as_ref() {
                    None => ValueLayout::PreRendered(Cow::Borrowed("uninit")),
                    Some(tls_inner_val) => tls_inner_val.value_layout()?,
                },
                SpecializedValue::HashMap(map) => ValueLayout::Map(&map.kv_items),
                SpecializedValue::HashSet(set) => ValueLayout::NonIndexedList(&set.items),
                SpecializedValue::BTreeMap(map) => ValueLayout::Map(&map.kv_items),
                SpecializedValue::BTreeSet(set) => ValueLayout::NonIndexedList(&set.items),
                SpecializedValue::Cell(cell) | SpecializedValue::RefCell(cell) => {
                    cell.value_layout()?
                }
                SpecializedValue::Rc(ptr) | SpecializedValue::Arc(ptr) => {
                    let ptr = ptr.value?;
                    ValueLayout::Referential(ptr)
                }
                SpecializedValue::Uuid(bytes) => {
                    let uuid = uuid::Uuid::from_slice(bytes).expect("infallible");
                    ValueLayout::PreRendered(Cow::Owned(uuid.to_string()))
                }
                SpecializedValue::SystemTime((sec, n_sec)) => {
                    let mb_dt = chrono::DateTime::from_timestamp(*sec, *n_sec);
                    let dt_rendered = mb_dt
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or("Broken date time".to_string());
                    ValueLayout::PreRendered(Cow::Owned(dt_rendered))
                }
                SpecializedValue::Instant((sec, n_sec)) => {
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
            Value::Specialized { original, .. } => {
                ValueLayout::Structure(original.members.as_ref())
            }
            Value::Subroutine(_) => {
                // currently this line is unreachable because dereference of fn pointer is forbidden
                return None;
            }
            Value::CModifiedVariable(v) => ValueLayout::Wrapped(v.value.as_ref()?),
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
