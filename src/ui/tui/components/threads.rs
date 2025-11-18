use crate::debugger::register::debug::BreakCondition;
use crate::ui;
use crate::ui::command::thread::ExecutionResult as ThreadResult;
use crate::ui::proto::ClientExchanger;
use crate::ui::short::Abbreviator;
use crate::ui::syntax::StylizedLine;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::CommonAction;
use crate::ui::tui::utils::syntect::into_text_span;
use crate::ui::tui::{Id, Msg};
use crate::ui::{command, syntax};
use nix::sys::signal::Signal;
use std::sync::Arc;
use tui_realm_treeview::{Node, TREE_CMD_CLOSE, TREE_CMD_OPEN, TREE_INITIAL_NODE, Tree, TreeView};
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::props::{BorderType, Borders, Style, TextSpan};
use tuirealm::ratatui::layout::Alignment;
use tuirealm::ratatui::style::Color;
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(MockComponent)]
pub struct Threads {
    component: TreeView<Vec<TextSpan>>,
    exchanger: Arc<ClientExchanger>,
}

fn render_frame(line: &str) -> anyhow::Result<Vec<TextSpan>> {
    let syntax_renderer = syntax::rust_syntax_renderer();
    let mut line_renderer = syntax_renderer.line_renderer();

    let line_spans = match line_renderer.render_line(line)? {
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

impl Threads {
    fn update_threads(&mut self) {
        let Ok(threads) = self.exchanger.request_sync(|dbg| {
            let thread_result = command::thread::Handler::new(dbg)
                .handle(command::thread::Command::Info)
                .unwrap_or(ThreadResult::List(vec![]));

            let ThreadResult::List(threads) = thread_result else {
                unreachable!()
            };

            threads
        }) else {
            return;
        };

        let mut root = Node::new("root".to_string(), vec![TextSpan::new("threads")]);
        for (i, thread_snap) in threads.iter().enumerate() {
            let pid = thread_snap.thread.pid;
            let func_name = thread_snap
                .bt
                .as_ref()
                .and_then(|bt| bt.first()?.func_name.clone())
                .unwrap_or("unknown".to_string());
            let line = thread_snap
                .place
                .as_ref()
                .map(|l| l.line_number.to_string())
                .unwrap_or("???".to_string());

            let value = if thread_snap.in_focus {
                format!(" (CURRENT) [{pid}] {func_name}(:{line})")
            } else {
                format!(" [{pid}] {func_name}(:{line})")
            };

            let mut thread_node = Node::new(
                format!("thread_{i}"),
                render_frame(&value).expect("should be rendered"),
            );

            let abbreviator = Abbreviator::new("/", "/..", 30);

            if let Some(ref bt) = thread_snap.bt {
                for (frame_num, frame) in bt.iter().enumerate() {
                    let file_and_line = frame
                        .place
                        .as_ref()
                        .map(|p| {
                            format!(
                                "at {}:{}",
                                abbreviator.apply(&p.file.to_string_lossy()),
                                p.line_number
                            )
                        })
                        .unwrap_or_default();

                    let frame_info = format!(
                        "#{frame_num} {} - {} {}",
                        frame.ip,
                        frame.func_name.as_deref().unwrap_or("???"),
                        file_and_line,
                    );
                    thread_node.add_child(Node::new(
                        format!("thread_{i}_frame_{frame_num}"),
                        render_frame(&frame_info).expect("should be rendered"),
                    ));
                }
            }

            root.add_child(thread_node);
        }

        self.component.set_tree(Tree::new(root));
        self.component.attr(
            Attribute::Custom(TREE_INITIAL_NODE),
            AttrValue::String("thread_0".to_string()),
        );
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
        let tree_view = TreeView::default()
            .borders(
                Borders::default()
                    .color(Color::LightYellow)
                    .modifiers(BorderType::Rounded),
            )
            .inactive(Style::default().fg(Color::Gray))
            .indent_size(3)
            .scroll_step(6)
            .preserve_state(true)
            .title("Threads", Alignment::Center)
            .highlighted_color(Color::LightYellow)
            .highlight_symbol("▶");

        let mut this = Self {
            component: tree_view,
            exchanger,
        };
        this.update_threads();
        this
    }
}

impl Component<Msg, UserEvent> for Threads {
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
                self.update_threads();
            }
            _ => {}
        };
        Some(Msg::None)
    }
}
