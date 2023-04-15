use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::{assert_no_proc, HW_APP, VARS_APP};
use crate::{debugger_env, CALC_APP};
use serial_test::serial;

#[test]
#[serial]
fn test_step_into() {
    debugger_env!(CALC_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(CALC_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("main").unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(2));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(11));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(7));
        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(8));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(12));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(7));
        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(8));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(13));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(3));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_out() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("main").unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        debugger.step_into().unwrap();
        assert_eq!(info.line.take(), Some(15));

        debugger.step_out().unwrap();
        assert_eq!(info.line.take(), Some(7));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_over() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("main").unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(5));

        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(7));
        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(9));
        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(10));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_over_inline_code() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 442).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(442));
        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(443));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_step_over_on_fn_decl() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 14)
            .unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(14));

        debugger.step_over().unwrap();
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();
        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}
