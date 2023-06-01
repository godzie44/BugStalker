use crate::common::TestHooks;
use crate::prepare_debugee_process;
use crate::{assert_no_proc, HW_APP};
use bugstalker::debugger::Debugger;
use object::SymbolKind;
use serial_test::serial;

#[test]
#[serial]
fn test_symbol() {
    let process = prepare_debugee_process(HW_APP, &[]);
    let debugee_pid = process.pid();
    let mut debugger = Debugger::new(process, TestHooks::default()).unwrap();

    let main_sym = debugger.get_symbol("main").unwrap();
    assert_eq!(SymbolKind::Text, main_sym.kind);
    assert_ne!(main_sym.addr, 0);

    debugger.start_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
