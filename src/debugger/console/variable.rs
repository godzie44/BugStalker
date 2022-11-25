use crate::debugger::dwarf::r#type::TypeDeclaration;
use crate::debugger::Variable;
use bytes::Bytes;
use nix::unistd::Pid;
use std::borrow::Cow;
use std::fmt::Display;
use std::mem;
use std::ops::AddAssign;

pub fn render_variable_value(var: &Variable, pid: Pid) -> String {
    fn render_scalar<T: Copy + Display>(var: &Variable) -> String {
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

        format!("{type_view}({value_view})")
    }

    var.r#type
        .as_ref()
        .map(|var_type| match var_type {
            TypeDeclaration::Scalar { name, .. } => match name.as_deref() {
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
                _ => format!("unknown type (raw: {:?})", var.value),
            },
            TypeDeclaration::Structure { name, members, .. } => {
                let mut struct_view = format!("{} {{", name.as_deref().unwrap_or("unknown"));
                for member in members {
                    struct_view.add_assign("\n");
                    struct_view.add_assign(member.name.as_deref().unwrap_or("unknown"));
                    struct_view.add_assign(": ");

                    let member_val = match var.value.as_ref() {
                        None => None,
                        Some(var_value) => member.value(var_value.as_ptr() as usize, pid),
                    };
                    let member_as_var = Variable {
                        name: member.name.as_ref().map(|n| Cow::Borrowed(n.as_str())),
                        r#type: member.r#type.clone(),
                        value: member_val,
                    };
                    struct_view.add_assign(&render_variable_value(&member_as_var, pid));
                }

                struct_view.add_assign("\n}");

                struct_view
            }
        })
        .unwrap_or_else(|| "unknown type".to_string())
}

fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> &T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { &*ptr.cast() }
}
