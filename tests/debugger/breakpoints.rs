use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::prepare_debugee_process;
use crate::{assert_no_proc, HW_APP};
use bugstalker::debugger::address::Address;
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
    dbg.new_breakpoint(Address::Relocated(addr_1)).unwrap();
    dbg.new_breakpoint(Address::Relocated(addr_2)).unwrap();

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
    let pc1 = debugger.current_thread_stop_at().unwrap().pc;
    assert!(u64::from(pc1) > 0);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    let pc2 = debugger.current_thread_stop_at().unwrap().pc;
    assert_eq!(pc1, pc2);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
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
    let pc1 = debugger.current_thread_stop_at().unwrap().pc;
    assert!(u64::from(pc1) > 0);
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    let pc2 = debugger.current_thread_stop_at().unwrap().pc;
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
