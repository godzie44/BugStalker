use crate::debugger::address::RelocatedAddress;
use crate::debugger::variable::render::{RenderRepr, ValueLayout};
use crate::debugger::variable::VariableIR;
use crate::ui::syntax;
use crate::ui::syntax::StylizedLine;
use syntect::util::as_24_bit_terminal_escaped;

const TAB: &str = "\t";

pub fn render_variable(var: &VariableIR) -> anyhow::Result<String> {
    let syntax_renderer = syntax::rust_syntax_renderer();
    let mut line_renderer = syntax_renderer.line_renderer();
    let var_as_string = format!("{} = {}", var.name(), render_variable_ir(var, 0));
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
            ValueLayout::Referential { addr } => {
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
            ValueLayout::Structure { members } => {
                let mut render = format!("{} {{", view.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for v in members {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        v.name(),
                        render_variable_ir(v, depth + 1)
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
            ValueLayout::List { members, indexed } => {
                let mut render = format!("{} {{", view.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for v in members {
                    render = format!("{render}\n");
                    if indexed {
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
        },
        None => format!("{}(unknown)", view.r#type().name_fmt()),
    }
}
