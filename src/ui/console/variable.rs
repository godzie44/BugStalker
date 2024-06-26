use crate::debugger::address::RelocatedAddress;
use crate::debugger::variable::VariableIR;
use crate::debugger::variable::render::{RenderRepr, ValueLayout};
use crate::ui::syntax;
use crate::ui::syntax::StylizedLine;
use syntect::util::as_24_bit_terminal_escaped;

const TAB: &str = "\t";

pub fn render_variable(var: &VariableIR) -> anyhow::Result<String> {
    let syntax_renderer = syntax::rust_syntax_renderer();
    let mut line_renderer = syntax_renderer.line_renderer();
    let prefix = var
        .name()
        .map(|name| format!("{name} = "))
        .unwrap_or_default();

    let var_as_string = format!("{prefix}{}", render_variable_ir(var, 0));
    Ok(var_as_string
        .lines()
        .map(|l| -> anyhow::Result<String> {
            let line = match line_renderer.render_line(l)? {
                StylizedLine::NoneStyle(l) => l.to_string(),
                StylizedLine::Stylized(segments) => {
                    let line = as_24_bit_terminal_escaped(&segments, false);
                    format!("{line}\x1b[0m")
                }
            };
            Ok(line)
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .join("\n"))
}

pub fn render_variable_ir(view: &VariableIR, depth: usize) -> String {
    match view.value() {
        Some(value) => match value {
            ValueLayout::PreRendered(rendered_value) => match view {
                VariableIR::CEnum(_) => format!("{}::{}", view.r#type().name_fmt(), rendered_value),
                _ => format!("{}({})", view.r#type().name_fmt(), rendered_value),
            },
            ValueLayout::Referential(addr) => {
                format!(
                    "{} [{}]",
                    view.r#type().name_fmt(),
                    RelocatedAddress::from(addr as usize)
                )
            }
            ValueLayout::Wrapped(val) => {
                format!(
                    "{}::{}",
                    view.r#type().name_fmt(),
                    render_variable_ir(val, depth)
                )
            }
            ValueLayout::Structure(members) => {
                let mut render = format!("{} {{", view.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for member in members {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        member.field_name.as_deref().unwrap_or_default(),
                        render_variable_ir(&member.value, depth + 1)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
            ValueLayout::Map(kv_children) => {
                let mut render = format!("{} {{", view.r#type().name_fmt());

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
            ValueLayout::IndexedList(items) => {
                let mut render = format!("{} {{", view.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for item in items {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        item.index,
                        render_variable_ir(&item.value, depth + 1)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
            ValueLayout::NonIndexedList(values) => {
                let mut render = format!("{} {{", view.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for val in values {
                    render = format!("{render}\n");
                    render = format!("{render}{tabs}{}", render_variable_ir(val, depth + 1));
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
        },
        None => format!("{}(unknown)", view.r#type().name_fmt()),
    }
}
