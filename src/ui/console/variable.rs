use crate::debugger::address::RelocatedAddress;
use crate::debugger::variable::execute::{QueryResult, QueryResultKind};
use crate::debugger::variable::render::{RenderValue, ValueLayout};
use crate::debugger::variable::value::Value;
use crate::ui::syntax;
use crate::ui::syntax::StylizedLine;
use syntect::util::as_24_bit_terminal_escaped;

const TAB: &str = "    ";

pub fn render_variable(var: &QueryResult) -> anyhow::Result<String> {
    let syntax_renderer = syntax::rust_syntax_renderer();
    let mut line_renderer = syntax_renderer.line_renderer();
    let prefix = if var.kind() == QueryResultKind::Root && var.identity().name.is_some() {
        format!("{} = ", var.identity())
    } else {
        String::default()
    };

    let var_as_string = format!("{prefix}{}", render_value(var.value()));
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

pub fn render_value(value: &Value) -> String {
    render_value_inner(value, 0, true)
}

fn render_value_inner(value: &Value, depth: usize, print_type: bool) -> String {
    match value.value_layout() {
        Some(layout) => match layout {
            ValueLayout::PreRendered(rendered_value) => match value {
                Value::CEnum(_) => format!("{}::{}", value.r#type().name_fmt(), rendered_value),
                _ if print_type => format!("{}({})", value.r#type().name_fmt(), rendered_value),
                _ => format!("{}", rendered_value),
            },
            ValueLayout::Referential(addr) => {
                if print_type {
                    format!(
                        "{} [{}]",
                        value.r#type().name_fmt(),
                        RelocatedAddress::from(addr as usize)
                    )
                } else {
                    format!("{}", RelocatedAddress::from(addr as usize))
                }
            }
            ValueLayout::Wrapped(val) => {
                format!(
                    "{}::{}",
                    value.r#type().name_fmt(),
                    render_value_inner(val, depth, true)
                )
            }
            #[allow(clippy::useless_format)]
            ValueLayout::Structure(members) => {
                let mut render = if print_type {
                    format!("{} {{", value.r#type().name_fmt())
                } else {
                    format!("{{")
                };

                let tabs = TAB.repeat(depth + 1);

                for member in members {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        member.field_name.as_deref().unwrap_or_default(),
                        render_value_inner(&member.value, depth + 1, true)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
            ValueLayout::Map(kv_children) => {
                let mut render = format!("{} {{", value.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                let mut last_seen_kv_types = None;
                let mut show_kv_type = false;
                for (key, val) in kv_children {
                    if last_seen_kv_types != Some((key.r#type(), val.r#type())) {
                        last_seen_kv_types = Some((key.r#type(), val.r#type()));
                        show_kv_type = true;
                    }

                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        render_value_inner(key, depth + 1, show_kv_type),
                        render_value_inner(val, depth + 1, show_kv_type)
                    );
                    show_kv_type = false;
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
            ValueLayout::IndexedList(items) => {
                let mut render = format!("{} {{", value.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for item in items {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}: {}",
                        item.index,
                        render_value_inner(&item.value, depth + 1, false)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
            ValueLayout::NonIndexedList(values) => {
                let mut render = format!("{} {{", value.r#type().name_fmt());

                let tabs = TAB.repeat(depth + 1);

                for val in values {
                    render = format!("{render}\n");
                    render = format!(
                        "{render}{tabs}{}",
                        render_value_inner(val, depth + 1, false)
                    );
                }

                format!("{render}\n{}}}", TAB.repeat(depth))
            }
        },
        None => format!("{}(unknown)", value.r#type().name_fmt()),
    }
}
