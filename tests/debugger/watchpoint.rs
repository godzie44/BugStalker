use crate::common::{TestHooks, TestInfo};
use crate::variables::assert_scalar_n;
use crate::{assert_no_proc, VARS_APP};
use crate::{prepare_debugee_process, CALCULATIONS_APP};
use bugstalker::debugger::address::RelocatedAddress;
use bugstalker::debugger::register::debug::{BreakCondition, BreakSize};
use bugstalker::debugger::variable::select::{Selector, DQE};
use bugstalker::debugger::variable::{PointerVariable, SupportedScalar, VariableIR};
use bugstalker::debugger::{Debugger, DebuggerBuilder};
use serial_test::serial;
use BreakCondition::DataWrites;
use BreakSize::Bytes8;

fn assert_old_new(
    info: &TestInfo,
    name: &str,
    r#type: &str,
    old: SupportedScalar,
    mb_new: Option<SupportedScalar>,
) {
    let old_val = &info.old_value.take().unwrap();
    assert_scalar_n(old_val, name, r#type, Some(old));
    match mb_new {
        None => {
            assert!(&info.new_value.take().is_none())
        }
        Some(new) => {
            let new_val = &info.new_value.take().unwrap();
            assert_scalar_n(new_val, name, r#type, Some(new));
        }
    }
}

#[test]
#[serial]
fn test_watchpoint_works() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();
    debugger.set_breakpoint_at_line("vars.rs", 7).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(7));
    let wp_dqe = DQE::Variable(Selector::by_name("int8", true));
    debugger
        .set_watchpoint_on_expr("int8", wp_dqe, DataWrites)
        .unwrap();

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(8));
    let var_val = &info.new_value.take().unwrap();
    assert_scalar_n(var_val, "int8", "i8", Some(SupportedScalar::I8(1)));
    assert!(info.file.take().unwrap().contains("vars.rs"));

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(31));
    assert!(info.new_value.take().is_none());

    assert!(debugger.watchpoint_list().is_empty());

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_watchpoint_works_2() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_line("calculations.rs", 8).unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(8));
    let wp_dqe = DQE::Variable(Selector::by_name("int8", true));
    dbg.set_watchpoint_on_expr("int8", wp_dqe, DataWrites)
        .unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I8(1), Some(SupportedScalar::I8(2)));
    assert_old_new(&info, "int8", "i8", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I8(2), Some(SupportedScalar::I8(0)));
    assert_old_new(&info, "int8", "i8", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I8(0), Some(SupportedScalar::I8(-5)));
    assert_old_new(&info, "int8", "i8", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I8(-5), Some(SupportedScalar::I8(6)));
    assert_old_new(&info, "int8", "i8", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I8(6), None);
    assert_old_new(&info, "int8", "i8", old, new);

    assert!(dbg.watchpoint_list().is_empty());
    dbg.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_watchpoint_global_var() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_fn("main").unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(108));
    let wp_dqe = DQE::Variable(Selector::by_name("GLOBAL_1", false));
    dbg.set_watchpoint_on_expr("GLOBAL_1", wp_dqe, DataWrites)
        .unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I64(1), Some(SupportedScalar::I64(0)));
    assert_old_new(&info, "calculations::GLOBAL_1", "i64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I64(0), Some(SupportedScalar::I64(3)));
    assert_old_new(&info, "calculations::GLOBAL_1", "i64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I64(3), Some(SupportedScalar::I64(1)));
    assert_old_new(&info, "calculations::GLOBAL_1", "i64", old, new);
    dbg.continue_debugee().unwrap();

    // watchpoint at global variables never removed automatically
    assert_eq!(dbg.watchpoint_list().len(), 1);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_max_watchpoint_count() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_line("calculations.rs", 22).unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(22));

    let wp_dqe = DQE::Variable(Selector::by_name("a", false));
    dbg.set_watchpoint_on_expr("a", wp_dqe, DataWrites).unwrap();
    let wp_dqe = DQE::Variable(Selector::by_name("b", false));
    dbg.set_watchpoint_on_expr("b", wp_dqe, DataWrites).unwrap();
    let wp_dqe = DQE::Variable(Selector::by_name("c", false));
    dbg.set_watchpoint_on_expr("c", wp_dqe, DataWrites).unwrap();
    let wp_dqe = DQE::Variable(Selector::by_name("d", false));
    dbg.set_watchpoint_on_expr("d", wp_dqe, DataWrites).unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(1), Some(SupportedScalar::U64(6)));
    assert_old_new(&info, "a", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(2), Some(SupportedScalar::U64(3)));
    assert_old_new(&info, "b", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(3), Some(SupportedScalar::U64(1)));
    assert_old_new(&info, "c", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(4), Some(SupportedScalar::U64(3)));
    assert_old_new(&info, "d", "u64", old, new);

    dbg.continue_debugee().unwrap();
    assert!(info.new_value.take().is_none());

    assert!(dbg.watchpoint_list().is_empty());
    dbg.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_watchpoint_remove_and_continue() {
    // 1) set 2 watchpoint with same scope end and same companion breakpoints
    // 2) remove one of watchpoint
    // 3) check that for another watchpoint stop at the end of scope works well - this means
    // that companion not deleted at step 2)
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_line("calculations.rs", 22).unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(22));

    let a_wp_dqe = DQE::Variable(Selector::by_name("a", false));
    dbg.set_watchpoint_on_expr("a", a_wp_dqe.clone(), DataWrites)
        .unwrap();
    let d_wp_dqe = DQE::Variable(Selector::by_name("d", false));
    dbg.set_watchpoint_on_expr("d", d_wp_dqe, DataWrites)
        .unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(1), Some(SupportedScalar::U64(6)));
    assert_old_new(&info, "a", "u64", old, new);
    dbg.remove_watchpoint_by_expr(a_wp_dqe).unwrap();
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(4), Some(SupportedScalar::U64(3)));
    assert_old_new(&info, "d", "u64", old, new);
    dbg.continue_debugee().unwrap();
    assert!(info.new_value.take().is_none());

    assert!(dbg.watchpoint_list().is_empty());
    dbg.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_watchpoint_global_var_multithread() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_fn("calculation_global_value_mt")
        .unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(56));

    let wp_dqe = DQE::Field(
        DQE::Field(
            DQE::Variable(Selector::by_name("GLOBAL_2", false)).boxed(),
            "data".to_string(),
        )
        .boxed(),
        "value".to_string(),
    );
    dbg.set_watchpoint_on_expr("GLOBAL_2.data.value", wp_dqe, DataWrites)
        .unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(1), Some(SupportedScalar::U64(2)));
    assert_old_new(&info, "GLOBAL_2.data.value", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(2), Some(SupportedScalar::U64(3)));
    assert_old_new(&info, "GLOBAL_2.data.value", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(3), Some(SupportedScalar::U64(4)));
    assert_old_new(&info, "GLOBAL_2.data.value", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(4), Some(SupportedScalar::U64(5)));
    assert_old_new(&info, "GLOBAL_2.data.value", "u64", old, new);
    dbg.continue_debugee().unwrap();

    // watchpoint at global variables never removed automatically
    assert_eq!(dbg.watchpoint_list().len(), 1);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_watchpoint_local_var_multithread() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_line("calculations.rs", 67).unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(67));

    let wp_dqe = DQE::Variable(Selector::by_name("a", false));
    dbg.set_watchpoint_on_expr("a", wp_dqe, DataWrites).unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(1), Some(SupportedScalar::I32(2)));
    assert_old_new(&info, "a", "i32", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(2), Some(SupportedScalar::I32(7)));
    assert_old_new(&info, "a", "i32", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(7), None);
    assert_old_new(&info, "a", "i32", old, new);

    dbg.continue_debugee().unwrap();
    assert!(dbg.watchpoint_list().is_empty());
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_max_watchpoint_count_at_address() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_line("calculations.rs", 22).unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(22));

    fn get_ptr_of(dbg: &Debugger, var: &str) -> RelocatedAddress {
        let addr_dqe = DQE::Address(DQE::Variable(Selector::by_name(var, true)).boxed());
        let var = dbg.read_variable(addr_dqe).unwrap();
        let VariableIR::Pointer(PointerVariable { value: Some(p), .. }) = var[0] else {
            panic!("not a pointer")
        };
        RelocatedAddress::from(p as usize)
    }

    let ptr_a = get_ptr_of(&dbg, "a");
    let ptr_b = get_ptr_of(&dbg, "b");
    let ptr_c = get_ptr_of(&dbg, "c");
    let ptr_d = get_ptr_of(&dbg, "d");

    let b8 = Bytes8;
    dbg.set_watchpoint_on_memory(ptr_a, b8, DataWrites).unwrap();
    dbg.set_watchpoint_on_memory(ptr_b, b8, DataWrites).unwrap();
    dbg.set_watchpoint_on_memory(ptr_c, b8, DataWrites).unwrap();
    dbg.set_watchpoint_on_memory(ptr_d, b8, DataWrites).unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(1), Some(SupportedScalar::U64(6)));
    assert_old_new(&info, "data", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(2), Some(SupportedScalar::U64(3)));
    assert_old_new(&info, "data", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(3), Some(SupportedScalar::U64(1)));
    assert_old_new(&info, "data", "u64", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::U64(4), Some(SupportedScalar::U64(3)));
    assert_old_new(&info, "data", "u64", old, new);

    dbg.remove_watchpoint_by_addr(ptr_a).unwrap();
    dbg.remove_watchpoint_by_addr(ptr_b).unwrap();
    dbg.remove_watchpoint_by_addr(ptr_c).unwrap();
    dbg.remove_watchpoint_by_addr(ptr_d).unwrap();

    dbg.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_watchpoint_argument() {
    let process = prepare_debugee_process(CALCULATIONS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut dbg = builder.build(process).unwrap();
    dbg.set_breakpoint_at_fn("calculate_from_arg").unwrap();

    dbg.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(101));

    let wp_dqe = DQE::Variable(Selector::by_name("arg", false));
    dbg.set_watchpoint_on_expr("arg", wp_dqe, DataWrites)
        .unwrap();

    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(1), Some(SupportedScalar::I32(2)));
    assert_old_new(&info, "arg", "i32", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(2), Some(SupportedScalar::I32(4)));
    assert_old_new(&info, "arg", "i32", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(4), Some(SupportedScalar::I32(-1)));
    assert_old_new(&info, "arg", "i32", old, new);
    dbg.continue_debugee().unwrap();
    let (old, new) = (SupportedScalar::I32(-1), None);
    assert_old_new(&info, "arg", "i32", old, new);

    dbg.continue_debugee().unwrap();
    assert!(dbg.watchpoint_list().is_empty());
    assert_no_proc!(debugee_pid);
}
