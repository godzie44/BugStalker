use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::{assert_no_proc, CALC_APP};
use crate::{debugger_env, HW_APP};
use bugstalker::debugger::variable::render::{RenderRepr, ValueLayout};
use serial_test::serial;
use std::borrow::Cow;
use std::mem;

#[test]
#[serial]
fn test_read_register_write() {
    debugger_env!(HW_APP, child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 10)
            .unwrap();

        debugger.run_debugee().unwrap();

        debugger.set_register_value("rip", 0x55555555BD20).unwrap();

        let val = debugger.get_register_value("rip");
        assert_eq!(val.unwrap(), 0x55555555BD20);

        mem::drop(debugger);
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_backtrace() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 15)
            .unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(15));

        let bt = debugger.backtrace(child).unwrap();
        assert_eq!(bt.len(), 2);

        assert!(bt[0].place.as_ref().unwrap().start_ip != 0);
        assert_eq!(bt[0].place.as_ref().unwrap().func_name, ("myprint"));
        assert!(bt[1].place.as_ref().unwrap().start_ip != 0);
        assert_eq!(bt[1].place.as_ref().unwrap().func_name, "hello_world::main");

        debugger.continue_debugee().unwrap();
        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_value_u64() {
    debugger_env!(CALC_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(CALC_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("calc.rs", 3).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(3));

        let vars = debugger.read_local_variables().unwrap();

        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name(), "s");
        assert_eq!(vars[0].r#type(), "i64");
        let _three = "3".to_string();
        assert!(matches!(
            vars[0].value().unwrap(),
            ValueLayout::PreRendered(Cow::Owned(_three))
        ));

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}
