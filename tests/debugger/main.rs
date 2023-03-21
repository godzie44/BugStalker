mod common;

mod breakpoints;
mod io;
mod multithreaded;
mod steps;
mod symbol;
mod variables;

use crate::common::{DebugeeRunInfo, TestHooks};
use bugstalker::debugger::register;
use bugstalker::debugger::register::Register;
use serial_test::serial;
use std::mem;

const HW_APP: &str = "./target/debug/hello_world";
const CALC_APP: &str = "./target/debug/calc";
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
fn test_debugger_graceful_shutdown_multithread() {
    debugger_env!(MT_APP, child, {
        let mut debugger = Debugger::new(MT_APP, child, TestHooks::default()).unwrap();
        debugger.set_breakpoint_at_line("mt.rs", 31).unwrap();
        debugger.run_debugee().unwrap();
        mem::drop(debugger);

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_frame_cfa() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 5)
            .unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 15)
            .unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        let sp = register::get_register_value(child, Register::Rsp).unwrap();

        debugger.continue_debugee().unwrap();
        let frame_info = debugger.frame_info(child).unwrap();

        // expect that cfa equals stack pointer from callee function.
        assert_eq!(sp, u64::from(frame_info.cfa));

        mem::drop(debugger);
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_registers() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 5)
            .unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        // there is only info about return address (dwarf reg 16) in .debug_info section
        // so assert it with libunwind provided address
        let pc = debugger.current_thread_stop_at().unwrap().pc;
        let frame = debugger.frame_info(child).unwrap();
        let registers = debugger.current_thread_registers_at_pc(pc).unwrap();
        assert_eq!(
            u64::from(frame.return_addr.unwrap()),
            registers.get(gimli::Register(16)).unwrap()
        );

        mem::drop(debugger);
        assert_no_proc!(child);
    });
}
