use crate::debugger::dwarf::r#type::StructureMember;
use crate::debugger::{Debugger, EventHook, TypeDeclaration};
use bytes::Bytes;
use nix::unistd::Pid;
use std::borrow::Cow;
use std::fmt::Display;
use std::mem;

pub struct Variable<'a> {
    pub name: Option<Cow<'a, str>>,
    pub r#type: Option<TypeDeclaration>,
    pub value: Option<Bytes>,
}

pub type RenderView = RenderItem;

pub struct RenderItem {
    pub name: String,
    pub r#type: String,
    pub value: Option<String>,
    pub children: Vec<RenderItem>,
}

impl<'a> Variable<'a> {
    fn name_cloned(&self) -> String {
        self.name
            .clone()
            .map(Into::into)
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn split_by_member(&self, member: &'a StructureMember, pid: Pid) -> Self {
        let member_val = self
            .value
            .as_ref()
            .and_then(|val| member.value(val.as_ptr() as usize, pid));

        Variable {
            name: member.name.as_ref().map(|n| Cow::Borrowed(n.as_str())),
            r#type: member.r#type.clone(),
            value: member_val,
        }
    }

    pub fn render<T: EventHook>(&self, debugger: &Debugger<T>) -> RenderView {
        fn render_scalar<T: Copy + Display>(var: &Variable) -> (String, Option<String>) {
            let type_view = var
                .r#type
                .as_ref()
                .and_then(|ty| ty.name())
                .unwrap_or_else(|| "unknown".to_string());

            let value_view = var
                .value
                .as_ref()
                .map(|v| {
                    let v = scalar_from_bytes::<T>(v);
                    format!("{v}")
                })
                .unwrap_or_else(|| "unknown".to_string());

            (type_view, Some(value_view))
        }

        fn make_render_item<T: EventHook>(var: &Variable, debugger: &Debugger<T>) -> RenderItem {
            match &var.r#type {
                Some(TypeDeclaration::Scalar { name, .. }) => {
                    let (type_view, value_view) = match name.as_deref() {
                        Some("i8") => render_scalar::<i8>(var),
                        Some("i16") => render_scalar::<i16>(var),
                        Some("i32") => render_scalar::<i32>(var),
                        Some("i64") => render_scalar::<i64>(var),
                        Some("i128") => render_scalar::<i128>(var),
                        Some("isize") => render_scalar::<isize>(var),
                        Some("u8") => render_scalar::<u8>(var),
                        Some("u16") => render_scalar::<u16>(var),
                        Some("u32") => render_scalar::<u32>(var),
                        Some("u64") => render_scalar::<u64>(var),
                        Some("u128") => render_scalar::<u128>(var),
                        Some("usize") => render_scalar::<usize>(var),
                        Some("f32") => render_scalar::<f32>(var),
                        Some("f64") => render_scalar::<f64>(var),
                        Some("bool") => render_scalar::<bool>(var),
                        Some("char") => render_scalar::<char>(var),
                        Some("()") => ("()".to_string(), None),
                        _ => ("unknown".to_string(), None),
                    };
                    RenderItem {
                        name: var.name_cloned(),
                        r#type: type_view,
                        value: value_view,
                        children: vec![],
                    }
                }
                Some(TypeDeclaration::Structure { name, members, .. }) => {
                    let mut item = RenderItem {
                        name: var.name_cloned(),
                        r#type: name.as_deref().unwrap_or("unknown").to_string(),
                        value: None,
                        children: Vec::with_capacity(members.len()),
                    };

                    for member in members {
                        let member_as_var = var.split_by_member(member, debugger.pid);
                        item.children
                            .push(make_render_item(&member_as_var, debugger));
                    }

                    item
                }
                Some(TypeDeclaration::Array(arr)) => {
                    let bounds = arr.bounds(debugger.pid).unwrap();
                    let el_count = bounds.1 - bounds.0;
                    let el_size = arr.size_in_bytes(debugger.pid).unwrap() / el_count as u64;
                    let bytes = var.value.as_ref().unwrap();
                    let children = bytes
                        .chunks(el_size as usize)
                        .enumerate()
                        .map(|(i, chunk)| Variable {
                            name: Some(Cow::Owned(format!("{}", bounds.0 + i as i64))),
                            r#type: arr.element_type.as_ref().map(|et| *et.clone()),
                            value: Some(bytes.slice_ref(chunk)),
                        })
                        .map(|var| var.render(debugger))
                        .collect::<Vec<_>>();
                    RenderItem {
                        name: var.name_cloned(),
                        r#type: var
                            .r#type
                            .as_ref()
                            .and_then(|ty| ty.name())
                            .unwrap_or_else(|| "unknown".to_string()),
                        value: None,
                        children,
                    }
                }
                Some(TypeDeclaration::CStyleEnum {
                    name,
                    discr_type,
                    enumerators,
                    ..
                }) => {
                    let discr = Variable {
                        name: None,
                        r#type: discr_type.clone().map(|t| *t),
                        value: var.value.clone(),
                    };
                    let value = discr.as_discriminator();

                    RenderItem {
                        name: var.name_cloned(),
                        r#type: name.as_deref().unwrap_or("unknown").to_string(),
                        value: value.and_then(|val| enumerators.get(&(val as i64)).cloned()),
                        children: vec![],
                    }
                }
                Some(TypeDeclaration::RustEnum {
                    name,
                    discr_type: discr_member,
                    enumerators,
                    ..
                }) => {
                    let value = discr_member.as_ref().and_then(|member| {
                        let discr_as_var = var.split_by_member(member, debugger.pid);
                        discr_as_var.as_discriminator()
                    });

                    let enumerator = value
                        .and_then(|v| enumerators.get(&Some(v)).or_else(|| enumerators.get(&None)));

                    let enumerator = enumerator.map(|member| {
                        let member_as_var = var.split_by_member(member, debugger.pid);
                        make_render_item(&member_as_var, debugger)
                    });

                    RenderItem {
                        name: var.name_cloned(),
                        r#type: name.as_deref().unwrap_or("unknown").to_string(),
                        value: None,
                        children: enumerator.map(|item| vec![item]).unwrap_or_default(),
                    }
                }
                Some(TypeDeclaration::Pointer { name, target_type }) => {
                    let mb_ptr = var
                        .value
                        .as_ref()
                        .map(scalar_from_bytes::<*const ()>)
                        .copied();

                    let deref_var = mb_ptr.map(|ptr| {
                        let read_size = target_type
                            .as_ref()
                            .and_then(|t| t.size_in_bytes(debugger.pid));

                        let val = read_size
                            .and_then(|sz| debugger.read_memory(ptr as usize, sz as usize).ok());

                        Variable {
                            name: Some(Cow::from("__0")),
                            value: val.map(Bytes::from),
                            r#type: target_type.clone().map(|t| *t),
                        }
                    });
                    let target_item = deref_var.map(|var| make_render_item(&var, debugger));

                    RenderItem {
                        name: var.name_cloned(),
                        r#type: name.as_deref().unwrap_or("unknown").to_string(),
                        value: mb_ptr.map(|ptr| format!("{:#016x}", ptr as usize)),
                        children: target_item.map(|item| vec![item]).unwrap_or_default(),
                    }
                }
                _ => {
                    unreachable!()
                }
            }
        }

        make_render_item(self, debugger)
    }

    fn as_discriminator(&self) -> Option<i64> {
        if let Some(TypeDeclaration::Scalar { name, .. }) = self.r#type.as_ref() {
            match name.as_deref() {
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
            }
        } else {
            None
        }
    }
}

fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> &T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { &*ptr.cast() }
}
