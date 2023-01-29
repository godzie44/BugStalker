use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::debugger_env;
use crate::{assert_no_proc, MT_APP};
use bugstalker::debugger::uw::Backtrace;
use serial_test::serial;

#[test]
#[serial]
fn test_multithreaded_app_running() {
    debugger_env!(MT_APP, child, {
        let mut debugger = Debugger::new(MT_APP, child, TestHooks::default()).unwrap();
        debugger.run_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_multithreaded_breakpoints() {
    debugger_env!(MT_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(MT_APP, child, TestHooks::new(info.clone())).unwrap();
        // set breakpoint at program start.
        debugger.set_breakpoint_at_line("mt.rs", 6).unwrap();
        // set breakpoints at thread 1 code.
        debugger.set_breakpoint_at_line("mt.rs", 21).unwrap();
        // set breakpoint at thread 2 code.
        debugger.set_breakpoint_at_line("mt.rs", 31).unwrap();
        // set breakpoint at program ends.
        debugger.set_breakpoint_at_line("mt.rs", 14).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(6));
        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(31));
        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(21));
        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(14));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

fn backtrace_contains_fn(backtrace: &Backtrace, f_name: &str) -> bool {
    backtrace.iter().any(|bt_part| {
        bt_part.place.as_ref().map(|p| p.func_name.clone()) == Some(f_name.to_string())
    })
}

#[test]
#[serial]
fn test_multithreaded_backtrace() {
    debugger_env!(MT_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(MT_APP, child, TestHooks::new(info.clone())).unwrap();

        debugger.set_breakpoint_at_line("mt.rs", 21).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(21));

        let threads = debugger.thread_state().unwrap();
        let current_thread = threads.into_iter().find(|t| t.in_focus).unwrap();

        let bt = debugger.backtrace(current_thread.thread.pid).unwrap();
        assert_eq!(bt[0].place.as_ref().unwrap().func_name, "mt::sum1");
        assert!(backtrace_contains_fn(
            &bt,
            "std::sys::unix::thread::Thread::new::thread_start"
        ));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_multithreaded_trace() {
    debugger_env!(MT_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(MT_APP, child, TestHooks::new(info.clone())).unwrap();

        debugger.set_breakpoint_at_line("mt.rs", 31).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(31));

        let trace = debugger.thread_state().unwrap();
        assert_eq!(trace.len(), 3);

        trace
            .iter()
            .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "mt::main"));
        trace
            .iter()
            .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "std::thread::sleep"));
        trace
            .iter()
            .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "mt::sum2"));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}
