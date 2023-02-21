use crate::debugger::variable::render::{RenderRepr, ValueLayout};
use crate::debugger::variable::VariableIR;

const TAB: &str = "\t";

pub fn render_variable_ir(view: &VariableIR, depth: usize) -> String {
    match view.value() {
        Some(value) => match value {
            ValueLayout::PreRendered(rendered_value) => match view {
                VariableIR::CEnum(_) => format!("{}::{}", view.r#type(), rendered_value),
                _ => format!("{}({})", view.r#type(), rendered_value),
            },
            ValueLayout::Referential { addr, val } => {
                format!(
                    "{} [{addr:p}] ({})",
                    view.r#type(),
                    render_variable_ir(val, depth)
                )
            }
            ValueLayout::Wrapped(val) => {
                format!("{}::{}", view.r#type(), render_variable_ir(val, depth))
            }
            ValueLayout::Nested { members, named } => {
                let mut render = format!("{} {{", view.r#type());

                let tabs = TAB.repeat(depth + 1);

                for v in members {
                    render = format!("{render}\n");
                    if named {
                        render = format!(
                            "{render}{tabs}{}: {}",
                            v.name(),
                            render_variable_ir(v, depth + 1)
                        );
                    } else {
                        render = format!("{render}{tabs}{}", render_variable_ir(v, depth + 1));
                    }
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
            ValueLayout::Map(kv_children) => {
                let mut render = format!("{} {{", view.r#type());

                let tabs = TAB.repeat(depth + 1);

                for kv in kv_children {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        render_variable_ir(&kv.0, depth + 1),
                        render_variable_ir(&kv.1, depth + 1)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
        },
        None => format!("{}(unknown)", view.r#type()),
    }
}
