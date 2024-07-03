use crate::common::TestInfo;
use crate::common::{rust_version, TestHooks};
use crate::CALC_APP;
use crate::{assert_no_proc, prepare_debugee_process, HW_APP, RECURSION_APP, VARS_APP};
use bugstalker::debugger::variable::value::{SupportedScalar, Value};
use bugstalker::debugger::{Debugger, DebuggerBuilder};
use bugstalker::ui::command::parser::expression;
use bugstalker::version_switch;
use chumsky::Parser;
use serial_test::serial;
use std::mem;

#[test]
#[serial]
fn test_step_into() {
    let process = prepare_debugee_process(CALC_APP, &["1", "2", "3", "--description", "result"]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("main.rs", 10).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(10));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(25));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(21));
    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(22));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(26));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(21));
    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(22));

    debugger.step_into().unwrap();
    assert_eq!(info.line.take(), Some(27));

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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_fn("infinite_inc").unwrap();

    fn assert_arg(debugger: &Debugger, expected: u64) {
        let get_i_expr = expression::parser().parse("i").unwrap();
        let i_arg = debugger.read_argument(get_i_expr).unwrap().pop().unwrap();
        let Value::Scalar(scalar) = i_arg.into_value() else {
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_fn("main").unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(5));

    debugger.step_into().unwrap();
    debugger.step_into().unwrap();
    debugger.step_into().unwrap();
    debugger.step_into().unwrap();
    let rust_version = rust_version(HW_APP).unwrap();
    version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 80, u32::MAX) => {},
            (1, 81, 0) ..= (1, u32::MAX, u32::MAX) => {
                debugger.step_into().unwrap();
            },
    );

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
    let info = TestInfo::default();
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
    // TODO this test should be reworked
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 545).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(545));
    debugger.step_over().unwrap();
    assert_eq!(info.line.take(), Some(546));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_step_over_on_fn_decl() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
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
