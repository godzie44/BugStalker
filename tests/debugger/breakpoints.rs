use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::{assert_no_proc, HW_APP, SHARED_LIB_APP};
use crate::{prepare_debugee_process, CALC_APP};
use bugstalker::debugger::Debugger;
use serial_test::serial;

#[test]
#[serial]
fn test_debugee_run() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let mut debugger = Debugger::new(process, TestHooks::default()).unwrap();
    debugger.start_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_multiple_brkpt_on_addr() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut dbg = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    dbg.set_breakpoint_at_line("hello_world.rs", 5).unwrap();
    dbg.set_breakpoint_at_line("hello_world.rs", 9).unwrap();

    dbg.start_debugee().unwrap();

    // save addresses
    assert_eq!(info.line.take(), Some(5));
    let addr_1 = info.addr.take().unwrap();

    dbg.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(9));
    let addr_2 = info.addr.take().unwrap();

    dbg.remove_breakpoint_at_line("hello_world.rs", 5).unwrap();
    dbg.remove_breakpoint_at_line("hello_world.rs", 9).unwrap();

    // set new breakpoints at addresses
    dbg.set_breakpoint_at_addr(addr_1).unwrap();
    dbg.set_breakpoint_at_addr(addr_2).unwrap();

    // restart
    dbg.restart_debugee().unwrap();

    // assert stop points
    assert_eq!(info.line.take(), Some(5));

    dbg.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(9));

    dbg.continue_debugee().unwrap();

    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_brkpt_on_function() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    debugger.set_breakpoint_at_fn("myprint").unwrap();

    debugger.start_debugee().unwrap();
    let pc1 = debugger.exploration_ctx().location().pc;
    assert!(u64::from(pc1) > 0);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    let pc2 = debugger.exploration_ctx().location().pc;
    assert_eq!(pc1, pc2);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_brkpt_on_function_name_collision() {
    let process = prepare_debugee_process(CALC_APP, &[]);
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info)).unwrap();

    // assert that two breakpoints is set
    assert_eq!(debugger.set_breakpoint_at_fn("sum2").unwrap().len(), 2);
    // assert that two breakpoints is removed
    assert_eq!(debugger.remove_breakpoint_at_fn("sum2").unwrap().len(), 2);

    // assert that two breakpoints is set
    assert_eq!(debugger.set_breakpoint_at_fn("sum3").unwrap().len(), 2);
    // assert that two breakpoints is removed
    assert_eq!(debugger.remove_breakpoint_at_fn("sum3").unwrap().len(), 2);

    // set breakpoint to function in concrete module
    assert_eq!(
        debugger.set_breakpoint_at_fn("float::sum3").unwrap().len(),
        1
    );
    assert_eq!(
        debugger
            .remove_breakpoint_at_fn("float::sum3")
            .unwrap()
            .len(),
        1
    );
}

#[test]
#[serial]
fn test_brkpt_on_line_collision() {
    let process = prepare_debugee_process(SHARED_LIB_APP, &[]);
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    debugger.set_breakpoint_at_line("main.rs", 14).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(14));

    // assert that two breakpoints is set at two lib.rs from two shared libraries
    let brkpts = debugger.set_breakpoint_at_line("lib.rs", 3).unwrap();
    assert_eq!(brkpts.len(), 2);
    // assert that two breakpoints is removed
    let brkpts = debugger.remove_breakpoint_at_line("lib.rs", 3).unwrap();
    assert_eq!(brkpts.len(), 2);

    // set breakpoint to function in concrete file
    let brkpts = debugger
        .set_breakpoint_at_line("printer_lib/src/lib.rs", 3)
        .unwrap();
    assert_eq!(brkpts.len(), 1);
    let brkpts = debugger
        .remove_breakpoint_at_line("printer_lib/src/lib.rs", 3)
        .unwrap();
    assert_eq!(brkpts.len(), 1);
}

#[test]
#[serial]
fn test_brkpt_on_line() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 15)
        .unwrap();

    debugger.start_debugee().unwrap();
    let pc1 = debugger.exploration_ctx().location().pc;
    assert!(u64::from(pc1) > 0);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    let pc2 = debugger.exploration_ctx().location().pc;
    assert_eq!(pc1, pc2);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_set_breakpoint_idempotence() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();
    debugger
        .set_breakpoint_at_line("hello_world.rs", 15)
        .unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(15));

    // set brkpt again on same address, but debugee now in execution state
    debugger
        .set_breakpoint_at_line("hello_world.rs", 15)
        .unwrap();

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
