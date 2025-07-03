use crate::debugger::{CreateTransparentBreakpointRequest, Debugger};
use crate::oracle::builtin::nop::tui::NopComponent;
use crate::oracle::{ConsolePlugin, Oracle, TuiPlugin};
use crate::ui::console::print::ExternalPrinter;
use crate::ui::tui::Msg;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::KeyMap;
use std::sync::Arc;
use tuirealm::Component;

/// Nop-oracle, just for test purposes.
#[derive(Default)]
pub struct NopOracle {}

impl ConsolePlugin for NopOracle {
    fn print(&self, printer: &ExternalPrinter, _: Option<&str>) {
        printer.println("nop");
    }

    fn help(&self) -> &str {
        "Nop oracle, for test purposes only"
    }
}

impl TuiPlugin for NopOracle {
    fn make_tui_component(
        self: Arc<Self>,
        _: &'static KeyMap,
    ) -> Box<dyn Component<Msg, UserEvent>> {
        Box::<NopComponent>::default()
    }
}

impl Oracle for NopOracle {
    fn name(&self) -> &'static str {
        "nop"
    }

    fn ready_for_install(&self, _: &Debugger) -> bool {
        true
    }

    fn spy_points(self: Arc<Self>) -> Vec<CreateTransparentBreakpointRequest> {
        vec![]
    }
}

pub mod tui {
    use crate::ui::tui::Msg;
    use crate::ui::tui::app::port::UserEvent;
    use tui_realm_stdlib::Paragraph;
    use tuirealm::props::TextSpan;
    use tuirealm::{Component, Event, MockComponent};

    #[derive(MockComponent)]
    pub struct NopComponent {
        component: Paragraph,
    }

    impl Default for NopComponent {
        fn default() -> Self {
            Self {
                component: tui_realm_stdlib::Paragraph::default()
                    .text([TextSpan::new("Nop oracle, for test purposes only")]),
            }
        }
    }

    impl Component<Msg, UserEvent> for NopComponent {
        fn on(&mut self, _ev: Event<UserEvent>) -> Option<Msg> {
            Some(Msg::None)
        }
    }
}
