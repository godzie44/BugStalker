use crate::debugger::variable::RenderView;

pub fn render_variable(view: &RenderView, depth: usize) -> String {
    const TAB: &str = "\t";

    if view.children.is_empty() {
        format!(
            "{}{}",
            view.r#type,
            view.value
                .as_deref()
                .map(|val| format!("({val})"))
                .unwrap_or_default()
        )
    } else {
        let mut str_view = format!("{} {{", view.r#type);
        let tabs = TAB.repeat(depth + 1);

        for v in &view.children {
            str_view = format!("{str_view}\n");
            str_view = format!(
                "{str_view}{tabs}{}: {}",
                v.name,
                render_variable(v, depth + 1)
            );
        }

        format!("{str_view}\n{}}}", TAB.repeat(depth))
    }
}
