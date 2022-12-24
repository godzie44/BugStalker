use crate::debugger::variable::{IRValueRepr, VariableIR};

const TAB: &str = "\t";

pub fn render_variable_ir(view: &VariableIR, depth: usize) -> String {
    match view.value() {
        Some(value) => match value {
            IRValueRepr::Rendered(rendered_value) => match view {
                VariableIR::CEnum(_) => format!("{}::{}", view.r#type(), rendered_value),
                _ => format!("{}({})", view.r#type(), rendered_value),
            },
            IRValueRepr::Referential { addr, val } => {
                format!(
                    "{} [{addr:p}] ({})",
                    view.r#type(),
                    render_variable_ir(val, depth)
                )
            }
            IRValueRepr::Wrapped(val) => {
                format!("{}::{}", view.r#type(), render_variable_ir(val, depth))
            }
            IRValueRepr::Nested(children) => {
                let mut render = format!("{} {{", view.r#type());

                let tabs = TAB.repeat(depth + 1);

                for v in children {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        v.name().to_string(),
                        render_variable_ir(v, depth + 1)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
        },
        None => format!("{}(unknown)", view.r#type()),
    }
}
