use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::{assert_no_proc, CALC_APP};
use crate::{debugger_env, HW_APP};
use bugstalker::debugger;
use bugstalker::debugger::variable::render::{RenderRepr, ValueRepr};
use debugger::PCValue;
use debugger::RelocatedAddress;
use serial_test::serial;
use std::borrow::Cow;

#[test]
#[serial]
fn test_read_register_write() {
    debugger_env!(HW_APP, child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        let brkpt_addr = PCValue::Relocated(RelocatedAddress(0x55555555BD6C));
        debugger.set_breakpoint(brkpt_addr).unwrap();

        debugger.run_debugee().unwrap();

        // move pc to program start
        debugger.set_register_value("rip", 0x55555555BD20).unwrap();

        debugger.continue_debugee().unwrap();

        // assert that breakpoint hit again
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(pc, RelocatedAddress(0x55555555BD6C));

        debugger.continue_debugee().unwrap();

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

        assert_eq!(bt[0].place.as_ref().unwrap().start_ip, 0x0055555555BD70);
        assert_eq!(bt[0].place.as_ref().unwrap().func_name, ("myprint"));
        assert_eq!(bt[1].place.as_ref().unwrap().start_ip, 0x0055555555BD20);
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
            ValueRepr::PreRendered(Cow::Owned(_three))
        ));

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}
