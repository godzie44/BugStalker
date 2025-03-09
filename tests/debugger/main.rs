mod common;

mod breakpoints;
mod io;
mod multithreaded;
mod signal;
mod steps;
mod symbol;
mod tokio;
mod unwind;
mod variables;
mod watchpoint;

use crate::common::{TestHooks, TestInfo};
use bugstalker::debugger::process::{Child, Installed};
use bugstalker::debugger::register::{Register, RegisterMap};
use bugstalker::debugger::{DebuggerBuilder, rust};
use serial_test::serial;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::thread;

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

    let runner = Child::new(
        prog,
        args.to_vec(),
        None::<&Path>,
        writer.try_clone().unwrap(),
        writer,
    );
    runner.install().unwrap()
}

const HW_APP: &str = "./examples/target/debug/hello_world";
const CALC_APP: &str = "./examples/target/debug/calc";
const MT_APP: &str = "./examples/target/debug/mt";
const VARS_APP: &str = "./examples/target/debug/vars";
const RECURSION_APP: &str = "./examples/target/debug/recursion";
const SIGNALS_APP: &str = "./examples/target/debug/signals";
const SHARED_LIB_APP: &str = "./examples/target/debug/calc_bin";
const SLEEPER_APP: &str = "./examples/target/debug/sleeper";
const FIZZBUZZ_APP: &str = "./examples/target/debug/fizzbuzz";
const CALCULATIONS_APP: &str = "./examples/target/debug/calculations";
const TOKIO_TICKER_APP: &str = "./examples/target/debug/tokioticker";
const CALLS_APP: &str = "./examples/target/debug/calls";

#[test]
#[serial]
fn test_debugger_graceful_shutdown() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let pid = process.pid();

    let builder = DebuggerBuilder::new().with_hooks(TestHooks::default());
    let mut debugger = builder.build(process).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 5)
        .unwrap();
    debugger.start_debugee().unwrap();
    drop(debugger);

    assert_no_proc!(pid);
}

#[test]
#[serial]
fn test_debugger_graceful_shutdown_multithread() {
    let process = prepare_debugee_process(MT_APP, &[]);
    let pid = process.pid();

    let builder = DebuggerBuilder::new().with_hooks(TestHooks::default());
    let mut debugger = builder.build(process).unwrap();
    debugger.set_breakpoint_at_line("mt.rs", 31).unwrap();
    debugger.start_debugee().unwrap();
    drop(debugger);

    assert_no_proc!(pid);
}

#[test]
#[serial]
fn test_frame_cfa() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();

    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();
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
    let frame_info = debugger.frame_info().unwrap();

    // expect that cfa equals stack pointer from callee function.
    assert_eq!(sp, u64::from(frame_info.cfa));

    drop(debugger);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_registers() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();

    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 5)
        .unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(5));

    // there is only info about return address (dwarf reg 16) in .debug_info section
    // so assert it with the built-in unwinder provided address
    let pc = debugger.ecx().location().pc;
    let frame = debugger.frame_info().unwrap();
    let registers = debugger.current_thread_registers_at_pc(pc).unwrap();
    let ip_register = Register::Rip
        .dwarf_register()
        .expect("instruction pointer register must map to dwarf register");
    assert_eq!(
        u64::from(frame.return_addr.unwrap()),
        registers.value(ip_register).unwrap()
    );

    drop(debugger);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_debugger_disassembler() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let pid = process.pid();

    let builder = DebuggerBuilder::new().with_hooks(TestHooks::default());
    let mut debugger = builder.build(process).unwrap();
    debugger.set_breakpoint_at_fn("main").unwrap();
    debugger.start_debugee().unwrap();

    let fn_assembly = debugger.disasm().unwrap();
    assert_eq!(fn_assembly.name, Some("hello_world::main".to_string()));
    assert!(!fn_assembly.instructions.is_empty());

    debugger.set_breakpoint_at_fn("myprint").unwrap();
    debugger.continue_debugee().unwrap();

    let fn_assembly = debugger.disasm().unwrap();
    assert_eq!(fn_assembly.name, Some("hello_world::myprint".to_string()));
    assert!(!fn_assembly.instructions.is_empty());

    drop(debugger);
    assert_no_proc!(pid);
}
