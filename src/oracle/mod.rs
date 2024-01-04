//! Oracles system.
//! Oracle is an optional plugin for debugger. Oracles use `watch points` for analyse
//! debug information and visualize it. As example - tokio oracle can can keep track of active
//! tasks. There is builtin and external (created by user) oracles.

pub mod builtin;

use crate::debugger::CreateTransparentBreakpointRequest;
use crate::debugger::Debugger;
use crate::ui::console::print::ExternalPrinter;
use std::rc::Rc;

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

pub trait Oracle: ConsolePlugin {
    /// Return oracle name.
    fn name(&self) -> &'static str;

    /// True if oracle is ready for install on specific debugee. If false then debugger will
    /// not use this oracle. Typically, in this method, oracle will check some symbols and
    /// makes a decision about the possibility of further work.
    ///
    /// # Arguments
    ///
    /// * `dbg`: debugger instance
    fn ready_for_install(&self, dbg: &Debugger) -> bool;

    /// A list of watch_point using by oracle. In debugger watch point implement by transparent
    /// breakpoints.
    fn watch_points(self: Rc<Self>) -> Vec<CreateTransparentBreakpointRequest>;
}
