use crate::debugger::TypeDeclaration;
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

    pub fn render(&self, pid: Pid) -> RenderView {
        fn render_scalar<T: Copy + Display>(var: &Variable) -> (String, String) {
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

            (type_view, value_view)
        }

        fn make_render_item(var: &Variable, pid: Pid) -> RenderItem {
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
                        _ => ("unknown".to_string(), "unknown".to_string()),
                    };
                    RenderItem {
                        name: var.name_cloned(),
                        r#type: type_view,
                        value: Some(value_view),
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
                        let member_val = match var.value.as_ref() {
                            None => None,
                            Some(var_value) => member.value(var_value.as_ptr() as usize, pid),
                        };

                        let member_as_var = Variable {
                            name: member.name.as_ref().map(|n| Cow::Borrowed(n.as_str())),
                            r#type: member.r#type.clone(),
                            value: member_val,
                        };

                        item.children.push(make_render_item(&member_as_var, pid));
                    }

                    item
                }
                Some(TypeDeclaration::Array(arr)) => {
                    let bounds = arr.bounds(pid).unwrap();
                    let el_count = bounds.1 - bounds.0;
                    let el_size = arr.size_in_bytes(pid).unwrap() / el_count as u64;
                    let bytes = var.value.as_ref().unwrap();
                    let children = bytes
                        .chunks(el_size as usize)
                        .enumerate()
                        .map(|(i, chunk)| Variable {
                            name: Some(Cow::Owned(format!("{}", bounds.0 + i as i64))),
                            r#type: arr.element_type.as_ref().map(|et| *et.clone()),
                            value: Some(bytes.slice_ref(chunk)),
                        })
                        .map(|var| var.render(pid))
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
                _ => {
                    unreachable!()
                }
            }
        }

        make_render_item(self, pid)
    }
}

fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> &T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { &*ptr.cast() }
}
