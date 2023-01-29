use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::debugger_env;
use crate::{assert_no_proc, HW_APP};
use bugstalker::debugger::{PCValue, RelocatedAddress};
use serial_test::serial;

#[test]
#[serial]
fn test_debugee_run() {
    debugger_env!(HW_APP, child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        debugger.run_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_brkpt_on_addr() {
    debugger_env!(HW_APP, child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        debugger
            .set_breakpoint(PCValue::Relocated(RelocatedAddress(0x55555555BD63)))
            .unwrap();
        debugger.run_debugee().unwrap();

        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x55555555BD63), pc);

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_multiple_brkpt_on_addr() {
    debugger_env!(HW_APP, child, {
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::default()).unwrap();
        debugger
            .set_breakpoint(PCValue::Relocated(RelocatedAddress(0x55555555BD30)))
            .unwrap();
        debugger
            .set_breakpoint(PCValue::Relocated(RelocatedAddress(0x55555555BD63)))
            .unwrap();

        debugger.run_debugee().unwrap();
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x0055555555BD30), pc);

        debugger.continue_debugee().unwrap();
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x55555555BD63), pc);

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_brkpt_on_function() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_fn("myprint").unwrap();

        debugger.run_debugee().unwrap();
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x55555555BD7E), pc);
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x55555555BD7E), pc);
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_brkpt_on_line() {
    debugger_env!(HW_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(HW_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger
            .set_breakpoint_at_line("hello_world.rs", 15)
            .unwrap();

        debugger.run_debugee().unwrap();
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x55555555BD7E), pc);
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();
        let pc = debugger.get_current_thread_pc().unwrap();
        assert_eq!(RelocatedAddress(0x55555555BD7E), pc);
        assert_eq!(info.line.take(), Some(15));

        debugger.continue_debugee().unwrap();

        assert_no_proc!(child);
    });
}
