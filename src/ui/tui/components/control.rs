use crate::ui::command;
use crate::ui::command::run;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::proto::ClientExchanger;
use crate::ui::tui::{Id, Msg};
use nix::sys::signal::Signal;
use std::sync::Arc;
use tuirealm::event::{Key, KeyEvent, KeyModifiers};
use tuirealm::{Component, Event, MockComponent, Sub, SubClause, SubEventClause};

#[derive(MockComponent)]
pub struct GlobalControl {
    component: tui_realm_stdlib::Phantom,
    exchanger: Arc<ClientExchanger>,
    already_run: bool,
}

impl GlobalControl {
    pub fn new(exchanger: Arc<ClientExchanger>, app_already_run: bool) -> Self {
        Self {
            component: tui_realm_stdlib::Phantom::default(),
            exchanger,
            already_run: app_already_run,
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
            })
            | Event::Keyboard(KeyEvent {
                code: Key::Char('c'),
                modifiers: KeyModifiers::CONTROL,
            }) => Msg::AppClose,
            Event::Keyboard(KeyEvent {
                code: Key::Char('c'),
                modifiers: KeyModifiers::NONE,
            })
            | Event::Keyboard(KeyEvent {
                code: Key::Function(9),
                ..
            }) => {
                self.exchanger
                    .request_async(|dbg| Ok(command::r#continue::Handler::new(dbg).handle()?));
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
                if self.already_run {
                    Msg::PopupConfirmDebuggerRestart
                } else {
                    self.exchanger.request_async(|dbg| {
                        Ok(run::Handler::new(dbg).handle(run::Command::Start)?)
                    });
                    self.already_run = true;
                    Msg::AppRunning
                }
            }
            Event::User(UserEvent::Signal(sig)) => Msg::ShowOkPopup(
                Some("Signal stop".to_string()),
                format!("Application receive signal: {sig}"),
            ),
            Event::Keyboard(KeyEvent {
                code: Key::Function(8),
                modifiers: KeyModifiers::NONE,
            }) => {
                self.exchanger
                    .request_async(|dbg| Ok(command::step_over::Handler::new(dbg).handle()?));
                Msg::AppRunning
            }
            Event::Keyboard(KeyEvent {
                code: Key::Function(7),
                modifiers: KeyModifiers::NONE,
            }) => {
                self.exchanger
                    .request_async(|dbg| Ok(command::step_into::Handler::new(dbg).handle()?));
                Msg::AppRunning
            }
            Event::Keyboard(KeyEvent {
                code: Key::Function(6),
                modifiers: KeyModifiers::NONE,
            }) => {
                self.exchanger
                    .request_async(|dbg| Ok(command::step_out::Handler::new(dbg).handle()?));
                Msg::AppRunning
            }
            Event::User(UserEvent::AsyncErrorResponse(err)) => {
                Msg::ShowOkPopup(Some("Error".to_string()), err)
            }
            _ => Msg::None,
        };
        Some(msg)
    }
}
