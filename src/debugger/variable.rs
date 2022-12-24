use crate::debugger::dwarf::r#type::{ArrayDeclaration, StructureMember};
use crate::debugger::{Debugger, EventHook, TypeDeclaration};
use bytes::Bytes;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::mem;

pub struct GenericVariable<'a, T: EventHook> {
    pub(super) debugger: &'a Debugger<T>,
    pub name: Option<Cow<'a, str>>,
    pub r#type: Option<TypeDeclaration<'a>>,
    pub value: Option<Bytes>,
}

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

pub struct ScalarVariableIR {
    name: String,
    r#type: String,
    value: Option<SupportedScalar>,
}

pub struct StructIR {
    name: String,
    r#type: String,
    #[allow(unused)]
    fields_search: HashMap<String, usize>,
    fields: Option<Vec<VariableIR>>,
}

pub struct ArrayIR {
    name: String,
    r#type: String,
    items: Option<Vec<VariableIR>>,
}

pub struct CEnumIR {
    name: String,
    r#type: String,
    value: Option<String>,
}

pub struct RustEnumIR {
    name: String,
    r#type: String,
    value: Option<Box<VariableIR>>,
}

pub struct PointerIR {
    name: String,
    r#type: String,
    value: Option<*const ()>,
    deref: Option<Box<VariableIR>>,
}

pub enum IRValueRepr<'a> {
    Rendered(Cow<'a, str>),
    Referential {
        addr: *const (),
        val: &'a VariableIR,
    },
    Wrapped(&'a VariableIR),
    Nested(&'a [VariableIR]),
}

pub enum VariableIR {
    Scalar(ScalarVariableIR),
    Struct(StructIR),
    Array(ArrayIR),
    CEnum(CEnumIR),
    RustEnum(RustEnumIR),
    Pointer(PointerIR),
}

impl VariableIR {
    /// Return item name, prettify if name is a position in tuple.
    pub fn name(&self) -> &str {
        let name = match self {
            VariableIR::Scalar(s) => &s.name,
            VariableIR::Struct(s) => &s.name,
            VariableIR::Array(a) => &a.name,
            VariableIR::CEnum(e) => &e.name,
            VariableIR::RustEnum(e) => &e.name,
            VariableIR::Pointer(p) => &p.name,
        };
        if name.starts_with("__") {
            let mb_num = name.trim_start_matches('_');
            if mb_num.parse::<u32>().is_ok() {
                return mb_num;
            }
        }
        name
    }

    pub fn value(&self) -> Option<IRValueRepr> {
        let value_repr = match self {
            VariableIR::Scalar(scalar) => {
                IRValueRepr::Rendered(Cow::Owned(scalar.value.as_ref()?.to_string()))
            }
            VariableIR::Struct(r#struct) => IRValueRepr::Nested(r#struct.fields.as_deref()?),
            VariableIR::Array(array) => IRValueRepr::Nested(array.items.as_deref()?),
            VariableIR::CEnum(r#enum) => {
                IRValueRepr::Rendered(Cow::Borrowed(r#enum.value.as_ref()?))
            }
            VariableIR::RustEnum(r#enum) => IRValueRepr::Wrapped(r#enum.value.as_ref()?),
            VariableIR::Pointer(pointer) => {
                let ptr = pointer.value?;
                let val = pointer.deref.as_ref()?;
                IRValueRepr::Referential { addr: ptr, val }
            }
        };
        Some(value_repr)
    }

    pub fn r#type(&self) -> &str {
        match self {
            VariableIR::Scalar(s) => &s.r#type,
            VariableIR::Struct(s) => &s.r#type,
            VariableIR::Array(a) => &a.r#type,
            VariableIR::CEnum(e) => &e.r#type,
            VariableIR::RustEnum(e) => &e.r#type,
            VariableIR::Pointer(p) => &p.r#type,
        }
    }
}

impl<'a, T: EventHook + 'static> GenericVariable<'a, T> {
    fn name_cloned(&self) -> String {
        self.name
            .clone()
            .map(Into::into)
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub fn as_ir(&self) -> VariableIR {
        match &self.r#type {
            Some(TypeDeclaration::Scalar { name, .. }) => {
                VariableIR::Scalar(self.as_scalar_ir(name.as_deref()))
            }
            Some(TypeDeclaration::Structure { name, members, .. }) => {
                VariableIR::Struct(self.as_struct_ir(name.as_deref(), members))
            }
            Some(TypeDeclaration::Array(arr)) => VariableIR::Array(self.as_array_ir(arr)),
            Some(TypeDeclaration::CStyleEnum {
                name,
                discr_type,
                enumerators,
                ..
            }) => VariableIR::CEnum(self.as_c_enum_ir(
                name.as_deref(),
                discr_type.clone().map(|t| *t),
                enumerators,
            )),
            Some(TypeDeclaration::RustEnum {
                name,
                discr_type: discr_member,
                enumerators,
                ..
            }) => VariableIR::RustEnum(self.as_rust_enum_ir(
                name.as_deref(),
                discr_member,
                enumerators,
            )),
            Some(TypeDeclaration::Pointer { name, target_type }) => {
                VariableIR::Pointer(self.as_pointer_ir(name.as_deref(), target_type))
            }
            _ => {
                todo!()
            }
        }
    }

    fn as_scalar_ir(&self, type_name: Option<&str>) -> ScalarVariableIR {
        fn render_scalar<S: Copy + Display, T: EventHook>(var: &GenericVariable<T>) -> Option<S> {
            var.value.as_ref().map(|v| *scalar_from_bytes::<S>(v))
        }

        let type_view = self
            .r#type
            .as_ref()
            .and_then(|ty| ty.name())
            .unwrap_or_else(|| "unknown".to_string());

        let value_view = match type_name {
            Some("i8") => render_scalar::<i8, _>(self).map(SupportedScalar::I8),
            Some("i16") => render_scalar::<i16, _>(self).map(SupportedScalar::I16),
            Some("i32") => render_scalar::<i32, _>(self).map(SupportedScalar::I32),
            Some("i64") => render_scalar::<i64, _>(self).map(SupportedScalar::I64),
            Some("i128") => render_scalar::<i128, _>(self).map(SupportedScalar::I128),
            Some("isize") => render_scalar::<isize, _>(self).map(SupportedScalar::Isize),
            Some("u8") => render_scalar::<u8, _>(self).map(SupportedScalar::U8),
            Some("u16") => render_scalar::<u16, _>(self).map(SupportedScalar::U16),
            Some("u32") => render_scalar::<u32, _>(self).map(SupportedScalar::U32),
            Some("u64") => render_scalar::<u64, _>(self).map(SupportedScalar::U64),
            Some("u128") => render_scalar::<u128, _>(self).map(SupportedScalar::U128),
            Some("usize") => render_scalar::<usize, _>(self).map(SupportedScalar::Usize),
            Some("f32") => render_scalar::<f32, _>(self).map(SupportedScalar::F32),
            Some("f64") => render_scalar::<f64, _>(self).map(SupportedScalar::F64),
            Some("bool") => render_scalar::<bool, _>(self).map(SupportedScalar::Bool),
            Some("char") => render_scalar::<char, _>(self).map(SupportedScalar::Char),
            Some("()") => Some(SupportedScalar::Empty()),
            _ => None,
        };
        ScalarVariableIR {
            name: self.name_cloned(),
            r#type: type_view,
            value: value_view,
        }
    }

    fn as_array_ir(&self, arr_decl: &ArrayDeclaration) -> ArrayIR {
        let items = arr_decl.bounds(self.debugger.pid).and_then(|bounds| {
            let len = bounds.1 - bounds.0;
            let el_size = arr_decl.size_in_bytes(self.debugger.pid)? / len as u64;
            let bytes = self.value.as_ref()?;
            Some(
                bytes
                    .chunks(el_size as usize)
                    .enumerate()
                    .map(|(i, chunk)| GenericVariable {
                        debugger: self.debugger,
                        name: Some(Cow::Owned(format!("{}", bounds.0 + i as i64))),
                        r#type: arr_decl.element_type.as_ref().map(|et| *et.clone()),
                        value: Some(bytes.slice_ref(chunk)),
                    })
                    .map(|var| var.as_ir())
                    .collect::<Vec<_>>(),
            )
        });

        ArrayIR {
            name: self.name_cloned(),
            r#type: self
                .r#type
                .as_ref()
                .and_then(|ty| ty.name())
                .unwrap_or_else(|| "unknown".to_string()),
            items,
        }
    }

    fn as_struct_ir(&self, type_name: Option<&str>, members: &[StructureMember]) -> StructIR {
        let mut children = Vec::with_capacity(members.len());
        let mut search = HashMap::new();

        for member in members {
            let member_as_var = self.split_by_member(member);
            let child = member_as_var.as_ir();
            let field_name = child.name().to_string();
            children.push(child);
            search.insert(field_name, children.len());
        }

        StructIR {
            name: self.name_cloned(),
            r#type: type_name.unwrap_or("unknown").to_string(),
            fields: Some(children),
            fields_search: search,
        }
    }

    fn as_c_enum_ir(
        &self,
        name: Option<&str>,
        discr_type: Option<TypeDeclaration>,
        enumerators: &HashMap<i64, String>,
    ) -> CEnumIR {
        let discr = GenericVariable {
            debugger: self.debugger,
            name: None,
            r#type: discr_type,
            value: self.value.clone(),
        };
        let value = discr.as_discriminator();

        CEnumIR {
            name: self.name_cloned(),
            r#type: name.unwrap_or("unknown").to_string(),
            value: value.and_then(|val| enumerators.get(&(val as i64)).cloned()),
        }
    }

    fn as_rust_enum_ir(
        &self,
        name: Option<&str>,
        discr_member: &Option<Box<StructureMember>>,
        enumerators: &HashMap<Option<i64>, StructureMember>,
    ) -> RustEnumIR {
        let value = discr_member.as_ref().and_then(|member| {
            let discr_as_var = self.split_by_member(member);
            discr_as_var.as_discriminator()
        });

        let enumerator =
            value.and_then(|v| enumerators.get(&Some(v)).or_else(|| enumerators.get(&None)));

        let enumerator = enumerator.map(|member| {
            let member_as_var = self.split_by_member(member);
            member_as_var.as_ir()
        });

        RustEnumIR {
            name: self.name_cloned(),
            r#type: name.unwrap_or("unknown").to_string(),
            value: enumerator.map(Box::new),
        }
    }

    fn as_pointer_ir(
        &self,
        name: Option<&str>,
        target_type: &Option<Box<TypeDeclaration>>,
    ) -> PointerIR {
        let mb_ptr = self
            .value
            .as_ref()
            .map(scalar_from_bytes::<*const ()>)
            .copied();

        let deref_var = mb_ptr.map(|ptr| {
            let read_size = target_type
                .as_ref()
                .and_then(|t| t.size_in_bytes(self.debugger.pid));

            let val =
                read_size.and_then(|sz| self.debugger.read_memory(ptr as usize, sz as usize).ok());

            GenericVariable {
                debugger: self.debugger,
                name: Some(Cow::from("*")),
                value: val.map(Bytes::from),
                r#type: target_type.clone().map(|t| *t),
            }
        });
        let target_item = deref_var.map(|var| Box::new(var.as_ir()));

        PointerIR {
            name: self.name_cloned(),
            r#type: name.unwrap_or("unknown").to_string(),
            value: mb_ptr,
            deref: target_item,
        }
    }

    fn split_by_member(&self, member: &'a StructureMember) -> Self {
        let member_val = self
            .value
            .as_ref()
            .and_then(|val| member.value(val.as_ptr() as usize, self.debugger.pid));

        GenericVariable {
            debugger: self.debugger,
            name: member.name.as_ref().map(|n| Cow::Borrowed(n.as_str())),
            r#type: member.r#type.clone(),
            value: member_val,
        }
    }

    fn as_discriminator(&self) -> Option<i64> {
        if let Some(TypeDeclaration::Scalar { name, .. }) = self.r#type.as_ref() {
            return match name.as_deref() {
                Some("u8") => self
                    .value
                    .as_ref()
                    .map(|v| *scalar_from_bytes::<u8>(v) as i64),
                Some("u16") => self
                    .value
                    .as_ref()
                    .map(|v| *scalar_from_bytes::<u16>(v) as i64),
                Some("u32") => self
                    .value
                    .as_ref()
                    .map(|v| *scalar_from_bytes::<u32>(v) as i64),
                Some("u64") => self
                    .value
                    .as_ref()
                    .map(|v| *scalar_from_bytes::<u64>(v) as i64),
                _ => None,
            };
        }
        None
    }
}

fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> &T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { &*ptr.cast() }
}
