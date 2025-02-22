use crate::common::TestHooks;
use crate::prepare_debugee_process;
use crate::{HW_APP, assert_no_proc};
use bugstalker::debugger::DebuggerBuilder;
use object::SymbolKind;
use serial_test::serial;

#[test]
#[serial]
fn test_symbol() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::default());
    let mut debugger = builder.build(process).unwrap();

    let main_sym = debugger.get_symbols("^main$").unwrap()[0];
    assert_eq!(SymbolKind::Text, main_sym.kind);
    assert_ne!(usize::from(main_sym.addr), 0);

    debugger.start_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
