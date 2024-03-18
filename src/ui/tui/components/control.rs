use crate::debugger::Error;
use crate::ui::command;
use crate::ui::command::{run, CommandError};
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{Id, Msg};
use log::warn;
use nix::sys::signal;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use std::sync::Arc;
use tuirealm::event::{Key, KeyEvent, KeyModifiers};
use tuirealm::{Component, Event, MockComponent, Sub, SubClause, SubEventClause};

#[derive(MockComponent)]
pub struct GlobalControl {
    component: tui_realm_stdlib::Phantom,
    exchanger: Arc<ClientExchanger>,
    last_seen_pid: Pid,
}

impl GlobalControl {
    pub fn new(exchanger: Arc<ClientExchanger>, pid: Pid) -> Self {
        Self {
            component: tui_realm_stdlib::Phantom::default(),
            exchanger,
            last_seen_pid: pid,
        }
    }

    pub fn subscriptions() -> Vec<Sub<Id, UserEvent>> {
        vec![
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('1'), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('2'), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('c'), KeyModifiers::ALT)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Esc, KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('q'), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('c'), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('c'), KeyModifiers::CONTROL)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('r'), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Function(6), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Function(7), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Function(8), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Function(9), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Function(10), KeyModifiers::NONE)),
                SubClause::Always,
            ),
            Sub::new(
                // concrete signal doesn't meter
                SubEventClause::User(UserEvent::Signal(Signal::SIGUSR2)),
                SubClause::Always,
            ),
            Sub::new(
                // concrete error doesn't meter
                SubEventClause::User(UserEvent::AsyncErrorResponse(String::default())),
                SubClause::Always,
            ),
            Sub::new(
                // concrete pid doesn't meter
                SubEventClause::User(UserEvent::ProcessInstall(Pid::from_raw(0))),
                SubClause::Always,
            ),
            Sub::new(
                // concrete brkpt doesn't meter
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
                // concrete step doesn't meter
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
}

impl Component<Msg, UserEvent> for GlobalControl {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        let msg = match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Char('1'),
                modifiers: KeyModifiers::NONE,
            }) => Msg::LeftTabsInFocus,
            Event::Keyboard(KeyEvent {
                code: Key::Char('2'),
                modifiers: KeyModifiers::NONE,
            }) => Msg::RightTabsInFocus,
            Event::Keyboard(KeyEvent {
                code: Key::Esc,
                modifiers: KeyModifiers::NONE,
            }) => Msg::SwitchUI,
            Event::Keyboard(KeyEvent {
                code: Key::Char('q'),
                modifiers: KeyModifiers::NONE,
            }) => Msg::AppClose,
            Event::Keyboard(KeyEvent {
                code: Key::Char('c'),
                modifiers: KeyModifiers::CONTROL,
            }) => {
                _ = signal::kill(self.last_seen_pid, Signal::SIGINT);
                Msg::None
            }
            Event::Keyboard(KeyEvent {
                code: Key::Char('c'),
                modifiers: KeyModifiers::NONE,
            })
            | Event::Keyboard(KeyEvent {
                code: Key::Function(9),
                ..
            }) => {
                if !self.exchanger.is_messaging_enabled() {
                    warn!(target: "tui", "try start/restart but messaging disabled");
                    return None;
                }

                self.exchanger
                    .request_async(|dbg| Ok(command::r#continue::Handler::new(dbg).handle()?))
                    .expect("messaging enabled");

                self.exchanger.disable_messaging();
                Msg::AppRunning
            }
            Event::Keyboard(KeyEvent {
                code: Key::Char('r'),
                ..
            })
            | Event::Keyboard(KeyEvent {
                code: Key::Function(10),
                ..
            }) => {
                if !self.exchanger.is_messaging_enabled() {
                    warn!(target: "tui", "try start/restart but messaging disabled");
                    return None;
                }

                let mb_err = self
                    .exchanger
                    .request_sync(|dbg| run::Handler::new(dbg).handle(run::Command::DryStart))
                    .expect("messaging enabled");

                let already_run =
                    matches!(mb_err.err(), Some(CommandError::Handle(Error::AlreadyRun)));

                if already_run {
                    Msg::PopupConfirmDebuggerRestart
                } else {
                    self.exchanger
                        .request_async(
                            |dbg| Ok(run::Handler::new(dbg).handle(run::Command::Start)?),
                        )
                        .expect("messaging enabled");
                    self.exchanger.disable_messaging();
                    Msg::AppRunning
                }
            }
            Event::Keyboard(KeyEvent {
                code: Key::Function(8),
                modifiers: KeyModifiers::NONE,
            }) => {
                if !self.exchanger.is_messaging_enabled() {
                    warn!(target: "tui", "try start/restart but messaging disabled");
                    return None;
                }

                self.exchanger
                    .request_async(|dbg| Ok(command::step_over::Handler::new(dbg).handle()?))
                    .expect("messaging enabled");

                Msg::AppRunning
            }
            Event::Keyboard(KeyEvent {
                code: Key::Function(7),
                modifiers: KeyModifiers::NONE,
            }) => {
                if !self.exchanger.is_messaging_enabled() {
                    warn!(target: "tui", "try start/restart but messaging disabled");
                    return None;
                }

                self.exchanger
                    .request_async(|dbg| Ok(command::step_into::Handler::new(dbg).handle()?))
                    .expect("messaging enabled");

                Msg::AppRunning
            }
            Event::Keyboard(KeyEvent {
                code: Key::Function(6),
                modifiers: KeyModifiers::NONE,
            }) => {
                if !self.exchanger.is_messaging_enabled() {
                    warn!(target: "tui", "try start/restart but messaging disabled");
                    return None;
                }

                self.exchanger
                    .request_async(|dbg| Ok(command::step_out::Handler::new(dbg).handle()?))
                    .expect("messaging enabled");

                Msg::AppRunning
            }
            Event::User(UserEvent::AsyncErrorResponse(err)) => {
                Msg::ShowOkPopup(Some("Error".to_string()), err)
            }
            Event::User(UserEvent::ProcessInstall(pid)) => {
                self.last_seen_pid = pid;
                Msg::None
            }
            Event::User(UserEvent::Signal(sig)) => {
                self.exchanger.enable_messaging();
                Msg::ShowOkPopup(
                    Some("Signal stop".to_string()),
                    format!("Application receive signal: {sig}"),
                )
            }
            Event::User(UserEvent::Breakpoint { .. })
            | Event::User(UserEvent::Exit(_))
            | Event::User(UserEvent::Step { .. }) => {
                self.exchanger.enable_messaging();
                Msg::None
            }
            _ => Msg::None,
        };
        Some(msg)
    }
}
