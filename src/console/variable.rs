use crate::debugger::variable::{RenderHint, RenderView};

const TAB: &str = "\t";

pub fn render_variable(view: &RenderView, depth: usize) -> String {
    if view.children.is_empty() {
        match view.hint {
            RenderHint::Enum => {
                format!(
                    "{}::{}",
                    view.r#type,
                    view.value.as_deref().unwrap_or_default()
                )
            }
            _ => {
                format!(
                    "{}({})",
                    view.r#type,
                    view.value.as_deref().unwrap_or_default()
                )
            }
        }
    } else {
        match view.hint {
            RenderHint::Enum => {
                let mut str_view = format!("{}::", view.r#type);

                if view.children.len() == 1 {
                    format!(
                        "{str_view}{val}",
                        val = render_variable(&view.children[0], depth)
                    )
                } else {
                    let tabs = TAB.repeat(depth + 1);

                    for v in &view.children {
                        str_view = format!("{str_view}\n");
                        str_view = format!(
                            "{str_view}{tabs}{}: {}",
                            v.name(),
                            render_variable(v, depth + 1)
                        );
                    }
                    format!("{str_view}\n{}", TAB.repeat(depth))
                }
            }
            RenderHint::Pointer => {
                format!(
                    "{}({val})",
                    view.r#type,
                    val = render_variable(&view.children[0], depth)
                )
            }
            _ => {
                let mut str_view = format!("{} {{", view.r#type);
                let tabs = TAB.repeat(depth + 1);

                for v in &view.children {
                    str_view = format!("{str_view}\n");
                    str_view = format!(
                        "{str_view}{tabs}{}: {}",
                        v.name(),
                        render_variable(v, depth + 1)
                    );
                }

                format!("{str_view}\n{}}}", TAB.repeat(depth))
            }
        }
    }
}
