use crate::debugger::register::debug::BreakCondition;
use crate::debugger::variable::render::{RenderRepr, ValueLayout};
use crate::debugger::variable::select::{DQE, Literal, VariableSelector};
use crate::debugger::variable::{VariableIR, select};
use crate::ui;
use crate::ui::syntax::StylizedLine;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::utils::syntect::into_text_span;
use crate::ui::tui::{Id, Msg};
use crate::ui::{command, syntax};
use nix::sys::signal::Signal;
use std::sync::Arc;
use tui_realm_treeview::{Node, TREE_CMD_CLOSE, TREE_CMD_OPEN, TREE_INITIAL_NODE, Tree, TreeView};
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::props::{BorderType, Borders, TextSpan};
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::style::{Color, Style};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

const MAX_RECURSION: u32 = 15;

#[derive(MockComponent)]
pub struct Variables {
    component: TreeView<Vec<TextSpan>>,
    exchanger: Arc<ClientExchanger>,
}

fn render_var_inner(name: &str, typ: &str, value: Option<&str>) -> anyhow::Result<Vec<TextSpan>> {
    let syntax_renderer = syntax::rust_syntax_renderer();
    let mut line_renderer = syntax_renderer.line_renderer();

    let line = match value {
        None => {
            format!("{name} {typ}")
        }
        Some(value) => {
            format!("{name} {typ}({value})")
        }
    };

    let line_spans = match line_renderer.render_line(&line)? {
        StylizedLine::NoneStyle(l) => {
            vec![TextSpan::new(l)]
        }
        StylizedLine::Stylized(segment) => segment
            .into_iter()
            .filter_map(|s| into_text_span(s).ok())
            .collect(),
    };
    Ok(line_spans)
}

fn render_var(name: &str, typ: &str, value: &str) -> anyhow::Result<Vec<TextSpan>> {
    render_var_inner(name, typ, Some(value))
}

fn render_var_def(name: &str, typ: &str) -> anyhow::Result<Vec<TextSpan>> {
    render_var_inner(name, typ, None)
}

impl Variables {
    fn node_from_var(
        &self,
        recursion: u32,
        node_name: &str,
        var: &VariableIR,
        select_path: Option<DQE>,
    ) -> Node<Vec<TextSpan>> {
        let name = var.name();
        let typ = var.r#type();

        // recursion guard
        if recursion >= MAX_RECURSION {
            return Node::new(
                node_name.to_string(),
                render_var(&name, typ, "...").expect("should be rendered"),
            );
        }

        match var.value() {
            None => Node::new(
                node_name.to_string(),
                render_var(&name, typ, "unknown").expect("should be rendered"),
            ),
            Some(layout) => match layout {
                ValueLayout::PreRendered(val) => {
                    let s = val.as_ref();
                    if let Err(e) = String::from_utf8(s.as_bytes().to_vec()) {
                        println!("err: {e}");
                    }

                    Node::new(
                        node_name.to_string(),
                        render_var(&name, typ, &val).expect("should be rendered"),
                    )
                }
                ValueLayout::Referential { addr, .. } => {
                    let value = format!("{addr:p}");
                    let mut node = Node::new(
                        node_name.to_string(),
                        render_var(&name, typ, &value).expect("should be rendered"),
                    );

                    if let Some(path) = select_path {
                        let deref_expr = DQE::Deref(Box::new(path));

                        let variables = {
                            let deref_expr = deref_expr.clone();
                            self.exchanger
                                .request_sync(|dbg| {
                                    let handler = command::variables::Handler::new(dbg);
                                    handler.handle(deref_expr)
                                })
                                .expect("messaging enabled")
                        };

                        if let Ok(variables) = variables {
                            if let Some(var) = variables.first() {
                                let deref_node = self.node_from_var(
                                    recursion + 1,
                                    format!("{node_name}_deref").as_str(),
                                    var,
                                    Some(deref_expr),
                                );
                                node.add_child(deref_node);
                            }
                        }
                    }

                    node
                }
                ValueLayout::Wrapped(other) => {
                    let mut node = Node::new(
                        node_name.to_string(),
                        render_var_def(&name, typ).expect("should be rendered"),
                    );
                    node.add_child(self.node_from_var(
                        recursion + 1,
                        format!("{node_name}_1").as_str(),
                        other,
                        select_path,
                    ));
                    node
                }
                ValueLayout::Structure { members, .. } => {
                    let mut node = Node::new(
                        node_name.to_string(),
                        render_var_def(&name, typ).expect("should be rendered"),
                    );
                    for (i, member) in members.iter().enumerate() {
                        node.add_child(
                            self.node_from_var(
                                recursion + 1,
                                format!("{node_name}_{i}").as_str(),
                                member,
                                select_path
                                    .clone()
                                    .map(|expr| DQE::Field(Box::new(expr), member.name())),
                            ),
                        );
                    }
                    node
                }
                ValueLayout::Map(kvs) => {
                    let mut node = Node::new(
                        node_name.to_string(),
                        render_var_def(&name, typ).expect("should be rendered"),
                    );
                    for (i, (key, val)) in kvs.iter().enumerate() {
                        let mut kv_pair = Node::new(
                            format!("{node_name}_kv_{i}"),
                            vec![TextSpan::new(format!("kv {i}"))],
                        );

                        kv_pair.add_child(self.node_from_var(
                            recursion + 1,
                            format!("{node_name}_kv_{i}_key").as_str(),
                            key,
                            // currently no way to use expressions with keys
                            None,
                        ));

                        kv_pair.add_child(
                            self.node_from_var(
                                recursion + 1,
                                format!("{node_name}_kv_{i}_val").as_str(),
                                val,
                                // todo works only if key is a String or &str, need better support of field expr on maps
                                select_path
                                    .clone()
                                    .map(|expr| DQE::Field(Box::new(expr), key.name())),
                            ),
                        );
                        node.add_child(kv_pair);
                    }
                    node
                }
                ValueLayout::List { members, indexed } => {
                    let mut node = Node::new(
                        node_name.to_string(),
                        render_var_def(&name, typ).expect("should be rendered"),
                    );
                    for (i, member) in members.iter().enumerate() {
                        let el_path = if indexed {
                            select_path.clone().and_then(|expr| {
                                let mb_idx: Option<u64> = member.name().parse().ok();
                                mb_idx
                                    .map(|idx| DQE::Index(Box::new(expr), Literal::Int(idx as i64)))
                            })
                        } else {
                            None
                        };

                        node.add_child(self.node_from_var(
                            recursion + 1,
                            format!("{node_name}_{i}").as_str(),
                            member,
                            el_path,
                        ));
                    }
                    node
                }
            },
        }
    }

    fn update(&mut self) {
        let Ok(variables) = self.exchanger.request_sync(|dbg| {
            let expr = select::DQE::Variable(VariableSelector::Any);
            let vars = command::variables::Handler::new(dbg)
                .handle(expr)
                .unwrap_or_default();
            vars
        }) else {
            return;
        };
        let Ok(arguments) = self.exchanger.request_sync(|dbg| {
            let expr = select::DQE::Variable(VariableSelector::Any);
            let args = command::arguments::Handler::new(dbg)
                .handle(expr)
                .unwrap_or_default();
            args
        }) else {
            return;
        };

        let mut root = Node::new(
            "root".to_string(),
            vec![TextSpan::new("arguments and variables")],
        );

        let mut args_node = Node::new("arguments".to_string(), vec![TextSpan::new("arguments")]);
        for (i, arg) in arguments.iter().enumerate() {
            let node_name = format!("arg_{i}");
            let var_node = self.node_from_var(
                0,
                node_name.as_str(),
                arg,
                Some(DQE::Variable(VariableSelector::Name {
                    var_name: arg.name(),
                    only_local: false,
                })),
            );
            args_node.add_child(var_node);
        }
        root.add_child(args_node);

        let mut vars_node = Node::new("variables".to_string(), vec![TextSpan::new("variables")]);
        for (i, var) in variables.iter().enumerate() {
            let node_name = format!("var_{i}");
            let var_node = self.node_from_var(
                0,
                node_name.as_str(),
                var,
                Some(DQE::Variable(VariableSelector::Name {
                    var_name: var.name(),
                    only_local: true,
                })),
            );
            vars_node.add_child(var_node);
        }
        root.add_child(vars_node);

        self.component.set_tree(Tree::new(root));
        if !variables.is_empty() {
            self.component.attr(
                Attribute::Custom(TREE_INITIAL_NODE),
                AttrValue::String("var_0".to_string()),
            );
        }
        if !arguments.is_empty() {
            self.component.attr(
                Attribute::Custom(TREE_INITIAL_NODE),
                AttrValue::String("arg_0".to_string()),
            );
        }
    }

    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![
            Sub::new(
                // concrete signal doesn't meter
                SubEventClause::User(UserEvent::Signal(Signal::SIGUSR2)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::User(UserEvent::Breakpoint {
                    pc: Default::default(),
                    num: 0,
                    file: None,
                    line: None,
                    function: None,
                }),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::User(UserEvent::Watchpoint {
                    pc: Default::default(),
                    num: 0,
                    file: None,
                    line: None,
                    cond: BreakCondition::DataReadsWrites,
                    old_value: None,
                    new_value: None,
                    end_of_scope: false,
                }),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::User(UserEvent::Step {
                    pc: Default::default(),
                    file: None,
                    line: None,
                    function: None,
                }),
                SubClause::Always,
            ),
            // concrete code doesn't meter
            Sub::new(SubEventClause::User(UserEvent::Exit(0)), SubClause::Always),
        ]
    }

    pub fn new(exchanger: Arc<ClientExchanger>) -> Self {
        let mut this = Self {
            component: TreeView::default()
                .borders(
                    Borders::default()
                        .color(Color::LightYellow)
                        .modifiers(BorderType::Rounded),
                )
                .inactive(Style::default().fg(Color::Gray))
                .indent_size(3)
                .scroll_step(6)
                .preserve_state(true)
                .title("Variables", Alignment::Center)
                .highlighted_color(Color::LightYellow)
                .highlight_symbol("▶"),
            exchanger,
        };
        this.update();
        this
    }
}

impl Component<Msg, UserEvent> for Variables {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Keyboard(key_event) => {
                let keymap = &ui::config::current().tui_keymap;
                if let Some(action) = keymap.get_common(&key_event) {
                    match action {
                        CommonAction::Left => {
                            self.perform(Cmd::Custom(TREE_CMD_CLOSE));
                        }
                        CommonAction::Right => {
                            self.perform(Cmd::Custom(TREE_CMD_OPEN));
                        }
                        CommonAction::Up => {
                            self.perform(Cmd::Move(Direction::Up));
                        }
                        CommonAction::Down => {
                            self.perform(Cmd::Move(Direction::Down));
                        }
                        CommonAction::ScrollUp => {
                            self.perform(Cmd::Scroll(Direction::Up));
                        }
                        CommonAction::ScrollDown => {
                            self.perform(Cmd::Scroll(Direction::Down));
                        }
                        CommonAction::GotoBegin => {
                            self.perform(Cmd::GoTo(Position::Begin));
                        }
                        CommonAction::GotoEnd => {
                            self.perform(Cmd::GoTo(Position::End));
                        }
                        CommonAction::Submit => {
                            self.perform(Cmd::Submit);
                        }
                        _ => {}
                    }
                }
            }

            Event::User(UserEvent::Breakpoint { .. })
            | Event::User(UserEvent::Watchpoint { .. })
            | Event::User(UserEvent::Exit(_))
            | Event::User(UserEvent::Step { .. }) => {
                self.exchanger.enable_messaging();
                self.update();
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
