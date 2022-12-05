use crate::debugger::variable::RenderView;

pub fn render_variable(view: &RenderView) -> String {
    if view.children.is_empty() {
        format!("{}({})", view.r#type, view.value.as_ref().unwrap())
    } else {
        let mut str_view = format!("{} {{", view.r#type);
        for v in &view.children {
            str_view = format!("{str_view}\n");
            str_view = format!("{str_view}{}: {}", v.name, render_variable(v));
        }
        format!("{str_view}\n}}")
    }
}
