mod common;

mod breakpoints;
mod io;
mod multithreaded;
mod steps;
mod symbol;
mod variables;

use crate::common::TestHooks;
use serial_test::serial;
use std::mem;

const HW_APP: &str = "./tests/hello_world";
const CALC_APP: &str = "./tests/calc";
const MT_APP: &str = "./target/debug/mt";
const VARS_APP: &str = "./target/debug/vars";

#[test]
#[serial]
fn test_debugger_graceful_shutdown() {
    debugger_env!(HW_APP, child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 5)
            .unwrap();
        debugger.run_debugee().unwrap();
        mem::drop(debugger);

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_debugger_graceful_shutdown_multiprocess() {
    debugger_env!(MT_APP, child, {
        let mut debugger = Debugger::new(MT_APP, child, TestHooks::default()).unwrap();
        debugger.set_breakpoint_at_line("mt.rs", 31).unwrap();
        debugger.run_debugee().unwrap();
        mem::drop(debugger);

        assert_no_proc!(child);
    });
}
