//! Oracles system.
//! Oracle is an optional plugin for debugger. Oracles use `watch points` for analyse
//! debug information and visualize it. As example - tokio oracle can can keep track of active
//! tasks. There is builtin and external (created by user) oracles.

pub mod builtin;

use crate::debugger::CreateTransparentBreakpointRequest;
use crate::debugger::Debugger;
use crate::ui::console::print::ExternalPrinter;
use crate::ui::tui::app::port::UserEvent;
use crate::ui::tui::config::KeyMap;
use crate::ui::tui::Msg;
use std::sync::Arc;
use tuirealm::Component;

pub trait ConsolePlugin {
    /// Print information into console.
    ///
    /// # Arguments
    ///
    /// * `printer`: console printer instance
    /// * `subcommand`: subcommand referenced to oracle
    fn print(&self, printer: &ExternalPrinter, subcommand: Option<&str>);

    /// Return help information about specific oracle.
    fn help(&self) -> &str;
}

pub trait TuiPlugin: Send + Sync {
    /// Return tui component for visualize oracle information.
    fn make_tui_component(
        self: Arc<Self>,
        keymap: &'static KeyMap,
    ) -> Box<dyn Component<Msg, UserEvent>>;
}

pub trait Oracle: ConsolePlugin + TuiPlugin {
    /// Return oracle name.
    fn name(&self) -> &'static str;

    /// True if oracle is ready for install on specific debugee.
    /// If false, then the debugger will not use this oracle.
    /// Typically, in this method, oracle will check some symbols and
    /// make a decision about the possibility of further work.
    ///
    /// # Arguments
    ///
    /// * `dbg`: debugger instance
    fn ready_for_install(&self, dbg: &Debugger) -> bool;

    /// A list of spy-points using by oracle.
    /// In debugger spy-point implemented by transparent breakpoints.
    fn spy_points(self: Arc<Self>) -> Vec<CreateTransparentBreakpointRequest>;
}
