use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::VARS_APP;
use crate::{assert_no_proc, prepare_debugee_process};
use bugstalker::debugger;
use bugstalker::debugger::variable::render::RenderRepr;
use bugstalker::debugger::variable::select::{Expression, VariableSelector};
use bugstalker::debugger::variable::{select, VariableIR};
use bugstalker::debugger::{variable, Debugger};
use bugstalker::ui::command::parser::expression;
use debugger::variable::SupportedScalar;
use serial_test::serial;

fn assert_scalar(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_val: Option<SupportedScalar>,
) {
    let VariableIR::Scalar(scalar) = var else {
      panic!("not a scalar");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    assert_eq!(scalar.value, exp_val);
}

fn assert_struct(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    for_each_member: impl Fn(usize, &VariableIR),
) {
    let VariableIR::Struct(structure) = var else {
      panic!("not a struct");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    for (i, member) in structure.members.iter().enumerate() {
        for_each_member(i, member)
    }
}

fn assert_array(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    for_each_item: impl Fn(usize, &VariableIR),
) {
    let VariableIR::Array(array) = var else {
      panic!("not a array");
    };
    assert_eq!(array.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(array.type_name.as_ref().unwrap(), exp_type);
    for (i, item) in array.items.as_ref().unwrap_or(&vec![]).iter().enumerate() {
        for_each_item(i, item)
    }
}

fn assert_c_enum(var: &VariableIR, exp_name: &str, exp_type: &str, exp_value: Option<String>) {
    let VariableIR::CEnum(c_enum) = var else {
      panic!("not a c_enum");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    assert_eq!(c_enum.value, exp_value);
}

fn assert_rust_enum(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_value: impl FnOnce(&VariableIR),
) {
    let VariableIR::RustEnum(rust_enum) = var else {
        panic!("not a c_enum");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    with_value(rust_enum.value.as_ref().unwrap());
}

fn assert_pointer(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Pointer(ptr) = var else {
        panic!("not a pointer");
    };
    assert_eq!(ptr.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(ptr.type_name.as_ref().unwrap(), exp_type);
}

fn assert_vec(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Vector {vec: Some(vector), ..}) = var else {
        panic!("not a vector");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    let VariableIR::Scalar(capacity) = &vector.structure.members[1] else {
        panic!("no capacity");
    };
    assert_eq!(capacity.value, Some(SupportedScalar::Usize(exp_cap)));
    with_buf(&vector.structure.members[0]);
}

fn assert_string(var: &VariableIR, exp_name: &str, exp_value: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::String {string: Some(string), ..}) = var else {
        panic!("not a string");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(string.value, exp_value);
}

fn assert_str(var: &VariableIR, exp_name: &str, exp_value: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Str {string: Some(str), ..}) = var else {
        panic!("not a &str");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(str.value, exp_value);
}

fn assert_init_tls(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_var: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Tls {tls_var: Some(tls), ..}) = var else {
        panic!("not a tls");
    };
    assert_eq!(tls.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(tls.inner_type.as_ref().unwrap(), exp_type);
    with_var(tls.inner_value.as_ref().unwrap());
}

fn assert_uninit_tls(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Tls {tls_var: Some(tls), ..}) = var else {
        panic!("not a tls");
    };
    assert_eq!(tls.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(tls.inner_type.as_ref().unwrap(), exp_type);
    assert!(tls.inner_value.is_none());
}

fn assert_hashmap(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::HashMap {map: Some(map), ..}) = var else {
        panic!("not a hashmap");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    let mut items = map.kv_items.clone();
    items.sort_by(|v1, v2| {
        let k1_render = format!("{:?}", v1.0.value());
        let k2_render = format!("{:?}", v2.0.value());
        k1_render.cmp(&k2_render)
    });
    with_kv_items(&items);
}

fn assert_hashset(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_items: impl FnOnce(&Vec<VariableIR>),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::HashSet {set: Some(set), ..}) = var else {
        panic!("not a hashset");
    };
    assert_eq!(set.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(set.type_name.as_ref().unwrap(), exp_type);
    let mut items = set.items.clone();
    items.sort_by(|v1, v2| {
        let k1_render = format!("{:?}", v1.value());
        let k2_render = format!("{:?}", v2.value());
        k1_render.cmp(&k2_render)
    });
    with_items(&items);
}

fn assert_btree_map(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::BTreeMap {map: Some(map), ..}) = var else {
        panic!("not a BTreeMap");
    };
    assert_eq!(map.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(map.type_name.as_ref().unwrap(), exp_type);
    with_kv_items(&map.kv_items);
}

fn assert_btree_set(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_items: impl FnOnce(&Vec<VariableIR>),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::BTreeSet {set: Some(set), ..}) = var else {
        panic!("not a BTreeSet");
    };
    assert_eq!(set.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(set.type_name.as_ref().unwrap(), exp_type);
    with_items(&set.items);
}

fn assert_vec_deque(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::VecDeque {vec: Some(vector), ..}) = var else {
        panic!("not a VecDeque");
    };
    assert_eq!(vector.structure.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(vector.structure.type_name.as_ref().unwrap(), exp_type);
    let VariableIR::Scalar(capacity) = &vector.structure.members[1] else {
        panic!("no capacity");
    };
    assert_eq!(capacity.value, Some(SupportedScalar::Usize(exp_cap)));
    with_buf(&vector.structure.members[0]);
}

fn assert_cell(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_value: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Cell {value, ..}) = var else {
        panic!("not a Cell");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    with_value(value.as_ref().unwrap());
}

fn assert_refcell(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_borrow: isize,
    with_value: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::RefCell {value, ..}) = var else {
        panic!("not a Cell");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
    let value = &**value.as_ref().unwrap();
    let VariableIR::Struct(as_struct) = value else {
        panic!("not a struct")
    };

    let VariableIR::Scalar(borrow) = &as_struct.members[0] else {
        panic!("no capacity");
    };
    assert_eq!(
        borrow.value.as_ref().unwrap(),
        &SupportedScalar::Isize(exp_borrow)
    );
    with_value(&as_struct.members[1]);
}

fn assert_rc(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Rc {..}) = var else {
        panic!("not an rc");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
}

fn assert_arc(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Arc {..}) = var else {
        panic!("not an arc");
    };
    assert_eq!(var.name(), exp_name);
    assert_eq!(var.r#type(), exp_type);
}

#[test]
#[serial]
fn test_read_scalar_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 30).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(30));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar(&vars[0], "int8", "i8", Some(SupportedScalar::I8(1)));
    assert_scalar(&vars[1], "int16", "i16", Some(SupportedScalar::I16(-1)));
    assert_scalar(&vars[2], "int32", "i32", Some(SupportedScalar::I32(2)));
    assert_scalar(&vars[3], "int64", "i64", Some(SupportedScalar::I64(-2)));
    assert_scalar(&vars[4], "int128", "i128", Some(SupportedScalar::I128(3)));
    assert_scalar(&vars[5], "isize", "isize", Some(SupportedScalar::Isize(-3)));
    assert_scalar(&vars[6], "uint8", "u8", Some(SupportedScalar::U8(1)));
    assert_scalar(&vars[7], "uint16", "u16", Some(SupportedScalar::U16(2)));
    assert_scalar(&vars[8], "uint32", "u32", Some(SupportedScalar::U32(3)));
    assert_scalar(&vars[9], "uint64", "u64", Some(SupportedScalar::U64(4)));
    assert_scalar(&vars[10], "uint128", "u128", Some(SupportedScalar::U128(5)));
    assert_scalar(&vars[11], "usize", "usize", Some(SupportedScalar::Usize(6)));
    assert_scalar(&vars[12], "f32", "f32", Some(SupportedScalar::F32(1.1)));
    assert_scalar(&vars[13], "f64", "f64", Some(SupportedScalar::F64(1.2)));
    assert_scalar(
        &vars[14],
        "boolean_true",
        "bool",
        Some(SupportedScalar::Bool(true)),
    );
    assert_scalar(
        &vars[15],
        "boolean_false",
        "bool",
        Some(SupportedScalar::Bool(false)),
    );
    assert_scalar(
        &vars[16],
        "char_ascii",
        "char",
        Some(SupportedScalar::Char('a')),
    );
    assert_scalar(
        &vars[17],
        "char_non_ascii",
        "char",
        Some(SupportedScalar::Char('😊')),
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_scalar_variables_at_place() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 11).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(11));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 4);

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_struct() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 53).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(53));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar(&vars[0], "tuple_0", "()", Some(SupportedScalar::Empty()));
    assert_struct(&vars[1], "tuple_1", "(f64, f64)", |i, member| match i {
        0 => assert_scalar(member, "0", "f64", Some(SupportedScalar::F64(0f64))),
        1 => assert_scalar(member, "1", "f64", Some(SupportedScalar::F64(1.1f64))),
        _ => panic!("2 members expected"),
    });
    assert_struct(
        &vars[2],
        "tuple_2",
        "(u64, i64, char, bool)",
        |i, member| match i {
            0 => assert_scalar(member, "0", "u64", Some(SupportedScalar::U64(1))),
            1 => assert_scalar(member, "1", "i64", Some(SupportedScalar::I64(-1))),
            2 => assert_scalar(member, "2", "char", Some(SupportedScalar::Char('a'))),
            3 => assert_scalar(member, "3", "bool", Some(SupportedScalar::Bool(false))),
            _ => panic!("4 members expected"),
        },
    );
    assert_struct(&vars[3], "foo", "Foo", |i, member| match i {
        0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(100))),
        1 => assert_scalar(member, "baz", "char", Some(SupportedScalar::Char('9'))),
        _ => panic!("2 members expected"),
    });
    assert_struct(&vars[4], "foo2", "Foo2", |i, member| match i {
        0 => assert_struct(member, "foo", "Foo", |i, member| match i {
            0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(100))),
            1 => assert_scalar(member, "baz", "char", Some(SupportedScalar::Char('9'))),
            _ => panic!("2 members expected"),
        }),
        1 => assert_scalar(
            member,
            "additional",
            "bool",
            Some(SupportedScalar::Bool(true)),
        ),
        _ => panic!("2 members expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_array() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 61).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(61));

    let vars = debugger.read_local_variables().unwrap();
    assert_array(&vars[0], "arr_1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(-1))),
        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(2))),
        3 => assert_scalar(item, "3", "i32", Some(SupportedScalar::I32(-2))),
        4 => assert_scalar(item, "4", "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("5 items expected"),
    });
    assert_array(&vars[1], "arr_2", "[[i32]]", |i, item| match i {
        0 => assert_array(item, "0", "[i32]", |i, item| match i {
            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(-1))),
            2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(2))),
            3 => assert_scalar(item, "3", "i32", Some(SupportedScalar::I32(-2))),
            4 => assert_scalar(item, "4", "i32", Some(SupportedScalar::I32(3))),
            _ => panic!("5 items expected"),
        }),
        1 => assert_array(item, "1", "[i32]", |i, item| match i {
            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(0))),
            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(1))),
            2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(2))),
            3 => assert_scalar(item, "3", "i32", Some(SupportedScalar::I32(3))),
            4 => assert_scalar(item, "4", "i32", Some(SupportedScalar::I32(4))),
            _ => panic!("5 items expected"),
        }),
        2 => assert_array(item, "2", "[i32]", |i, item| match i {
            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(0))),
            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(-1))),
            2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(-2))),
            3 => assert_scalar(item, "3", "i32", Some(SupportedScalar::I32(-3))),
            4 => assert_scalar(item, "4", "i32", Some(SupportedScalar::I32(-4))),
            _ => panic!("5 items expected"),
        }),
        _ => panic!("3 items expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_enum() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 93).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(93));

    let vars = debugger.read_local_variables().unwrap();
    assert_c_enum(&vars[0], "enum_1", "EnumA", Some("B".to_string()));
    assert_rust_enum(&vars[1], "enum_2", "EnumC", |enum_val| {
        assert_struct(enum_val, "C", "C", |_, member| {
            assert_scalar(member, "0", "char", Some(SupportedScalar::Char('b')));
        });
    });
    assert_rust_enum(&vars[2], "enum_3", "EnumC", |enum_val| {
        assert_struct(enum_val, "D", "D", |i, member| {
            match i {
                0 => assert_scalar(member, "0", "f64", Some(SupportedScalar::F64(1.1))),
                1 => assert_scalar(member, "1", "f32", Some(SupportedScalar::F32(1.2))),
                _ => panic!("2 members expected"),
            };
        });
    });
    assert_rust_enum(&vars[3], "enum_4", "EnumC", |enum_val| {
        assert_struct(enum_val, "E", "E", |_, _| {
            panic!("expected empty struct");
        });
    });
    assert_rust_enum(&vars[4], "enum_5", "EnumF", |enum_val| {
        assert_struct(enum_val, "F", "F", |i, member| {
            match i {
                0 => assert_rust_enum(member, "0", "EnumC", |enum_val| {
                    assert_struct(enum_val, "C", "C", |_, member| {
                        assert_scalar(member, "0", "char", Some(SupportedScalar::Char('f')));
                    });
                }),
                _ => panic!("1 members expected"),
            };
        });
    });
    assert_rust_enum(&vars[5], "enum_6", "EnumF", |enum_val| {
        assert_struct(enum_val, "G", "G", |i, member| {
            match i {
                0 => assert_struct(member, "0", "Foo", |i, member| match i {
                    0 => assert_scalar(member, "a", "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(member, "b", "char", Some(SupportedScalar::Char('1'))),
                    _ => panic!("2 members expected"),
                }),
                _ => panic!("1 members expected"),
            };
        });
    });
    assert_rust_enum(&vars[6], "enum_7", "EnumF", |enum_val| {
        assert_struct(enum_val, "J", "J", |i, member| {
            match i {
                0 => assert_c_enum(member, "0", "EnumA", Some("A".to_string())),
                _ => panic!("1 members expected"),
            };
        });
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

fn make_select_plan(expr: &str) -> Expression {
    let (_, expr) = expression::expr(expr).unwrap();
    expr
}

fn read_single_var(debugger: &Debugger, expr: &str) -> VariableIR {
    debugger
        .read_variable(make_select_plan(expr))
        .unwrap()
        .pop()
        .unwrap()
}

fn read_single_arg(debugger: &Debugger, expr: &str) -> VariableIR {
    debugger
        .read_argument(make_select_plan(expr))
        .unwrap()
        .pop()
        .unwrap()
}

#[test]
#[serial]
fn test_read_pointers() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 119).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(119));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar(&vars[0], "a", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer(&vars[1], "ref_a", "&i32");
    let deref = read_single_var(&debugger, "*ref_a");
    assert_scalar(&deref, "*ref_a", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer(&vars[2], "ptr_a", "*const i32");
    let deref = read_single_var(&debugger, "*ptr_a");
    assert_scalar(&deref, "*ptr_a", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer(&vars[3], "ptr_ptr_a", "*const *const i32");
    let deref = read_single_var(&debugger, "*ptr_ptr_a");
    assert_pointer(&deref, "*ptr_ptr_a", "*const i32");
    let deref = read_single_var(&debugger, "**ptr_ptr_a");
    assert_scalar(&deref, "**ptr_ptr_a", "i32", Some(SupportedScalar::I32(2)));

    assert_scalar(&vars[4], "b", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer(&vars[5], "mut_ref_b", "&mut i32");
    let deref = read_single_var(&debugger, "*mut_ref_b");
    assert_scalar(&deref, "*mut_ref_b", "i32", Some(SupportedScalar::I32(2)));

    assert_scalar(&vars[6], "c", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer(&vars[7], "mut_ptr_c", "*mut i32");
    let deref = read_single_var(&debugger, "*mut_ptr_c");
    assert_scalar(&deref, "*mut_ptr_c", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer(
        &vars[8],
        "box_d",
        "alloc::boxed::Box<i32, alloc::alloc::Global>",
    );
    let deref = read_single_var(&debugger, "*box_d");
    assert_scalar(&deref, "*box_d", "i32", Some(SupportedScalar::I32(2)));

    assert_struct(&vars[9], "f", "Foo", |i, member| match i {
        0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(1))),
        1 => assert_array(member, "baz", "[i32]", |i, item| match i {
            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
            _ => panic!("2 items expected"),
        }),
        2 => {
            assert_pointer(member, "foo", "&i32");
            let deref = read_single_var(&debugger, "*f.foo");
            assert_scalar(&deref, "*foo", "i32", Some(SupportedScalar::I32(2)));
        }
        _ => panic!("3 members expected"),
    });

    assert_pointer(&vars[10], "ref_f", "&vars::references::Foo");
    let deref = read_single_var(&debugger, "*ref_f");
    assert_struct(&deref, "*ref_f", "Foo", |i, member| match i {
        0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(1))),
        1 => assert_array(member, "baz", "[i32]", |i, item| match i {
            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
            _ => panic!("2 items expected"),
        }),
        2 => {
            assert_pointer(member, "foo", "&i32");
            let deref = read_single_var(&debugger, "*(*ref_f).foo");
            assert_scalar(&deref, "*foo", "i32", Some(SupportedScalar::I32(2)));
        }
        _ => panic!("3 members expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_type_alias() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 126).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(126));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar(&vars[0], "a_alias", "i32", Some(SupportedScalar::I32(1)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_type_parameters() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 135).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(135));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct(&vars[0], "a", "Foo<i32>", |i, member| match i {
        0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(1))),
        _ => panic!("1 members expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_vec_and_slice() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 151).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(151));

    let vars = debugger.read_local_variables().unwrap();
    assert_vec(
        &vars[0],
        "vec1",
        "Vec<i32, alloc::alloc::Global>",
        3,
        |buf| {
            assert_array(buf, "buf", "[i32]", |i, item| match i {
                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            })
        },
    );
    assert_vec(
        &vars[1],
        "vec2",
        "Vec<vars::vec_and_slice_types::Foo, alloc::alloc::Global>",
        2,
        |buf| {
            assert_array(buf, "buf", "[Foo]", |i, item| match i {
                0 => assert_struct(item, "0", "Foo", |i, member| match i {
                    0 => assert_scalar(member, "foo", "i32", Some(SupportedScalar::I32(1))),
                    _ => panic!("1 members expected"),
                }),
                1 => assert_struct(item, "1", "Foo", |i, member| match i {
                    0 => assert_scalar(member, "foo", "i32", Some(SupportedScalar::I32(2))),
                    _ => panic!("1 members expected"),
                }),
                _ => panic!("2 items expected"),
            })
        },
    );
    assert_vec(
        &vars[2],
        "vec3",
        "Vec<alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        2,
        |buf| {
            assert_array(
                buf,
                "buf",
                "[Vec<i32, alloc::alloc::Global>]",
                |i, item| match i {
                    0 => assert_vec(item, "0", "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                        assert_array(buf, "buf", "[i32]", |i, item| match i {
                            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                            2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                            _ => panic!("3 items expected"),
                        })
                    }),
                    1 => assert_vec(item, "1", "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                        assert_array(buf, "buf", "[i32]", |i, item| match i {
                            0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                            1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                            2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                            _ => panic!("3 items expected"),
                        })
                    }),
                    _ => panic!("2 items expected"),
                },
            )
        },
    );

    assert_pointer(&vars[3], "slice1", "&[i32; 3]");
    let deref = read_single_var(&debugger, "*slice1");
    assert_array(&deref, "*slice1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("3 items expected"),
    });

    assert_pointer(&vars[4], "slice2", "&[&[i32; 3]; 2]");
    let deref = read_single_var(&debugger, "*slice2");
    assert_array(&deref, "*slice2", "[&[i32; 3]]", |i, item| match i {
        0 => {
            assert_pointer(item, "0", "&[i32; 3]");
            let deref = read_single_var(&debugger, "*(*slice2)[0]");
            assert_array(&deref, "*0", "[i32]", |i, item| match i {
                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            });
        }
        1 => {
            assert_pointer(item, "1", "&[i32; 3]");
            let deref = read_single_var(&debugger, "*(*slice2)[1]");
            assert_array(&deref, "*1", "[i32]", |i, item| match i {
                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            });
        }
        _ => panic!("2 items expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_strings() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 159).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(159));

    let vars = debugger.read_local_variables().unwrap();
    assert_string(&vars[0], "s1", "hello world");
    assert_str(&vars[1], "s2", "hello world");
    assert_str(&vars[2], "s3", "hello world");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_static_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 168).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(168));

    let vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "GLOB_1".to_string(),
        )))
        .unwrap();
    assert_eq!(vars.len(), 1);
    assert_str(&vars[0], "vars::GLOB_1", "glob_1");

    let vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "GLOB_2".to_string(),
        )))
        .unwrap();
    assert_eq!(vars.len(), 1);
    assert_scalar(
        &vars[0],
        "vars::GLOB_2",
        "i32",
        Some(SupportedScalar::I32(2)),
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_static_variables_different_modules() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 179).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(179));

    let mut vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "GLOB_3".to_string(),
        )))
        .unwrap();
    assert_eq!(vars.len(), 2);
    vars.sort_by(|v1, v2| v1.r#type().cmp(v2.r#type()));

    assert_str(&vars[0], "vars::ns_1::GLOB_3", "glob_3");
    assert_scalar(
        &vars[1],
        "vars::GLOB_3",
        "i32",
        Some(SupportedScalar::I32(3)),
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_tls_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 194).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(194));

    let vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "THREAD_LOCAL_VAR_1".to_string(),
        )))
        .unwrap();
    assert_init_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>", |inner| {
        assert_cell(inner, "0", "Cell<i32>", |value| {
            assert_scalar(value, "value", "i32", Some(SupportedScalar::I32(2)))
        })
    });

    let vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "THREAD_LOCAL_VAR_2".to_string(),
        )))
        .unwrap();
    assert_init_tls(&vars[0], "THREAD_LOCAL_VAR_2", "Cell<&str>", |inner| {
        assert_cell(inner, "0", "Cell<&str>", |value| {
            assert_str(value, "value", "2")
        })
    });

    // assert uninit tls variables
    debugger.set_breakpoint_at_line("vars.rs", 199).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(199));

    let vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "THREAD_LOCAL_VAR_1".to_string(),
        )))
        .unwrap();
    assert_uninit_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>");

    // assert tls variables changes in another thread
    debugger.set_breakpoint_at_line("vars.rs", 203).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(203));

    let vars = debugger
        .read_variable(Expression::Variable(VariableSelector::Name(
            "THREAD_LOCAL_VAR_1".to_string(),
        )))
        .unwrap();
    assert_init_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>", |inner| {
        assert_cell(inner, "0", "Cell<i32>", |value| {
            assert_scalar(value, "value", "i32", Some(SupportedScalar::I32(1)))
        })
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_closures() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 223).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(223));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct(&vars[0], "inc", "{closure_env#0}", |_, _| {
        panic!("no members expected")
    });
    assert_struct(&vars[1], "inc_mut", "{closure_env#1}", |_, _| {
        panic!("no members expected")
    });
    assert_struct(&vars[3], "closure", "{closure_env#2}", |_, member| {
        assert_string(member, "outer", "outer val")
    });
    assert_struct(
        &vars[7],
        "trait_once",
        "alloc::boxed::Box<dyn core::ops::function::FnOnce<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_pointer(
                    member,
                    "pointer",
                    "*dyn core::ops::function::FnOnce<(), Output=()>",
                );
                let deref = read_single_var(&debugger, "*trait_once.pointer");
                assert_struct(
                    &deref,
                    "*pointer",
                    "dyn core::ops::function::FnOnce<(), Output=()>",
                    |_, _| {},
                );
            }
            1 => {
                assert_pointer(member, "vtable", "&[usize; 3]");
                let deref = read_single_var(&debugger, "*trait_once.vtable");
                assert_array(&deref, "*vtable", "[usize]", |i, _| match i {
                    0 | 1 | 2 => {}
                    _ => panic!("3 items expected"),
                });
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_struct(
        &vars[8],
        "trait_mut",
        "alloc::boxed::Box<dyn core::ops::function::FnMut<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_pointer(
                    member,
                    "pointer",
                    "*dyn core::ops::function::FnMut<(), Output=()>",
                );
                let deref = read_single_var(&debugger, "*trait_mut.pointer");
                assert_struct(
                    &deref,
                    "*pointer",
                    "dyn core::ops::function::FnMut<(), Output=()>",
                    |_, _| {},
                );
            }
            1 => {
                assert_pointer(member, "vtable", "&[usize; 3]");
                let deref = read_single_var(&debugger, "*trait_mut.vtable");
                assert_array(&deref, "*vtable", "[usize]", |i, _| match i {
                    0 | 1 | 2 => {}
                    _ => panic!("3 items expected"),
                });
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_struct(
        &vars[9],
        "trait_fn",
        "alloc::boxed::Box<dyn core::ops::function::Fn<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_pointer(
                    member,
                    "pointer",
                    "*dyn core::ops::function::Fn<(), Output=()>",
                );
                let deref = read_single_var(&debugger, "*trait_fn.pointer");
                assert_struct(
                    &deref,
                    "*pointer",
                    "dyn core::ops::function::Fn<(), Output=()>",
                    |_, _| {},
                );
            }
            1 => {
                assert_pointer(member, "vtable", "&[usize; 3]");
                let deref = read_single_var(&debugger, "*trait_fn.vtable");
                assert_array(&deref, "*vtable", "[usize]", |i, _| match i {
                    0 | 1 | 2 => {}
                    _ => panic!("3 items expected"),
                });
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_pointer(&vars[10], "fn_ptr", "fn() -> u8");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_arguments() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 232).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(232));

    let args = debugger
        .read_argument(select::Expression::Variable(VariableSelector::Any))
        .unwrap();
    assert_scalar(&args[0], "by_val", "i32", Some(SupportedScalar::I32(1)));
    assert_pointer(&args[1], "by_ref", "&i32");
    let deref = read_single_arg(&debugger, "*by_ref");
    assert_scalar(&deref, "*by_ref", "i32", Some(SupportedScalar::I32(2)));

    assert_vec(&args[2], "vec", "Vec<u8, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "buf", "[u8]", |i, item| match i {
            0 => assert_scalar(item, "0", "u8", Some(SupportedScalar::U8(3))),
            1 => assert_scalar(item, "1", "u8", Some(SupportedScalar::U8(4))),
            2 => assert_scalar(item, "2", "u8", Some(SupportedScalar::U8(5))),
            _ => panic!("3 items expected"),
        })
    });
    assert_struct(
        &args[3],
        "box_arr",
        "alloc::boxed::Box<[u8], alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_pointer(member, "data_ptr", "*u8");
                let deref = read_single_arg(&debugger, "*box_arr.data_ptr");
                assert_scalar(&deref, "*data_ptr", "u8", Some(SupportedScalar::U8(6)));
            }
            1 => assert_scalar(member, "length", "usize", Some(SupportedScalar::Usize(3))),
            _ => panic!("2 members expected"),
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_union() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 244).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(244));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct(&vars[0], "union", "Union1", |i, member| match i {
        0 => assert_scalar(member, "f1", "f32", Some(SupportedScalar::F32(1.1))),
        1 => {}
        2 => {}
        _ => panic!("3 members expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_hashmap() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 261).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(261));

    let vars = debugger.read_local_variables().unwrap();
    assert_hashmap(
        &vars[0],
        "hm1",
        "HashMap<bool, i64, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "0", "bool", Some(SupportedScalar::Bool(false)));
            assert_scalar(&items[0].1, "1", "i64", Some(SupportedScalar::I64(5)));
            assert_scalar(&items[1].0, "0", "bool", Some(SupportedScalar::Bool(true)));
            assert_scalar(&items[1].1, "1", "i64", Some(SupportedScalar::I64(3)));
        },
    );
    assert_hashmap(
        &vars[1],
        "hm2",
        "HashMap<&str, alloc::vec::Vec<i32, alloc::alloc::Global>, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_str(
                &items[0].0,
                "0",
                "abc",
            );
            assert_vec(&items[0].1, "1", "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                assert_array(buf, "buf", "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                    2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                    _ => panic!("3 items expected"),
                })
            });
            assert_str(
                &items[1].0,
                "0",
                "efg",
            );
            assert_vec(&items[1].1, "1", "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                assert_array(buf, "buf", "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(11))),
                    1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(12))),
                    2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(13))),
                    _ => panic!("3 items expected"),
                })
            });
        },
    );
    assert_hashmap(
        &vars[2],
        "hm3",
        "HashMap<i32, i32, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 100);

            let mut exp_items = (0..100).collect::<Vec<_>>();
            exp_items.sort_by_key(|i1| i1.to_string());

            for i in 0..100 {
                assert_scalar(
                    &items[i].0,
                    "0",
                    "i32",
                    Some(SupportedScalar::I32(exp_items[i])),
                );
            }
            for i in 0..100 {
                assert_scalar(
                    &items[i].1,
                    "1",
                    "i32",
                    Some(SupportedScalar::I32(exp_items[i])),
                );
            }
        },
    );
    assert_hashmap(
        &vars[3],
        "hm4",
        "HashMap<alloc::string::String, std::collections::hash::map::HashMap<i32, i32, std::collections::hash::map::RandomState>, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_string(
                &items[0].0,
                "0",
                "1",
            );
            assert_hashmap(
                &items[0].1,
                "1",
                "HashMap<i32, i32, std::collections::hash::map::RandomState>",
                |items| {
                    assert_eq!(items.len(), 2);
                    assert_scalar(
                        &items[0].0,
                        "0",
                        "i32",
                        Some(SupportedScalar::I32(1)),
                    );
                    assert_scalar(&items[0].1, "1", "i32", Some(SupportedScalar::I32(1)));
                    assert_scalar(
                        &items[1].0,
                        "0",
                        "i32",
                        Some(SupportedScalar::I32(2)),
                    );
                    assert_scalar(&items[1].1, "1", "i32", Some(SupportedScalar::I32(2)));
                },
            );

            assert_string(
                &items[1].0,
                "0",
                "3",
            );
            assert_hashmap(
                &items[1].1,
                "1",
                "HashMap<i32, i32, std::collections::hash::map::RandomState>",
                |items| {
                    assert_eq!(items.len(), 2);
                    assert_scalar(
                        &items[0].0,
                        "0",
                        "i32",
                        Some(SupportedScalar::I32(3)),
                    );
                    assert_scalar(&items[0].1, "1", "i32", Some(SupportedScalar::I32(3)));
                    assert_scalar(
                        &items[1].0,
                        "0",
                        "i32",
                        Some(SupportedScalar::I32(4)),
                    );
                    assert_scalar(&items[1].1, "1", "i32", Some(SupportedScalar::I32(4)));
                },
            );
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_hashset() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 274).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(274));

    let vars = debugger.read_local_variables().unwrap();
    assert_hashset(
        &vars[0],
        "hs1",
        "HashSet<i32, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 4);
            assert_scalar(&items[0], "0", "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[1], "0", "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[2], "0", "i32", Some(SupportedScalar::I32(3)));
            assert_scalar(&items[3], "0", "i32", Some(SupportedScalar::I32(4)));
        },
    );
    assert_hashset(
        &vars[1],
        "hs2",
        "HashSet<i32, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 100);
            let mut exp_items = (0..100).into_iter().collect::<Vec<_>>();
            exp_items.sort_by_key(|i1| i1.to_string());

            for i in 0..100 {
                assert_scalar(
                    &items[i],
                    "0",
                    "i32",
                    Some(SupportedScalar::I32(exp_items[i])),
                );
            }
        },
    );
    assert_hashset(
        &vars[2],
        "hs3",
        "HashSet<alloc::vec::Vec<i32, alloc::alloc::Global>, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_vec(&items[0], "0", "Vec<i32, alloc::alloc::Global>", 2, |buf| {
                assert_array(buf, "buf", "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                    _ => panic!("2 items expected"),
                })
            });
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_circular_ref_types() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 301).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(301));

    let vars = debugger.read_local_variables().unwrap();
    assert_rc(&vars[0], "a_circ", "Rc<vars::circular::List>");
    assert_rc(&vars[1], "b_circ", "Rc<vars::circular::List>");

    let deref = read_single_var(&debugger, "*a_circ");
    assert_struct(
        &deref,
        "*a_circ",
        "RcBox<vars::circular::List>",
        |i, member| match i {
            0 => assert_cell(member, "strong", "Cell<usize>", |inner| {
                assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            1 => assert_cell(member, "weak", "Cell<usize>", |inner| {
                assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(1)))
            }),
            2 => {
                assert_rust_enum(member, "value", "List", |enum_member| {
                    assert_struct(enum_member, "Cons", "Cons", |i, cons_member| match i {
                        0 => assert_scalar(cons_member, "0", "i32", Some(SupportedScalar::I32(5))),
                        1 => assert_refcell(
                            cons_member,
                            "1",
                            "RefCell<alloc::rc::Rc<vars::circular::List>>",
                            0,
                            |inner| assert_rc(inner, "value", "Rc<vars::circular::List>"),
                        ),
                        _ => panic!("2 members expected"),
                    });
                });
            }
            _ => panic!("3 members expected"),
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_lexical_blocks() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 307).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(307));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].name(), "alpha");

    debugger.set_breakpoint_at_line("vars.rs", 309).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(309));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name(), "alpha");
    assert_eq!(vars[1].name(), "beta");

    debugger.set_breakpoint_at_line("vars.rs", 310).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(310));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 3);
    assert_eq!(vars[0].name(), "alpha");
    assert_eq!(vars[1].name(), "beta");
    assert_eq!(vars[2].name(), "gama");

    debugger.set_breakpoint_at_line("vars.rs", 316).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(316));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name(), "alpha");
    assert_eq!(vars[1].name(), "delta");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_btree_map() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 334).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(334));

    let vars = debugger.read_local_variables().unwrap();
    assert_btree_map(
        &vars[0],
        "hm1",
        "BTreeMap<bool, i64, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "k", "bool", Some(SupportedScalar::Bool(false)));
            assert_scalar(&items[0].1, "v", "i64", Some(SupportedScalar::I64(5)));
            assert_scalar(&items[1].0, "k", "bool", Some(SupportedScalar::Bool(true)));
            assert_scalar(&items[1].1, "v", "i64", Some(SupportedScalar::I64(3)));
        },
    );
    assert_btree_map(
        &vars[1],
        "hm2",
        "BTreeMap<&str, alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_str(&items[0].0, "k", "abc");
            assert_vec(
                &items[0].1,
                "v",
                "Vec<i32, alloc::alloc::Global>",
                3,
                |buf| {
                    assert_array(buf, "buf", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    })
                },
            );
            assert_str(&items[1].0, "k", "efg");
            assert_vec(
                &items[1].1,
                "v",
                "Vec<i32, alloc::alloc::Global>",
                3,
                |buf| {
                    assert_array(buf, "buf", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(11))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(12))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(13))),
                        _ => panic!("3 items expected"),
                    })
                },
            );
        },
    );
    assert_btree_map(
        &vars[2],
        "hm3",
        "BTreeMap<i32, i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 100);

            let exp_items = (0..100).collect::<Vec<_>>();

            for i in 0..100 {
                assert_scalar(
                    &items[i].0,
                    "k",
                    "i32",
                    Some(SupportedScalar::I32(exp_items[i])),
                );
            }
            for i in 0..100 {
                assert_scalar(
                    &items[i].1,
                    "v",
                    "i32",
                    Some(SupportedScalar::I32(exp_items[i])),
                );
            }
        },
    );
    assert_btree_map(
        &vars[3],
        "hm4",
        "BTreeMap<alloc::string::String, alloc::collections::btree::map::BTreeMap<i32, i32, alloc::alloc::Global>, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_string(&items[0].0, "k", "1");
            assert_btree_map(
                &items[0].1,
                "v",
                "BTreeMap<i32, i32, alloc::alloc::Global>",
                |items| {
                    assert_eq!(items.len(), 2);
                    assert_scalar(&items[0].0, "k", "i32", Some(SupportedScalar::I32(1)));
                    assert_scalar(&items[0].1, "v", "i32", Some(SupportedScalar::I32(1)));
                    assert_scalar(&items[1].0, "k", "i32", Some(SupportedScalar::I32(2)));
                    assert_scalar(&items[1].1, "v", "i32", Some(SupportedScalar::I32(2)));
                },
            );

            assert_string(
                &items[1].0,
                "k",
                "3",
            );
            assert_btree_map(
                &items[1].1,
                "v",
                "BTreeMap<i32, i32, alloc::alloc::Global>",
                |items| {
                    assert_eq!(items.len(), 2);
                    assert_scalar(&items[0].0, "k", "i32", Some(SupportedScalar::I32(3)));
                    assert_scalar(&items[0].1, "v", "i32", Some(SupportedScalar::I32(3)));
                    assert_scalar(&items[1].0, "k", "i32", Some(SupportedScalar::I32(4)));
                    assert_scalar(&items[1].1, "v", "i32", Some(SupportedScalar::I32(4)));
                },
            );
        });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_btree_set() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 347).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(347));

    let vars = debugger.read_local_variables().unwrap();
    assert_btree_set(
        &vars[0],
        "hs1",
        "BTreeSet<i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 4);
            assert_scalar(&items[0], "k", "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[1], "k", "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[2], "k", "i32", Some(SupportedScalar::I32(3)));
            assert_scalar(&items[3], "k", "i32", Some(SupportedScalar::I32(4)));
        },
    );
    assert_btree_set(
        &vars[1],
        "hs2",
        "BTreeSet<i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 100);
            let exp_items = (0..100).collect::<Vec<_>>();

            for i in 0..100 {
                assert_scalar(
                    &items[i],
                    "k",
                    "i32",
                    Some(SupportedScalar::I32(exp_items[i])),
                );
            }
        },
    );
    assert_btree_set(
        &vars[2],
        "hs3",
        "BTreeSet<alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_vec(&items[0], "k", "Vec<i32, alloc::alloc::Global>", 2, |buf| {
                assert_array(buf, "buf", "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                    _ => panic!("2 items expected"),
                })
            });
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_vec_deque() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 365).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(365));

    let vars = debugger.read_local_variables().unwrap();
    assert_vec_deque(
        &vars[0],
        "vd1",
        "VecDeque<i32, alloc::alloc::Global>",
        8,
        |buf| {
            assert_array(buf, "buf", "[i32]", |i, item| match i {
                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(9))),
                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(10))),
                2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(0))),
                3 => assert_scalar(item, "3", "i32", Some(SupportedScalar::I32(1))),
                4 => assert_scalar(item, "4", "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("5 items expected"),
            })
        },
    );

    assert_vec_deque(
        &vars[1],
        "vd2",
        "VecDeque<alloc::collections::vec_deque::VecDeque<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        4,
        |buf| {
            assert_array(buf, "buf", "[VecDeque<i32, alloc::alloc::Global>]", |i, item| match i {
                0 => assert_vec_deque(item, "0", "VecDeque<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf, "buf", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(-2))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(-1))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(0))),
                        _ => panic!("3 items expected"),
                    })
                }),
                1 => assert_vec_deque(item, "1", "VecDeque<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf, "buf", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    })
                }),
                2 => assert_vec_deque(item, "2", "VecDeque<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf, "buf", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(4))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(5))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(6))),
                        _ => panic!("3 items expected"),
                    })
                }),
                _ => panic!("3 items expected"),
            })
        });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_atomic() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 375).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(375));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct(&vars[0], "int32_atomic", "AtomicI32", |i, member| match i {
        0 => assert_struct(member, "v", "UnsafeCell<i32>", |i, member| match i {
            0 => assert_scalar(member, "value", "i32", Some(SupportedScalar::I32(1))),
            _ => panic!("1 members expected"),
        }),
        _ => panic!("1 members expected"),
    });

    assert_struct(
        &vars[2],
        "int32_atomic_ptr",
        "AtomicPtr<i32>",
        |i, member| match i {
            0 => assert_struct(member, "p", "UnsafeCell<*mut i32>", |i, member| match i {
                0 => assert_pointer(member, "value", "*mut i32"),
                _ => panic!("1 members expected"),
            }),
            _ => panic!("1 members expected"),
        },
    );

    let deref = read_single_var(&debugger, "*int32_atomic_ptr.p.value");
    assert_scalar(&deref, "*value", "i32", Some(SupportedScalar::I32(2)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_cell() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 387).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(387));

    let vars = debugger.read_local_variables().unwrap();
    assert_cell(&vars[0], "a_cell", "Cell<i32>", |value| {
        assert_scalar(value, "value", "i32", Some(SupportedScalar::I32(1)))
    });

    assert_refcell(
        &vars[1],
        "b_refcell",
        "RefCell<alloc::vec::Vec<i32, alloc::alloc::Global>>",
        2,
        |value| {
            assert_vec(value, "value", "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                assert_array(buf, "buf", "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                    2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                    _ => panic!("3 items expected"),
                })
            })
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_shared_ptr() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 409).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(409));

    let vars = debugger.read_local_variables().unwrap();
    assert_rc(&vars[0], "rc0", "Rc<i32>");
    let deref = read_single_var(&debugger, "*rc0");
    assert_struct(&deref, "*rc0", "RcBox<i32>", |i, member| match i {
        0 => assert_cell(member, "strong", "Cell<usize>", |inner| {
            assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
        }),
        1 => assert_cell(member, "weak", "Cell<usize>", |inner| {
            assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
        }),
        2 => assert_scalar(member, "value", "i32", Some(SupportedScalar::I32(1))),
        _ => panic!("3 members expected"),
    });
    assert_rc(&vars[1], "rc1", "Rc<i32>");
    let deref = read_single_var(&debugger, "*rc1");
    assert_struct(&deref, "*rc1", "RcBox<i32>", |i, member| match i {
        0 => assert_cell(member, "strong", "Cell<usize>", |inner| {
            assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
        }),
        1 => assert_cell(member, "weak", "Cell<usize>", |inner| {
            assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
        }),
        2 => assert_scalar(member, "value", "i32", Some(SupportedScalar::I32(1))),
        _ => panic!("3 members expected"),
    });
    assert_rc(&vars[2], "weak_rc2", "Weak<i32>");
    let deref = read_single_var(&debugger, "*weak_rc2");
    assert_struct(&deref, "*weak_rc2", "RcBox<i32>", |i, member| match i {
        0 => assert_cell(member, "strong", "Cell<usize>", |inner| {
            assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
        }),
        1 => assert_cell(member, "weak", "Cell<usize>", |inner| {
            assert_scalar(inner, "value", "usize", Some(SupportedScalar::Usize(2)))
        }),
        2 => assert_scalar(member, "value", "i32", Some(SupportedScalar::I32(1))),
        _ => panic!("3 members expected"),
    });

    assert_arc(&vars[3], "arc0", "Arc<i32>");
    let deref = read_single_var(&debugger, "*arc0");
    assert_struct(&deref, "*arc0", "ArcInner<i32>", |i, member| match i {
        0 => assert_struct(member, "strong", "AtomicUsize", |i, member| match i {
            0 => assert_struct(member, "v", "UnsafeCell<usize>", |_, member| {
                assert_scalar(member, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            _ => panic!("1 member expected"),
        }),
        1 => assert_struct(member, "weak", "AtomicUsize", |i, member| match i {
            0 => assert_struct(member, "v", "UnsafeCell<usize>", |_, member| {
                assert_scalar(member, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            _ => panic!("1 member expected"),
        }),
        2 => assert_scalar(member, "data", "i32", Some(SupportedScalar::I32(2))),
        _ => panic!("3 members expected"),
    });
    assert_arc(&vars[4], "arc1", "Arc<i32>");
    let deref = read_single_var(&debugger, "*arc1");
    assert_struct(&deref, "*arc1", "ArcInner<i32>", |i, member| match i {
        0 => assert_struct(member, "strong", "AtomicUsize", |i, member| match i {
            0 => assert_struct(member, "v", "UnsafeCell<usize>", |_, member| {
                assert_scalar(member, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            _ => panic!("1 member expected"),
        }),
        1 => assert_struct(member, "weak", "AtomicUsize", |i, member| match i {
            0 => assert_struct(member, "v", "UnsafeCell<usize>", |_, member| {
                assert_scalar(member, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            _ => panic!("1 member expected"),
        }),
        2 => assert_scalar(member, "data", "i32", Some(SupportedScalar::I32(2))),
        _ => panic!("3 members expected"),
    });
    assert_arc(&vars[5], "weak_arc2", "Weak<i32>");
    let deref = read_single_var(&debugger, "*weak_arc2");
    assert_struct(&deref, "*weak_arc2", "ArcInner<i32>", |i, member| match i {
        0 => assert_struct(member, "strong", "AtomicUsize", |i, member| match i {
            0 => assert_struct(member, "v", "UnsafeCell<usize>", |_, member| {
                assert_scalar(member, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            _ => panic!("1 member expected"),
        }),
        1 => assert_struct(member, "weak", "AtomicUsize", |i, member| match i {
            0 => assert_struct(member, "v", "UnsafeCell<usize>", |_, member| {
                assert_scalar(member, "value", "usize", Some(SupportedScalar::Usize(2)))
            }),
            _ => panic!("1 member expected"),
        }),
        2 => assert_scalar(member, "data", "i32", Some(SupportedScalar::I32(2))),
        _ => panic!("3 members expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_zst_types() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone())).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 430).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(430));

    let vars = debugger.read_local_variables().unwrap();

    assert_pointer(&vars[0], "ptr_zst", "&()");
    let deref = read_single_var(&debugger, "*ptr_zst");
    assert_scalar(&deref, "*ptr_zst", "()", Some(SupportedScalar::Empty()));

    assert_array(&vars[1], "array_zst", "[()]", |i, item| match i {
        0 => assert_scalar(item, "0", "()", Some(SupportedScalar::Empty())),
        1 => assert_scalar(item, "1", "()", Some(SupportedScalar::Empty())),
        _ => panic!("2 members expected"),
    });

    assert_vec(
        &vars[2],
        "vec_zst",
        "Vec<(), alloc::alloc::Global>",
        0,
        |buf| {
            assert_array(buf, "buf", "[()]", |i, item| match i {
                0 => assert_scalar(item, "0", "()", Some(SupportedScalar::Empty())),
                1 => assert_scalar(item, "1", "()", Some(SupportedScalar::Empty())),
                2 => assert_scalar(item, "2", "()", Some(SupportedScalar::Empty())),
                _ => panic!("3 members expected"),
            })
        },
    );

    assert_vec(
        &vars[2],
        "vec_zst",
        "Vec<(), alloc::alloc::Global>",
        0,
        |buf| {
            assert_array(buf, "buf", "[()]", |i, item| match i {
                0 => assert_scalar(item, "0", "()", Some(SupportedScalar::Empty())),
                1 => assert_scalar(item, "1", "()", Some(SupportedScalar::Empty())),
                2 => assert_scalar(item, "2", "()", Some(SupportedScalar::Empty())),
                _ => panic!("3 members expected"),
            })
        },
    );

    assert_pointer(&vars[3], "slice_zst", "&[(); 4]");
    let deref = read_single_var(&debugger, "*slice_zst");
    assert_array(&deref, "*slice_zst", "[()]", |i, item| match i {
        0 => assert_scalar(item, "0", "()", Some(SupportedScalar::Empty())),
        1 => assert_scalar(item, "1", "()", Some(SupportedScalar::Empty())),
        2 => assert_scalar(item, "2", "()", Some(SupportedScalar::Empty())),
        3 => assert_scalar(item, "3", "()", Some(SupportedScalar::Empty())),
        _ => panic!("4 members expected"),
    });

    assert_struct(&vars[4], "struct_zst", "StructZst", |i, member| match i {
        0 => assert_scalar(member, "0", "()", Some(SupportedScalar::Empty())),
        _ => panic!("1 member expected"),
    });

    assert_rust_enum(&vars[5], "enum_zst", "Option<()>", |member| {
        assert_struct(member, "Some", "Some", |i, member| match i {
            0 => assert_scalar(member, "0", "()", Some(SupportedScalar::Empty())),
            _ => panic!("1 member expected"),
        })
    });

    assert_vec_deque(
        &vars[6],
        "vecdeque_zst",
        "VecDeque<(), alloc::alloc::Global>",
        0,
        |buf| {
            assert_array(buf, "buf", "[()]", |i, item| match i {
                0 => assert_scalar(item, "0", "()", Some(SupportedScalar::Empty())),
                1 => assert_scalar(item, "1", "()", Some(SupportedScalar::Empty())),
                2 => assert_scalar(item, "2", "()", Some(SupportedScalar::Empty())),
                3 => assert_scalar(item, "3", "()", Some(SupportedScalar::Empty())),
                4 => assert_scalar(item, "4", "()", Some(SupportedScalar::Empty())),
                _ => panic!("5 members expected"),
            })
        },
    );

    assert_hashmap(
        &vars[7],
        "hash_map_zst_key",
        "HashMap<(), i32, std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "0", "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "1", "i32", Some(SupportedScalar::I32(1)));
        },
    );
    assert_hashmap(
        &vars[8],
        "hash_map_zst_val",
        "HashMap<i32, (), std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "0", "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[0].1, "1", "()", Some(SupportedScalar::Empty()));
        },
    );
    assert_hashmap(
        &vars[9],
        "hash_map_zst",
        "HashMap<(), (), std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "0", "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "1", "()", Some(SupportedScalar::Empty()));
        },
    );
    assert_hashset(
        &vars[10],
        "hash_set_zst",
        "HashSet<(), std::collections::hash::map::RandomState>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0], "0", "()", Some(SupportedScalar::Empty()));
        },
    );

    assert_btree_map(
        &vars[11],
        "btree_map_zst_key",
        "BTreeMap<(), i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "k", "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "v", "i32", Some(SupportedScalar::I32(1)));
        },
    );
    assert_btree_map(
        &vars[12],
        "btree_map_zst_val",
        "BTreeMap<i32, (), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "k", "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[0].1, "v", "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[1].0, "k", "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[1].1, "v", "()", Some(SupportedScalar::Empty()));
        },
    );
    assert_btree_map(
        &vars[13],
        "btree_map_zst",
        "BTreeMap<(), (), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "k", "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "v", "()", Some(SupportedScalar::Empty()));
        },
    );
    assert_btree_set(
        &vars[14],
        "btree_set_zst",
        "BTreeSet<(), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0], "k", "()", Some(SupportedScalar::Empty()));
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
