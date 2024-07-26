use crate::common::TestHooks;
use crate::{prepare_debugee_process, TOKIO_TICKER_APP};
use bugstalker::debugger::DebuggerBuilder;
use serial_test::serial;

#[test]
#[serial]
fn test_async0() {
    let process = prepare_debugee_process(TOKIO_TICKER_APP, &[]);
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::default());
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("main.rs", 6).unwrap();
    debugger.start_debugee().unwrap();

    let async_bt = debugger.async_backtrace().unwrap();
    assert!(!async_bt.workers.is_empty());
    assert_eq!(async_bt.block_threads.len(), 0);
    assert!(async_bt.workers.iter().any(|w| w.active_task.is_some()));
    assert!(!async_bt.tasks.is_empty());
}
