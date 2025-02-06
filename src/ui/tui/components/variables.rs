use crate::debugger::register::debug::BreakCondition;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::execute::{QueryResult, QueryResultKind};
use crate::debugger::variable::render::{RenderValue, ValueLayout};
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
use tui_realm_treeview::{Node, Tree, TreeView, TREE_CMD_CLOSE, TREE_CMD_OPEN, TREE_INITIAL_NODE};
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::props::{BorderType, Borders, TextSpan};
use tuirealm::ratatui::layout::Alignment;
use tuirealm::ratatui::style::{Color, Style};
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

const MAX_RECURSION: u32 = 15;

#[derive(MockComponent)]
pub struct Variables {
    component: TreeView<Vec<TextSpan>>,
    exchanger: Arc<ClientExchanger>,
}

fn render_var_inner(
    name: Option<&str>,
    typ: Option<&str>,
    value: Option<&str>,
) -> anyhow::Result<Vec<TextSpan>> {
    let syntax_renderer = syntax::rust_syntax_renderer();
    let mut line_renderer = syntax_renderer.line_renderer();

    let mut line = String::default();
    if let Some(n) = name {
        line += n;
    }
    if let Some(t) = typ {
        line += " ";
        line += t;
    }
    if let Some(v) = value {
        line += " ";
        if typ.is_some() {
            line += &format!("({v})");
        } else {
            line += v;
        }
    }

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

fn render_var(name: Option<&str>, typ: Option<&str>, value: &str) -> anyhow::Result<Vec<TextSpan>> {
    render_var_inner(name, typ, Some(value))
}

fn render_var_def(name: Option<&str>, typ: Option<&str>) -> anyhow::Result<Vec<TextSpan>> {
    render_var_inner(name, typ, None)
}

fn node_from_var(
    recursion: u32,
    node_name: &str,
    name: Option<&str>,
    qr: QueryResult,
    print_type: bool,
) -> Node<Vec<TextSpan>> {
    let ty = if print_type {
        Some(qr.value().r#type().name_fmt())
    } else {
        None
    };

    // recursion guard
    if recursion >= MAX_RECURSION {
        return Node::new(
            node_name.to_string(),
            render_var(None, ty, "...").expect("should be rendered"),
        );
    }

    match qr.value().value_layout() {
        None => Node::new(
            node_name.to_string(),
            render_var(None, ty, "???").expect("should be rendered"),
        ),
        Some(layout) => match layout {
            ValueLayout::PreRendered(val) => Node::new(
                node_name.to_string(),
                render_var(name, ty, &val).expect("should be rendered"),
            ),
            ValueLayout::Referential(addr) => {
                let value = format!("{addr:p}");
                let mut node = Node::new(
                    node_name.to_string(),
                    render_var(name, ty, &value).expect("should be rendered"),
                );

                let qr = qr.modify_value(|ctx, val| val.deref(ctx));

                if let Some(qr) = qr {
                    let deref_node = node_from_var(
                        recursion + 1,
                        format!("{node_name}_deref").as_str(),
                        Some("*"),
                        qr,
                        true,
                    );
                    node.add_child(deref_node);
                }

                node
            }
            ValueLayout::Wrapped(inner) => {
                let mut node = Node::new(
                    node_name.to_string(),
                    render_var_def(name, ty).expect("should be rendered"),
                );
                let qr = qr
                    .clone()
                    .modify_value(|_, _| Some(inner.clone()))
                    .expect("should be `Some`");

                node.add_child(node_from_var(
                    recursion + 1,
                    format!("{node_name}_1").as_str(),
                    None,
                    qr,
                    true,
                ));

                node
            }
            ValueLayout::Structure(members) => {
                let mut node = Node::new(
                    node_name.to_string(),
                    render_var_def(name, ty).expect("should be rendered"),
                );
                for (i, member) in members.iter().enumerate() {
                    let member_var = qr
                        .clone()
                        .modify_value(|_, _| Some(member.value.clone()))
                        .expect("should be `Some`");

                    node.add_child(node_from_var(
                        recursion + 1,
                        format!("{node_name}_{i}").as_str(),
                        member.field_name.as_deref(),
                        member_var,
                        true,
                    ));
                }
                node
            }
            ValueLayout::IndexedList(items) => {
                let mut node = Node::new(
                    node_name.to_string(),
                    render_var_def(name, ty).expect("should be rendered"),
                );
                for (i, item) in items.iter().enumerate() {
                    let item_var = qr
                        .clone()
                        .modify_value(|_, _| Some(item.value.clone()))
                        .expect("should be `Some`");

                    node.add_child(node_from_var(
                        recursion + 1,
                        format!("{node_name}_{i}").as_str(),
                        Some(&format!("{}", item.index)),
                        item_var,
                        false,
                    ));
                }
                node
            }
            ValueLayout::NonIndexedList(items) => {
                let mut node = Node::new(
                    node_name.to_string(),
                    render_var_def(name, ty).expect("should be rendered"),
                );
                for (i, value) in items.iter().enumerate() {
                    let item_var = qr
                        .clone()
                        .modify_value(|_, _| Some(value.clone()))
                        .expect("should be `Some`");

                    node.add_child(node_from_var(
                        recursion + 1,
                        format!("{node_name}_{i}").as_str(),
                        None,
                        item_var,
                        false,
                    ));
                }
                node
            }
            ValueLayout::Map(kvs) => {
                let mut node = Node::new(
                    node_name.to_string(),
                    render_var_def(name, ty).expect("should be rendered"),
                );
                for (i, (key, _val)) in kvs.iter().enumerate() {
                    let mut kv_pair = Node::new(
                        format!("{node_name}_kv_{i}"),
                        vec![TextSpan::new(format!("kv {i}"))],
                    );

                    let key_var = qr
                        .clone()
                        .modify_value(|_, _| Some(key.clone()))
                        .expect("should be `Some`");
                    let key_literal = key_var.value().as_literal();

                    kv_pair.add_child(node_from_var(
                        recursion + 1,
                        format!("{node_name}_kv_{i}_key").as_str(),
                        Some("key"),
                        key_var,
                        true,
                    ));

                    if let Some(ref key_literal) = key_literal {
                        let value_var = qr.clone();
                        let value_var = value_var.modify_value(|_, value| value.index(key_literal));

                        if let Some(value_var) = value_var {
                            kv_pair.add_child(node_from_var(
                                recursion + 1,
                                format!("{node_name}_kv_{i}_val").as_str(),
                                Some("value"),
                                value_var,
                                true,
                            ));
                        }
                    }
                    node.add_child(kv_pair);
                }
                node
            }
        },
    }
}

impl Variables {
    fn update(&mut self) {
        let Ok(vars_node) = self.exchanger.request_sync(|dbg| {
            let expr = Dqe::Variable(Selector::Any);
            let handler = command::variables::Handler::new(dbg);
            let vars = handler.handle(expr).unwrap_or_default();

            let mut vars_node =
                Node::new("variables".to_string(), vec![TextSpan::new("variables")]);

            for (i, var) in vars.into_iter().enumerate() {
                let node_name = format!("var_{i}");
                let name = if var.kind() == QueryResultKind::Root && var.identity().name.is_some() {
                    Some(var.identity().to_string())
                } else {
                    None
                };

                let var_node = node_from_var(0, &node_name, name.as_deref(), var, true);
                vars_node.add_child(var_node);
            }

            vars_node
        }) else {
            return;
        };

        let Ok(args_node) = self.exchanger.request_sync(|dbg| {
            let expr = Dqe::Variable(Selector::Any);
            let handler = command::arguments::Handler::new(dbg);
            let args = handler.handle(expr).unwrap_or_default();

            let mut args_node =
                Node::new("arguments".to_string(), vec![TextSpan::new("arguments")]);
            for (i, arg) in args.into_iter().enumerate() {
                let node_name = format!("arg_{i}");
                let name = if arg.kind() == QueryResultKind::Root && arg.identity().name.is_some() {
                    Some(arg.identity().to_string())
                } else {
                    None
                };

                let var_node = node_from_var(0, &node_name, name.as_deref(), arg, true);
                args_node.add_child(var_node);
            }
            args_node
        }) else {
            return;
        };

        let mut root = Node::new(
            "root".to_string(),
            vec![TextSpan::new("arguments and variables")],
        );

        let vars_count = vars_node.children().len();
        let args_count = args_node.children().len();

        root.add_child(args_node);
        root.add_child(vars_node);

        self.component.set_tree(Tree::new(root));
        if vars_count != 0 {
            self.component.attr(
                Attribute::Custom(TREE_INITIAL_NODE),
                AttrValue::String("var_0".to_string()),
            );
        }
        if args_count != 0 {
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
                .preserve_state(false)
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
