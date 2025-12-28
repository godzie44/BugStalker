use crate::CALLS_APP;
use crate::common::{TestHooks, TestInfo};
use crate::{assert_no_proc, prepare_debugee_process};
use bugstalker::debugger::DebuggerBuilder;
use bugstalker::debugger::variable::render::{RenderValue, ValueLayout};
use serial_test::serial;

#[test]
#[serial]
fn test_unwind_restores_registers_for_caller_frame() {
    let process = prepare_debugee_process(CALLS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("calls.rs", 30).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(30));

    debugger.set_frame_into_focus(1).unwrap();
    let vars = debugger.read_local_variables().unwrap();

    let arg1 = vars
        .iter()
        .find(|var| var.identity().to_string() == "arg1")
        .expect("arg1 must be available in caller frame");
    let arg2 = vars
        .iter()
        .find(|var| var.identity().to_string() == "arg2")
        .expect("arg2 must be available in caller frame");

    match arg1.value().value_layout().unwrap() {
        ValueLayout::PreRendered(rendered) => assert_eq!(rendered.as_ref(), "100"),
        layout => panic!("unexpected arg1 layout: {layout:?}"),
    }

    match arg2.value().value_layout().unwrap() {
        ValueLayout::PreRendered(rendered) => assert_eq!(rendered.as_ref(), "101"),
        layout => panic!("unexpected arg2 layout: {layout:?}"),
    }

    drop(debugger);
    assert_no_proc!(debugee_pid);
}
