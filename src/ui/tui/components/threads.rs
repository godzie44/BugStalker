use crate::debugger::register::debug::BreakCondition;
use crate::ui::command;
use crate::ui::command::thread::ExecutionResult as ThreadResult;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{Id, Msg};
use nix::sys::signal::Signal;
use std::sync::Arc;
use tui_realm_treeview::{Node, Tree, TreeView, TREE_CMD_CLOSE, TREE_CMD_OPEN, TREE_INITIAL_NODE};
use tuirealm::command::{Cmd, Direction, Position};
use tuirealm::event::{Key, KeyEvent};
use tuirealm::props::{BorderType, Borders, Style};
use tuirealm::tui::layout::Alignment;
use tuirealm::tui::style::Color;
use tuirealm::{
    AttrValue, Attribute, Component, Event, MockComponent, Sub, SubClause, SubEventClause,
};

#[derive(MockComponent)]
pub struct Threads {
    component: TreeView,
    exchanger: Arc<ClientExchanger>,
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

        let mut root = Node::new("root".to_string(), "threads".to_string());
        for (i, thread_snap) in threads.iter().enumerate() {
            let pid = thread_snap.thread.pid;
            let func_name = thread_snap
                .bt
                .as_ref()
                .and_then(|bt| bt[0].func_name.clone())
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

            let mut thread_node = Node::new(format!("thread_{i}"), value);

            if let Some(ref bt) = thread_snap.bt {
                for (frame_num, frame) in bt.iter().enumerate() {
                    let fn_ip_or_zero = frame.fn_start_ip.unwrap_or_default();

                    let frame_info = format!(
                        "#{frame_num} {} ({} + {:#X})",
                        frame.func_name.as_deref().unwrap_or("???"),
                        frame
                            .fn_start_ip
                            .map(|addr| addr.to_string())
                            .unwrap_or("???".to_string()),
                        frame.ip.as_u64().saturating_sub(fn_ip_or_zero.as_u64()),
                    );
                    thread_node.add_child(Node::new(
                        format!("thread_{i}_frame_{frame_num}"),
                        frame_info,
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
            Event::Keyboard(KeyEvent {
                code: Key::Left, ..
            }) => {
                self.perform(Cmd::Custom(TREE_CMD_CLOSE));
            }
            Event::Keyboard(KeyEvent {
                code: Key::Right, ..
            }) => {
                self.perform(Cmd::Custom(TREE_CMD_OPEN));
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageDown,
                ..
            }) => {
                self.perform(Cmd::Scroll(Direction::Down));
            }
            Event::Keyboard(KeyEvent {
                code: Key::PageUp, ..
            }) => {
                self.perform(Cmd::Scroll(Direction::Up));
            }
            Event::Keyboard(KeyEvent {
                code: Key::Down, ..
            }) => {
                self.perform(Cmd::Move(Direction::Down));
            }
            Event::Keyboard(KeyEvent { code: Key::Up, .. }) => {
                self.perform(Cmd::Move(Direction::Up));
            }
            Event::Keyboard(KeyEvent {
                code: Key::Home, ..
            }) => {
                self.perform(Cmd::GoTo(Position::Begin));
            }
            Event::Keyboard(KeyEvent { code: Key::End, .. }) => {
                self.perform(Cmd::GoTo(Position::End));
            }
            Event::Keyboard(KeyEvent {
                code: Key::Enter, ..
            }) => {
                self.perform(Cmd::Submit);
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
