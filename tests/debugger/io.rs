use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::HW_APP;
use crate::{assert_no_proc, prepare_debugee_process, CALC_APP};
use bugstalker::debugger::variable::render::{RenderRepr, ValueLayout};
use bugstalker::debugger::DebuggerBuilder;
use serial_test::serial;
use std::borrow::Cow;
use std::mem;

#[test]
#[serial]
fn test_read_register_write() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::default());
    let mut debugger = builder.build(process).unwrap();

    debugger
        .set_breakpoint_at_line("hello_world.rs", 10)
        .unwrap();

    debugger.start_debugee().unwrap();

    debugger.set_register_value("rip", 0x55555555BD20).unwrap();

    let val = debugger.get_register_value("rip");
    assert_eq!(val.unwrap(), 0x55555555BD20);

    mem::drop(debugger);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_backtrace() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger
        .set_breakpoint_at_line("hello_world.rs", 15)
        .unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(15));

    let bt = debugger.backtrace(debugee_pid).unwrap();
    assert_eq!(bt.len(), 11);

    assert_ne!(bt[0].fn_start_ip.unwrap().as_u64(), 0);
    assert!(bt[0].func_name.as_ref().unwrap().contains("myprint"));
    assert_ne!(bt[1].fn_start_ip.unwrap().as_u64(), 0);
    assert_eq!(bt[1].func_name.as_ref().unwrap(), "hello_world::main");

    debugger.continue_debugee().unwrap();
    debugger.continue_debugee().unwrap();

    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_value_u64() {
    let process = prepare_debugee_process(CALC_APP, &["1", "2", "3", "--description", "result"]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();
    debugger.set_breakpoint_at_line("calc.rs", 15).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(15));

    let vars = debugger.read_local_variables().unwrap();

    assert_eq!(vars.len(), 5);
    assert_eq!(vars[4].name(), "s");
    assert_eq!(vars[4].r#type(), "i64");
    let _six = "6".to_string();
    assert!(matches!(
        vars[4].value().unwrap(),
        ValueLayout::PreRendered(Cow::Owned(_six))
    ));

    debugger.continue_debugee().unwrap();

    assert_no_proc!(debugee_pid);
}
