use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::{assert_no_proc, debugger_env, SIGNALS_APP};
use nix::sys::signal;
use nix::sys::signal::{SIGUSR1, SIGUSR2};
use serial_test::serial;
use std::thread;
use std::time::Duration;

#[test]
#[serial]
fn test_signal_stop_single_thread() {
    debugger_env!(SIGNALS_APP, ["single_thread"], child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(SIGNALS_APP, child, TestHooks::new(info.clone())).unwrap();

        debugger.set_breakpoint_at_line("signals.rs", 12).unwrap();

        thread::spawn(move || {
            thread::sleep(Duration::from_secs(1));
            signal::kill(child, SIGUSR1).unwrap();
        });

        debugger.run_debugee().unwrap();

        std::thread::sleep(Duration::from_secs(1));

        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(12));

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_signal_stop_multi_thread() {
    debugger_env!(SIGNALS_APP, ["multi_thread"], child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(SIGNALS_APP, child, TestHooks::new(info.clone())).unwrap();

        debugger.set_breakpoint_at_line("signals.rs", 42).unwrap();

        thread::spawn(move || {
            thread::sleep(Duration::from_secs(1));
            signal::kill(child, SIGUSR1).unwrap();
        });

        debugger.run_debugee().unwrap();
        std::thread::sleep(Duration::from_secs(1));

        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(42));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_signal_stop_multi_thread_multiple_signal() {
    debugger_env!(SIGNALS_APP, ["multi_thread_multi_signal"], child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(SIGNALS_APP, child, TestHooks::new(info.clone())).unwrap();

        debugger.set_breakpoint_at_line("signals.rs", 62).unwrap();

        thread::spawn(move || {
            thread::sleep(Duration::from_secs(1));
            signal::kill(child, SIGUSR1).unwrap();
            signal::kill(child, SIGUSR2).unwrap();
        });

        debugger.run_debugee().unwrap();

        std::thread::sleep(Duration::from_secs(1));

        debugger.continue_debugee().unwrap();
        debugger.continue_debugee().unwrap();

        assert_eq!(info.line.take(), Some(62));
        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}
