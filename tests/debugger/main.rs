mod common;

mod breakpoints;
mod io;
mod multithreaded;
mod signal;
mod steps;
mod symbol;
mod variables;

use crate::common::{DebugeeRunInfo, TestHooks};
use bugstalker::debugger::process::{Child, Installed};
use bugstalker::debugger::register::{Register, RegisterMap};
use bugstalker::debugger::{rust, Debugger};
use serial_test::serial;
use std::io::{BufRead, BufReader};
use std::{mem, thread};

pub fn prepare_debugee_process(prog: &str, args: &[&'static str]) -> Child<Installed> {
    let (reader, writer) = os_pipe::pipe().unwrap();

    thread::spawn(move || {
        let mut stream = BufReader::new(reader);
        loop {
            let mut line = String::new();
            let size = stream.read_line(&mut line).unwrap_or(0);
            if size == 0 {
                return;
            }
        }
    });

    rust::Environment::init(None);

    let runner = Child::new(prog, args.to_vec(), writer.try_clone().unwrap(), writer);
    runner.install().unwrap()
}

const HW_APP: &str = "./target/debug/hello_world";
const CALC_APP: &str = "./target/debug/calc";
const MT_APP: &str = "./target/debug/mt";
const VARS_APP: &str = "./target/debug/vars";
const RECURSION_APP: &str = "./target/debug/recursion";
const SIGNALS_APP: &str = "./target/debug/signals";

#[test]
#[serial]
fn test_debugger_graceful_shutdown() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let pid = process.pid();

    let mut debugger = Debugger::new(process, TestHooks::default()).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 5)
        .unwrap();
    debugger.start_debugee().unwrap();
    mem::drop(debugger);

    assert_no_proc!(pid);
}

#[test]
#[serial]
fn test_debugger_graceful_shutdown_multithread() {
    let process = prepare_debugee_process(MT_APP, &[]);
    let pid = process.pid();

    let mut debugger = Debugger::new(process, TestHooks::default()).unwrap();
    debugger.set_breakpoint_at_line("mt.rs", 31).unwrap();
    debugger.start_debugee().unwrap();
    mem::drop(debugger);

    assert_no_proc!(pid);
}

#[test]
#[serial]
fn test_frame_cfa() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();

    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 5)
        .unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 15)
        .unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(5));

    let sp = RegisterMap::current(debugee_pid)
        .unwrap()
        .value(Register::Rsp);

    debugger.continue_debugee().unwrap();
    let frame_info = debugger.frame_info(debugee_pid).unwrap();

    // expect that cfa equals stack pointer from callee function.
    assert_eq!(sp, u64::from(frame_info.cfa));

    mem::drop(debugger);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_registers() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();

    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 5)
        .unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(5));

    // there is only info about return address (dwarf reg 16) in .debug_info section
    // so assert it with libunwind provided address
    let pc = debugger.current_thread_stop_at().unwrap().pc;
    let frame = debugger.frame_info(debugee_pid).unwrap();
    let registers = debugger.current_thread_registers_at_pc(pc).unwrap();
    assert_eq!(
        u64::from(frame.return_addr.unwrap()),
        registers.value(gimli::Register(16)).unwrap()
    );

    mem::drop(debugger);
    assert_no_proc!(debugee_pid);
}
