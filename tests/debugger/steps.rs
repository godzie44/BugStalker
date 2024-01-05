use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::CALC_APP;
use crate::{assert_no_proc, prepare_debugee_process, HW_APP, RECURSION_APP, VARS_APP};
use bugstalker::debugger::variable::{SupportedScalar, VariableIR};
use bugstalker::debugger::{Debugger, DebuggerBuilder};
use bugstalker::ui::command::parser::expression;
use serial_test::serial;
use std::mem;

#[test]
#[serial]
fn test_step_into() {
    let process = prepare_debugee_process(CALC_APP, &["1", "2", "3", "--description", "result"]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("calc.rs", 10).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(10));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(23));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(19));
    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(20));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(24));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(19));
    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(20));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(25));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_step_into_recursion() {
    let process = prepare_debugee_process(RECURSION_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_fn("infinite_inc").unwrap();

    fn assert_arg(debugger: &Debugger, expected: u64) {
        let (_, get_i_expr) = expression::expr("i").unwrap();
        let i_arg = debugger.read_argument(get_i_expr).unwrap().pop().unwrap();
        let VariableIR::Scalar(scalar) = i_arg else {
            panic!("not a scalar");
        };
        assert_eq!(scalar.value, Some(SupportedScalar::U64(expected)));
    }

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(11));
    assert_arg(&debugger, 1);

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(11));
    assert_arg(&debugger, 2);

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(11));
    assert_arg(&debugger, 3);

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(11));
    assert_arg(&debugger, 4);

    mem::drop(debugger);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_step_out() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_fn("main").unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(5));

    debugger.step_into().unwrap();
    debugger.step_into().unwrap();
    debugger.step_into().unwrap();
    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(15));

    debugger.step_out().unwrap();
    assert_eq!(info.line.take(), Some(7));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_step_over() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_fn("main").unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(5));

    debugger.step_over().unwrap();
    assert_eq!(info.line.take(), Some(7));
    debugger.step_over().unwrap();
    assert_eq!(info.line.take(), Some(9));
    debugger.step_over().unwrap();
    assert_eq!(info.line.take(), Some(10));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_step_over_inline_code() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 442).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(442));
    debugger.step_over().unwrap();
    assert_eq!(info.line.take(), Some(443));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_step_over_on_fn_decl() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger
        .set_breakpoint_at_line("hello_world.rs", 14)
        .unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(14));

    debugger.step_over().unwrap();
    assert_eq!(info.line.take(), Some(15));

    debugger.continue_debugee().unwrap();
    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
