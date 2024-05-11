use crate::common::TestHooks;
use crate::common::TestInfo;
use crate::{assert_no_proc, prepare_debugee_process, SIGNALS_APP, SLEEPER_APP};
use bugstalker::debugger::DebuggerBuilder;
use nix::sys::signal;
use nix::sys::signal::{SIGINT, SIGUSR1, SIGUSR2};
use serial_test::serial;
use std::thread;
use std::time::Duration;

#[test]
#[serial]
fn test_signal_stop_single_thread() {
    let process = prepare_debugee_process(SIGNALS_APP, &["single_thread"]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("signals.rs", 12).unwrap();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(4));
        signal::kill(debugee_pid, SIGUSR1).unwrap();
    });

    debugger.start_debugee().unwrap();

    std::thread::sleep(Duration::from_secs(1));

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(12));

    debugger.continue_debugee().unwrap();

    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_signal_stop_multi_thread() {
    let process = prepare_debugee_process(SIGNALS_APP, &["multi_thread"]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("signals.rs", 42).unwrap();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(4));
        signal::kill(debugee_pid, SIGUSR1).unwrap();
    });

    debugger.start_debugee().unwrap();
    std::thread::sleep(Duration::from_secs(1));

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(42));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_signal_stop_multi_thread_multiple_signal() {
    let process = prepare_debugee_process(SIGNALS_APP, &["multi_thread_multi_signal"]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("signals.rs", 62).unwrap();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(4));
        signal::kill(debugee_pid, SIGUSR1).unwrap();
        signal::kill(debugee_pid, SIGUSR2).unwrap();
    });

    debugger.start_debugee().unwrap();

    std::thread::sleep(Duration::from_secs(1));

    debugger.continue_debugee().unwrap();
    debugger.continue_debugee().unwrap();

    assert_eq!(info.line.take(), Some(62));
    debugger.continue_debugee().unwrap();

    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_transparent_signals() {
    let process = prepare_debugee_process(SLEEPER_APP, &["-s", "1"]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));
        signal::kill(debugee_pid, SIGINT).unwrap();
    });

    debugger.start_debugee().unwrap();

    // debugger takeover control here, just check that debugee still alive
    assert!(!debugger.thread_state().unwrap().is_empty());

    drop(debugger);
    assert_no_proc!(debugee_pid);
}
