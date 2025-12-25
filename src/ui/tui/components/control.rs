use crate::debugger::Error;
use crate::debugger::register::debug::BreakCondition;
use crate::ui;
use crate::ui::command;
use crate::ui::command::{CommandError, run};
use crate::ui::proto::ClientExchanger;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::SpecialAction;
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
        let interested_actions = [
            SpecialAction::ExpandLeftWindow,
            SpecialAction::ExpandRightWindow,
            SpecialAction::FocusLeftWindow,
            SpecialAction::FocusRightWindow,
            SpecialAction::SwitchUI,
            SpecialAction::CloseApp,
            SpecialAction::ContinueDebugee,
            SpecialAction::RunDebugee,
            SpecialAction::StepOver,
            SpecialAction::StepInto,
            SpecialAction::StepOut,
        ];
        let mut subscriptions = vec![];

        let keymap = &ui::config::current().tui_keymap;
        for action in interested_actions {
            for key in keymap.keys_for_special_action(action) {
                subscriptions.push(Sub::new(SubEventClause::Keyboard(*key), SubClause::Always));
            }
        }

        let user_subs = vec![
            Sub::new(
                SubEventClause::Keyboard(KeyEvent::new(Key::Char('c'), KeyModifiers::CONTROL)),
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
                // concrete watchpoint doesn't meter
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
        ];

        subscriptions.extend(user_subs);
        subscriptions
    }
}

impl Component<Msg, UserEvent> for GlobalControl {
    fn on(&mut self, ev: Event<UserEvent>) -> Option<Msg> {
        let msg = match ev {
            Event::Keyboard(KeyEvent {
                code: Key::Char('c'),
                modifiers: KeyModifiers::CONTROL,
            }) => {
                _ = signal::kill(self.last_seen_pid, Signal::SIGINT);
                Msg::None
            }

            Event::Keyboard(key_event) => {
                let keymap = &ui::config::current().tui_keymap;
                if let Some(action) = keymap.get_special(&key_event) {
                    match action {
                        SpecialAction::ExpandLeftWindow => Msg::ExpandTab(Id::LeftTabs),
                        SpecialAction::ExpandRightWindow => Msg::ExpandTab(Id::RightTabs),
                        SpecialAction::FocusLeftWindow => Msg::LeftTabsInFocus { reset_to: None },
                        SpecialAction::FocusRightWindow => Msg::RightTabsInFocus { reset_to: None },
                        SpecialAction::SwitchUI => Msg::SwitchUI,
                        SpecialAction::CloseApp => Msg::AppClose,
                        SpecialAction::ContinueDebugee => {
                            if !self.exchanger.is_messaging_enabled() {
                                warn!(target: "tui", "try continue but messaging disabled");
                                return None;
                            }

                            self.exchanger
                                .request_async(|dbg| {
                                    Ok(command::r#continue::Handler::new(dbg).handle()?)
                                })
                                .expect("messaging enabled");

                            self.exchanger.disable_messaging();
                            Msg::AppRunning
                        }
                        SpecialAction::RunDebugee => {
                            if !self.exchanger.is_messaging_enabled() {
                                warn!(target: "tui", "try start/restart but messaging disabled");
                                return None;
                            }

                            let mb_err = self
                                .exchanger
                                .request_sync(|dbg| {
                                    run::Handler::new(dbg).handle(run::Command::DryStart)
                                })
                                .expect("messaging enabled");

                            let already_run = matches!(
                                mb_err.err(),
                                Some(CommandError::Handle(Error::AlreadyRun))
                            );

                            if already_run {
                                Msg::PopupConfirmDebuggerRestart
                            } else {
                                self.exchanger
                                    .request_async(|dbg| {
                                        Ok(run::Handler::new(dbg).handle(run::Command::Start)?)
                                    })
                                    .expect("messaging enabled");
                                self.exchanger.disable_messaging();
                                Msg::AppRunning
                            }
                        }
                        SpecialAction::StepOver => {
                            if !self.exchanger.is_messaging_enabled() {
                                warn!(target: "tui", "try step-over but messaging disabled");
                                return None;
                            }

                            self.exchanger
                                .request_async(|dbg| {
                                    Ok(command::step_over::Handler::new(dbg).handle()?)
                                })
                                .expect("messaging enabled");

                            Msg::AppRunning
                        }
                        SpecialAction::StepInto => {
                            if !self.exchanger.is_messaging_enabled() {
                                warn!(target: "tui", "try step-into but messaging disabled");
                                return None;
                            }

                            self.exchanger
                                .request_async(|dbg| {
                                    Ok(command::step_into::Handler::new(dbg).handle()?)
                                })
                                .expect("messaging enabled");

                            Msg::AppRunning
                        }
                        SpecialAction::StepOut => {
                            if !self.exchanger.is_messaging_enabled() {
                                warn!(target: "tui", "try step-out but messaging disabled");
                                return None;
                            }

                            self.exchanger
                                .request_async(|dbg| {
                                    Ok(command::step_out::Handler::new(dbg).handle()?)
                                })
                                .expect("messaging enabled");

                            Msg::AppRunning
                        }
                        _ => Msg::None,
                    }
                } else {
                    Msg::None
                }
            }
            Event::User(UserEvent::AsyncErrorResponse(err)) => {
                self.exchanger.enable_messaging();
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
            | Event::User(UserEvent::Step { .. })
            | Event::User(UserEvent::Watchpoint { .. }) => {
                self.exchanger.enable_messaging();
                Msg::None
            }
            _ => Msg::None,
        };
        Some(msg)
    }
}
