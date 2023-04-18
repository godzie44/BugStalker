use crate::common::TestHooks;
use crate::debugger_env;
use crate::{assert_no_proc, HW_APP};
use object::SymbolKind;
use serial_test::serial;

#[test]
#[serial]
fn test_symbol() {
    debugger_env!(HW_APP, [], child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        let main_sym = debugger.get_symbol("main").unwrap();
        assert_eq!(SymbolKind::Text, main_sym.kind);
        assert!(main_sym.addr != 0);

        debugger.run_debugee().unwrap();
        assert_no_proc!(child);
    });
}
