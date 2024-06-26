use crate::VARS_APP;
use crate::common::TestHooks;
use crate::common::{TestInfo, rust_version};
use crate::{assert_no_proc, prepare_debugee_process};
use bugstalker::debugger::variable::render::RenderRepr;
use bugstalker::debugger::variable::select::{Literal, LiteralOrWildcard, Selector, DQE};
use bugstalker::debugger::variable::{Member, VariableIR};
use bugstalker::debugger::{variable, Debugger, DebuggerBuilder};
use bugstalker::ui::command::parser::expression;
use bugstalker::version::Version;
use bugstalker::{debugger, version_switch};
use chumsky::Parser;
use debugger::variable::SupportedScalar;
use serial_test::serial;
use std::collections::HashMap;
use variable::SpecializedVariableIR;

pub fn assert_scalar_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    exp_val: Option<SupportedScalar>,
) {
    let VariableIR::Scalar(scalar) = var else {
        panic!("not a scalar");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    assert_eq!(scalar.value, exp_val);
}

pub fn assert_scalar_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_val: Option<SupportedScalar>,
) {
    assert_scalar_inner(var, Some(exp_name), exp_type, exp_val)
}

pub fn assert_scalar(var: &VariableIR, exp_type: &str, exp_val: Option<SupportedScalar>) {
    assert_scalar_inner(var, None, exp_type, exp_val)
}

fn assert_struct_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    for_each_member: impl Fn(usize, &Member),
) {
    let VariableIR::Struct(structure) = var else {
        panic!("not a struct");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    for (i, member) in structure.members.iter().enumerate() {
        for_each_member(i, member)
    }
}

fn assert_struct_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    for_each_member: impl Fn(usize, &Member),
) {
    assert_struct_inner(var, Some(exp_name), exp_type, for_each_member)
}

fn assert_struct(var: &VariableIR, exp_type: &str, for_each_member: impl Fn(usize, &Member)) {
    assert_struct_inner(var, None, exp_type, for_each_member)
}

fn assert_member(member: &Member, expected_field_name: &str, with_value: impl Fn(&VariableIR)) {
    assert_eq!(member.field_name.as_deref(), Some(expected_field_name));
    with_value(&member.value);
}

fn assert_array_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    for_each_item: impl Fn(usize, &VariableIR),
) {
    let VariableIR::Array(array) = var else {
        panic!("not a array");
    };
    assert_eq!(array.identity.name.as_deref(), exp_name);
    assert_eq!(array.type_ident.name_fmt(), exp_type);
    for (i, item) in array.items.as_ref().unwrap_or(&vec![]).iter().enumerate() {
        for_each_item(i, &item.value)
    }
}

fn assert_array_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    for_each_item: impl Fn(usize, &VariableIR),
) {
    assert_array_inner(var, Some(exp_name), exp_type, for_each_item);
}

fn assert_array(var: &VariableIR, exp_type: &str, for_each_item: impl Fn(usize, &VariableIR)) {
    assert_array_inner(var, None, exp_type, for_each_item);
}

fn assert_c_enum_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    exp_value: Option<String>,
) {
    let VariableIR::CEnum(c_enum) = var else {
        panic!("not a c_enum");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    assert_eq!(c_enum.value, exp_value);
}

fn assert_c_enum_n(var: &VariableIR, exp_name: &str, exp_type: &str, exp_value: Option<String>) {
    assert_c_enum_inner(var, Some(exp_name), exp_type, exp_value)
}

fn assert_c_enum(var: &VariableIR, exp_type: &str, exp_value: Option<String>) {
    assert_c_enum_inner(var, None, exp_type, exp_value)
}

fn assert_rust_enum_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    with_value: impl FnOnce(&VariableIR),
) {
    let VariableIR::RustEnum(rust_enum) = var else {
        panic!("not a c_enum");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    with_value(&rust_enum.value.as_ref().unwrap().value);
}

fn assert_rust_enum_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_value: impl FnOnce(&VariableIR),
) {
    assert_rust_enum_inner(var, Some(exp_name), exp_type, with_value)
}

fn assert_rust_enum(var: &VariableIR, exp_type: &str, with_value: impl FnOnce(&VariableIR)) {
    assert_rust_enum_inner(var, None, exp_type, with_value)
}

fn assert_pointer_inner(var: &VariableIR, exp_name: Option<&str>, exp_type: &str) {
    let VariableIR::Pointer(ptr) = var else {
        panic!("not a pointer");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(ptr.type_ident.name_fmt(), exp_type);
}

fn assert_pointer_n(var: &VariableIR, exp_name: &str, exp_type: &str) {
    assert_pointer_inner(var, Some(exp_name), exp_type)
}

fn assert_pointer(var: &VariableIR, exp_type: &str) {
    assert_pointer_inner(var, None, exp_type)
}

fn assert_vec_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Vector(vector)),
        ..
    } = var
    else {
        panic!("not a vector");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    let VariableIR::Scalar(capacity) = &vector.structure.members[1].value else {
        panic!("no capacity");
    };
    assert_eq!(capacity.value, Some(SupportedScalar::Usize(exp_cap)));
    with_buf(&vector.structure.members[0].value);
}

fn assert_vec_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    assert_vec_inner(var, Some(exp_name), exp_type, exp_cap, with_buf)
}

fn assert_vec(
    var: &VariableIR,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    assert_vec_inner(var, None, exp_type, exp_cap, with_buf)
}

fn assert_string_inner(var: &VariableIR, exp_name: Option<&str>, exp_value: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::String(string)),
        ..
    } = var
    else {
        panic!("not a string");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(string.value, exp_value);
}

fn assert_string_n(var: &VariableIR, exp_name: &str, exp_value: &str) {
    assert_string_inner(var, Some(exp_name), exp_value)
}

fn assert_string(var: &VariableIR, exp_value: &str) {
    assert_string_inner(var, None, exp_value)
}

fn assert_str_inner(var: &VariableIR, exp_name: Option<&str>, exp_value: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Str(str)),
        ..
    } = var
    else {
        panic!("not a &str");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(str.value, exp_value);
}

fn assert_str_n(var: &VariableIR, exp_name: &str, exp_value: &str) {
    assert_str_inner(var, Some(exp_name), exp_value)
}

fn assert_str(var: &VariableIR, exp_value: &str) {
    assert_str_inner(var, None, exp_value)
}

fn assert_init_tls_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_var: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Tls(tls)),
        ..
    } = var
    else {
        panic!("not a tls");
    };
    assert_eq!(tls.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(tls.inner_type.name_fmt(), exp_type);
    with_var(tls.inner_value.as_ref().unwrap());
}

fn assert_uninit_tls(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Tls(tls)),
        ..
    } = var
    else {
        panic!("not a tls");
    };
    assert_eq!(tls.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(tls.inner_type.name_fmt(), exp_type);
    assert!(tls.inner_value.is_none());
}

fn assert_hashmap_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::HashMap(map)),
        ..
    } = var
    else {
        panic!("not a hashmap");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    let mut items = map.kv_items.clone();
    items.sort_by(|v1, v2| {
        let k1_render = format!("{:?}", v1.0.value());
        let k2_render = format!("{:?}", v2.0.value());
        k1_render.cmp(&k2_render)
    });
    with_kv_items(&items);
}

fn assert_hashmap_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    assert_hashmap_inner(var, Some(exp_name), exp_type, with_kv_items)
}

fn assert_hashmap(
    var: &VariableIR,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    assert_hashmap_inner(var, None, exp_type, with_kv_items)
}

fn assert_hashset(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_items: impl FnOnce(&Vec<VariableIR>),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::HashSet(set)),
        ..
    } = var
    else {
        panic!("not a hashset");
    };
    assert_eq!(set.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(set.type_ident.name_fmt(), exp_type);
    let mut items = set.items.clone();
    items.sort_by(|v1, v2| {
        let k1_render = format!("{:?}", v1.value());
        let k2_render = format!("{:?}", v2.value());
        k1_render.cmp(&k2_render)
    });
    with_items(&items);
}

fn assert_btree_map_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::BTreeMap(map)),
        ..
    } = var
    else {
        panic!("not a BTreeMap");
    };
    assert_eq!(map.identity.name.as_deref(), exp_name);
    assert_eq!(map.type_ident.name_fmt(), exp_type);
    with_kv_items(&map.kv_items);
}

fn assert_btree_map_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    assert_btree_map_inner(var, Some(exp_name), exp_type, with_kv_items)
}

fn assert_btree_map(
    var: &VariableIR,
    exp_type: &str,
    with_kv_items: impl FnOnce(&Vec<(VariableIR, VariableIR)>),
) {
    assert_btree_map_inner(var, None, exp_type, with_kv_items)
}

fn assert_btree_set(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_items: impl FnOnce(&Vec<VariableIR>),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::BTreeSet(set)),
        ..
    } = var
    else {
        panic!("not a BTreeSet");
    };
    assert_eq!(set.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(set.type_ident.name_fmt(), exp_type);
    with_items(&set.items);
}

fn assert_vec_deque_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::VecDeque(vector)),
        ..
    } = var
    else {
        panic!("not a VecDeque");
    };
    assert_eq!(vector.structure.identity.name.as_deref(), exp_name);
    assert_eq!(vector.structure.type_ident.name_fmt(), exp_type);
    let VariableIR::Scalar(capacity) = &vector.structure.members[1].value else {
        panic!("no capacity");
    };
    assert_eq!(capacity.value, Some(SupportedScalar::Usize(exp_cap)));
    with_buf(&vector.structure.members[0].value);
}
fn assert_vec_deque_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    assert_vec_deque_inner(var, Some(exp_name), exp_type, exp_cap, with_buf)
}

fn assert_vec_deque(
    var: &VariableIR,
    exp_type: &str,
    exp_cap: usize,
    with_buf: impl FnOnce(&VariableIR),
) {
    assert_vec_deque_inner(var, None, exp_type, exp_cap, with_buf)
}

fn assert_cell_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    with_value: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Cell(value)),
        ..
    } = var
    else {
        panic!("not a Cell");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    with_value(value.as_ref());
}

fn assert_cell_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_value: impl FnOnce(&VariableIR),
) {
    assert_cell_inner(var, Some(exp_name), exp_type, with_value)
}

fn assert_cell(var: &VariableIR, exp_type: &str, with_value: impl FnOnce(&VariableIR)) {
    assert_cell_inner(var, None, exp_type, with_value)
}

fn assert_refcell_inner(
    var: &VariableIR,
    exp_name: Option<&str>,
    exp_type: &str,
    exp_borrow: isize,
    with_value: impl FnOnce(&VariableIR),
) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::RefCell(value)),
        ..
    } = var
    else {
        panic!("not a Cell");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
    let VariableIR::Struct(as_struct) = value.as_ref() else {
        panic!("not a struct")
    };

    let VariableIR::Scalar(borrow) = &as_struct.members[0].value else {
        panic!("no borrow flag");
    };
    assert_eq!(
        borrow.value.as_ref().unwrap(),
        &SupportedScalar::Isize(exp_borrow)
    );
    with_value(&as_struct.members[1].value);
}

fn assert_refcell_n(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    exp_borrow: isize,
    with_inner: impl FnOnce(&VariableIR),
) {
    assert_refcell_inner(var, Some(exp_name), exp_type, exp_borrow, with_inner)
}

fn assert_refcell(
    var: &VariableIR,
    exp_type: &str,
    exp_borrow: isize,
    with_inner: impl FnOnce(&VariableIR),
) {
    assert_refcell_inner(var, None, exp_type, exp_borrow, with_inner)
}

fn assert_rc_inner(var: &VariableIR, exp_name: Option<&str>, exp_type: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Rc(_)),
        ..
    } = var
    else {
        panic!("not an rc");
    };
    assert_eq!(var.name().as_deref(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
}

fn assert_rc_n(var: &VariableIR, exp_name: &str, exp_type: &str) {
    assert_rc_inner(var, Some(exp_name), exp_type)
}

fn assert_rc(var: &VariableIR, exp_type: &str) {
    assert_rc_inner(var, None, exp_type)
}

fn assert_arc_n(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Arc(_)),
        ..
    } = var
    else {
        panic!("not an arc");
    };
    assert_eq!(var.name().unwrap(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
}

fn assert_uuid_n(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Uuid(_)),
        ..
    } = var
    else {
        panic!("not an uuid");
    };
    assert_eq!(var.name().unwrap(), exp_name);
    assert_eq!(var.r#type().name_fmt(), exp_type);
}

fn assert_system_time_n(var: &VariableIR, exp_name: &str, exp_value: (i64, u32)) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::SystemTime(value)),
        ..
    } = var
    else {
        panic!("not a SystemTime");
    };
    assert_eq!(var.name().unwrap(), exp_name);
    assert_eq!(*value, exp_value);
}

fn assert_instant_n(var: &VariableIR, exp_name: &str) {
    let VariableIR::Specialized {
        value: Some(SpecializedVariableIR::Instant(_)),
        ..
    } = var
    else {
        panic!("not an Instant");
    };
    assert_eq!(var.name().unwrap(), exp_name);
}

#[test]
#[serial]
fn test_read_scalar_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 30).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(30));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar_n(&vars[0], "int8", "i8", Some(SupportedScalar::I8(1)));
    assert_scalar_n(&vars[1], "int16", "i16", Some(SupportedScalar::I16(-1)));
    assert_scalar_n(&vars[2], "int32", "i32", Some(SupportedScalar::I32(2)));
    assert_scalar_n(&vars[3], "int64", "i64", Some(SupportedScalar::I64(-2)));
    assert_scalar_n(&vars[4], "int128", "i128", Some(SupportedScalar::I128(3)));
    assert_scalar_n(&vars[5], "isize", "isize", Some(SupportedScalar::Isize(-3)));
    assert_scalar_n(&vars[6], "uint8", "u8", Some(SupportedScalar::U8(1)));
    assert_scalar_n(&vars[7], "uint16", "u16", Some(SupportedScalar::U16(2)));
    assert_scalar_n(&vars[8], "uint32", "u32", Some(SupportedScalar::U32(3)));
    assert_scalar_n(&vars[9], "uint64", "u64", Some(SupportedScalar::U64(4)));
    assert_scalar_n(&vars[10], "uint128", "u128", Some(SupportedScalar::U128(5)));
    assert_scalar_n(&vars[11], "usize", "usize", Some(SupportedScalar::Usize(6)));
    assert_scalar_n(&vars[12], "f32", "f32", Some(SupportedScalar::F32(1.1)));
    assert_scalar_n(&vars[13], "f64", "f64", Some(SupportedScalar::F64(1.2)));
    assert_scalar_n(
        &vars[14],
        "boolean_true",
        "bool",
        Some(SupportedScalar::Bool(true)),
    );
    assert_scalar_n(
        &vars[15],
        "boolean_false",
        "bool",
        Some(SupportedScalar::Bool(false)),
    );
    assert_scalar_n(
        &vars[16],
        "char_ascii",
        "char",
        Some(SupportedScalar::Char('a')),
    );
    assert_scalar_n(
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 11).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(11));

    let vars = debugger.read_local_variables().unwrap();
    // WAITFORFIX: https://github.com/rust-lang/rust/issues/113819
    // expected: assert_eq!(vars.len(), 4);
    // through this bug there is uninitialized variable here
    assert_eq!(vars.len(), 5);

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_struct() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 53).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(53));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar_n(&vars[0], "tuple_0", "()", Some(SupportedScalar::Empty()));
    assert_struct_n(&vars[1], "tuple_1", "(f64, f64)", |i, member| match i {
        0 => assert_member(member, "__0", |val| {
            assert_scalar(val, "f64", Some(SupportedScalar::F64(0f64)))
        }),
        1 => assert_member(member, "__1", |val| {
            assert_scalar(val, "f64", Some(SupportedScalar::F64(1.1f64)))
        }),
        _ => panic!("2 members expected"),
    });
    assert_struct_n(
        &vars[2],
        "tuple_2",
        "(u64, i64, char, bool)",
        |i, member| match i {
            0 => assert_member(member, "__0", |val| {
                assert_scalar(val, "u64", Some(SupportedScalar::U64(1)))
            }),
            1 => assert_member(member, "__1", |val| {
                assert_scalar(val, "i64", Some(SupportedScalar::I64(-1)))
            }),
            2 => assert_member(member, "__2", |val| {
                assert_scalar(val, "char", Some(SupportedScalar::Char('a')))
            }),
            3 => assert_member(member, "__3", |val| {
                assert_scalar(val, "bool", Some(SupportedScalar::Bool(false)))
            }),
            _ => panic!("4 members expected"),
        },
    );
    assert_struct_n(&vars[3], "foo", "Foo", |i, member| match i {
        0 => assert_member(member, "bar", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(100)))
        }),
        1 => assert_member(member, "baz", |val| {
            assert_scalar(val, "char", Some(SupportedScalar::Char('9')))
        }),
        _ => panic!("2 members expected"),
    });
    assert_struct_n(&vars[4], "foo2", "Foo2", |i, member| match i {
        0 => assert_member(member, "foo", |val| {
            assert_struct(val, "Foo", |i, member| match i {
                0 => assert_member(member, "bar", |val| {
                    assert_scalar(val, "i32", Some(SupportedScalar::I32(100)))
                }),
                1 => assert_member(member, "baz", |val| {
                    assert_scalar(val, "char", Some(SupportedScalar::Char('9')))
                }),
                _ => panic!("2 members expected"),
            })
        }),
        1 => assert_member(member, "additional", |val| {
            assert_scalar(val, "bool", Some(SupportedScalar::Bool(true)))
        }),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 61).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(61));

    let vars = debugger.read_local_variables().unwrap();
    assert_array_n(&vars[0], "arr_1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("5 items expected"),
    });
    assert_array_n(&vars[1], "arr_2", "[[i32]]", |i, item| match i {
        0 => assert_array(item, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
            3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
            4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
            _ => panic!("5 items expected"),
        }),
        1 => assert_array(item, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(0))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
            3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
            4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(4))),
            _ => panic!("5 items expected"),
        }),
        2 => assert_array(item, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(0))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
            3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-3))),
            4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-4))),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 93).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(93));

    let vars = debugger.read_local_variables().unwrap();
    assert_c_enum_n(&vars[0], "enum_1", "EnumA", Some("B".to_string()));
    assert_rust_enum_n(&vars[1], "enum_2", "EnumC", |enum_val| {
        assert_struct(enum_val, "C", |_, member| {
            assert_member(member, "__0", |val| {
                assert_scalar(val, "char", Some(SupportedScalar::Char('b')))
            })
        });
    });
    assert_rust_enum_n(&vars[2], "enum_3", "EnumC", |enum_val| {
        assert_struct(enum_val, "D", |i, member| {
            match i {
                0 => assert_member(member, "__0", |val| {
                    assert_scalar(val, "f64", Some(SupportedScalar::F64(1.1)))
                }),
                1 => assert_member(member, "__1", |val| {
                    assert_scalar(val, "f32", Some(SupportedScalar::F32(1.2)))
                }),
                _ => panic!("2 members expected"),
            };
        });
    });
    assert_rust_enum_n(&vars[3], "enum_4", "EnumC", |enum_val| {
        assert_struct(enum_val, "E", |_, _| {
            panic!("expected empty struct");
        });
    });
    assert_rust_enum_n(&vars[4], "enum_5", "EnumF", |enum_val| {
        assert_struct(enum_val, "F", |i, member| {
            match i {
                0 => assert_member(member, "__0", |val| {
                    assert_rust_enum(val, "EnumC", |enum_val| {
                        assert_struct(enum_val, "C", |_, member| {
                            assert_member(member, "__0", |val| {
                                assert_scalar(val, "char", Some(SupportedScalar::Char('f')))
                            })
                        });
                    })
                }),
                _ => panic!("1 members expected"),
            };
        });
    });
    assert_rust_enum_n(&vars[5], "enum_6", "EnumF", |enum_val| {
        assert_struct(enum_val, "G", |i, member| {
            match i {
                0 => assert_member(member, "__0", |val| {
                    assert_struct(val, "Foo", |i, member| match i {
                        0 => assert_member(member, "a", |val| {
                            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
                        }),
                        1 => assert_member(member, "b", |val| {
                            assert_scalar(val, "char", Some(SupportedScalar::Char('1')))
                        }),
                        _ => panic!("2 members expected"),
                    })
                }),
                _ => panic!("1 members expected"),
            };
        });
    });
    assert_rust_enum_n(&vars[6], "enum_7", "EnumF", |enum_val| {
        assert_struct(enum_val, "J", |i, member| {
            match i {
                0 => assert_member(member, "__0", |val| {
                    assert_c_enum(val, "EnumA", Some("A".to_string()))
                }),
                _ => panic!("1 members expected"),
            };
        });
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

fn make_select_plan(expr: &str) -> DQE {
    expression::parser().parse(expr).unwrap()
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 119).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(119));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar_n(&vars[0], "a", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer_n(&vars[1], "ref_a", "&i32");
    let deref = read_single_var(&debugger, "*ref_a");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_pointer_n(&vars[2], "ptr_a", "*const i32");
    let deref = read_single_var(&debugger, "*ptr_a");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_pointer_n(&vars[3], "ptr_ptr_a", "*const *const i32");
    let deref = read_single_var(&debugger, "*ptr_ptr_a");
    assert_pointer(&deref, "*const i32");
    let deref = read_single_var(&debugger, "**ptr_ptr_a");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_scalar_n(&vars[4], "b", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer_n(&vars[5], "mut_ref_b", "&mut i32");
    let deref = read_single_var(&debugger, "*mut_ref_b");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_scalar_n(&vars[6], "c", "i32", Some(SupportedScalar::I32(2)));

    assert_pointer_n(&vars[7], "mut_ptr_c", "*mut i32");
    let deref = read_single_var(&debugger, "*mut_ptr_c");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_pointer_n(
        &vars[8],
        "box_d",
        "alloc::boxed::Box<i32, alloc::alloc::Global>",
    );
    let deref = read_single_var(&debugger, "*box_d");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_struct_n(&vars[9], "f", "Foo", |i, member| match i {
        0 => assert_member(member, "bar", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
        1 => assert_member(member, "baz", |val| {
            assert_array(val, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("2 items expected"),
            })
        }),
        2 => {
            assert_member(member, "foo", |val| assert_pointer(val, "&i32"));
            let deref = read_single_var(&debugger, "*f.foo");
            assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));
        }
        _ => panic!("3 members expected"),
    });

    assert_pointer_n(&vars[10], "ref_f", "&vars::references::Foo");
    let deref = read_single_var(&debugger, "*ref_f");
    assert_struct(&deref, "Foo", |i, member| match i {
        0 => assert_member(member, "bar", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
        1 => assert_member(member, "baz", |val| {
            assert_array(val, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("2 items expected"),
            })
        }),
        2 => {
            assert_member(member, "foo", |val| assert_pointer(val, "&i32"));
            let deref = read_single_var(&debugger, "*(*ref_f).foo");
            assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 126).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(126));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar_n(&vars[0], "a_alias", "i32", Some(SupportedScalar::I32(1)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_type_parameters() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 135).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(135));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct_n(&vars[0], "a", "Foo<i32>", |i, member| match i {
        0 => assert_member(member, "bar", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 151).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(151));

    let vars = debugger.read_local_variables().unwrap();
    assert_vec_n(
        &vars[0],
        "vec1",
        "Vec<i32, alloc::alloc::Global>",
        3,
        |buf| {
            assert_array(buf, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            })
        },
    );
    assert_vec_n(
        &vars[1],
        "vec2",
        "Vec<vars::vec_and_slice_types::Foo, alloc::alloc::Global>",
        2,
        |buf| {
            assert_array(buf, "[Foo]", |i, item| match i {
                0 => assert_struct(item, "Foo", |i, member| match i {
                    0 => assert_member(member, "foo", |val| {
                        assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
                    }),
                    _ => panic!("1 members expected"),
                }),
                1 => assert_struct(item, "Foo", |i, member| match i {
                    0 => assert_member(member, "foo", |val| {
                        assert_scalar(val, "i32", Some(SupportedScalar::I32(2)))
                    }),
                    _ => panic!("1 members expected"),
                }),
                _ => panic!("2 items expected"),
            })
        },
    );
    assert_vec_n(
        &vars[2],
        "vec3",
        "Vec<alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        2,
        |buf| {
            assert_array(buf, "[Vec<i32, alloc::alloc::Global>]", |i, item| match i {
                0 => assert_vec(item, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf, "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    })
                }),
                1 => assert_vec(item, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf, "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    })
                }),
                _ => panic!("2 items expected"),
            })
        },
    );

    assert_pointer_n(&vars[3], "slice1", "&[i32; 3]");
    let deref = read_single_var(&debugger, "*slice1");
    assert_array(&deref, "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("3 items expected"),
    });

    assert_pointer_n(&vars[4], "slice2", "&[&[i32; 3]; 2]");
    let deref = read_single_var(&debugger, "*slice2");
    assert_array(&deref, "[&[i32; 3]]", |i, item| match i {
        0 => {
            assert_pointer(item, "&[i32; 3]");
            let deref = read_single_var(&debugger, "*(*slice2)[0]");
            assert_array(&deref, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            });
        }
        1 => {
            assert_pointer(item, "&[i32; 3]");
            let deref = read_single_var(&debugger, "*(*slice2)[1]");
            assert_array(&deref, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 159).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(159));

    let vars = debugger.read_local_variables().unwrap();
    assert_string_n(&vars[0], "s1", "hello world");
    assert_str_n(&vars[1], "s2", "hello world");
    assert_str_n(&vars[2], "s3", "hello world");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_static_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 168).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(168));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name("GLOB_1", false)))
        .unwrap();
    assert_eq!(vars.len(), 1);
    assert_str_n(&vars[0], "vars::GLOB_1", "glob_1");

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name("GLOB_2", false)))
        .unwrap();
    assert_eq!(vars.len(), 1);
    assert_scalar_n(
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
fn test_read_only_local_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 168).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(168));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name("GLOB_1", true)))
        .unwrap();
    assert_eq!(vars.len(), 0);

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_static_variables_different_modules() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 179).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(179));

    let mut vars = debugger
        .read_variable(DQE::Variable(Selector::by_name("GLOB_3", false)))
        .unwrap();
    assert_eq!(vars.len(), 2);
    vars.sort_by(|v1, v2| v1.r#type().cmp(v2.r#type()));

    assert_str_n(&vars[0], "vars::ns_1::GLOB_3", "glob_3");
    assert_scalar_n(
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 194).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(194));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_1",
            false,
        )))
        .unwrap();
    assert_init_tls_n(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>", |inner| {
        assert_cell(inner, "Cell<i32>", |value| {
            assert_scalar(value, "i32", Some(SupportedScalar::I32(2)))
        })
    });

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_2",
            false,
        )))
        .unwrap();
    assert_init_tls_n(&vars[0], "THREAD_LOCAL_VAR_2", "Cell<&str>", |inner| {
        assert_cell(inner, "Cell<&str>", |value| assert_str(value, "2"))
    });

    // assert uninit tls variables
    debugger.set_breakpoint_at_line("vars.rs", 199).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(199));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_1",
            false,
        )))
        .unwrap();
    let rust_version = rust_version(VARS_APP).unwrap();
    version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 79, u32::MAX) => {
                assert_uninit_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>");
            },
            (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => {
                assert!(vars.is_empty());
            },
    );

    // assert tls variables changes in another thread
    debugger.set_breakpoint_at_line("vars.rs", 203).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(203));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_1",
            false,
        )))
        .unwrap();
    assert_init_tls_n(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>", |inner| {
        assert_cell(inner, "Cell<i32>", |value| {
            assert_scalar(value, "i32", Some(SupportedScalar::I32(1)))
        })
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_tls_const_variables() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 538).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(538));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name(
            "CONSTANT_THREAD_LOCAL",
            false,
        )))
        .unwrap();
    assert_init_tls_n(&vars[0], "CONSTANT_THREAD_LOCAL", "i32", |value| {
        assert_scalar(value, "i32", Some(SupportedScalar::I32(1337)))
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_closures() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 223).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(223));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct_n(&vars[0], "inc", "{closure_env#0}", |_, _| {
        panic!("no members expected")
    });
    assert_struct_n(&vars[1], "inc_mut", "{closure_env#1}", |_, _| {
        panic!("no members expected")
    });
    assert_struct_n(&vars[3], "closure", "{closure_env#2}", |_, member| {
        assert_member(member, "outer", |val| assert_string(val, "outer val"))
    });
    let rust_version = rust_version(VARS_APP).unwrap();
    assert_struct_n(
        &vars[7],
        "trait_once",
        "alloc::boxed::Box<dyn core::ops::function::FnOnce<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "pointer", |val| {
                    assert_pointer(val, "*dyn core::ops::function::FnOnce<(), Output=()>")
                });
                let deref = read_single_var(&debugger, "*trait_once.pointer");
                assert_struct(
                    &deref,
                    "dyn core::ops::function::FnOnce<(), Output=()>",
                    |_, _| {},
                );
            }
            1 => {
                let exp_type = if rust_version >= Version((1, 80, 0)) {
                    "&[usize; 4]"
                } else {
                    "&[usize; 3]"
                };
                assert_member(member, "vtable", |val| assert_pointer(val, exp_type));
                let deref = read_single_var(&debugger, "*trait_once.vtable");
                assert_array(&deref, "[usize]", |i, _| |_, _| {});
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_struct_n(
        &vars[8],
        "trait_mut",
        "alloc::boxed::Box<dyn core::ops::function::FnMut<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "pointer", |val| {
                    assert_pointer(val, "*dyn core::ops::function::FnMut<(), Output=()>")
                });
                let deref = read_single_var(&debugger, "*trait_mut.pointer");
                assert_struct(
                    &deref,
                    "dyn core::ops::function::FnMut<(), Output=()>",
                    |_, _| {},
                );
            }
            1 => {
                let exp_type = if rust_version >= Version((1, 80, 0)) {
                    "&[usize; 5]"
                } else {
                    "&[usize; 3]"
                };
                assert_member(member, "vtable", |val| assert_pointer(val, exp_type));
                let deref = read_single_var(&debugger, "*trait_mut.vtable");
                assert_array(&deref, "[usize]", |_, _| {});
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_struct_n(
        &vars[9],
        "trait_fn",
        "alloc::boxed::Box<dyn core::ops::function::Fn<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "pointer", |val| {
                    assert_pointer(val, "*dyn core::ops::function::Fn<(), Output=()>")
                });
                let deref = read_single_var(&debugger, "*trait_fn.pointer");
                assert_struct(
                    &deref,
                    "dyn core::ops::function::Fn<(), Output=()>",
                    |_, _| {},
                );
            }
            1 => {
                let exp_type = if rust_version >= Version((1, 80, 0)) {
                    "&[usize; 6]"
                } else {
                    "&[usize; 3]"
                };
                assert_member(member, "vtable", |val| assert_pointer(val, "&[usize; 3]"));
                let deref = read_single_var(&debugger, "*trait_fn.vtable");
                assert_array(&deref, "[usize]", |_, _| {});
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_pointer_n(&vars[10], "fn_ptr", "fn() -> u8");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_arguments() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 232).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(232));

    let args = debugger
        .read_argument(DQE::Variable(Selector::Any))
        .unwrap();
    assert_scalar_n(&args[0], "by_val", "i32", Some(SupportedScalar::I32(1)));
    assert_pointer_n(&args[1], "by_ref", "&i32");
    let deref = read_single_arg(&debugger, "*by_ref");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    assert_vec_n(&args[2], "vec", "Vec<u8, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[u8]", |i, item| match i {
            0 => assert_scalar(item, "u8", Some(SupportedScalar::U8(3))),
            1 => assert_scalar(item, "u8", Some(SupportedScalar::U8(4))),
            2 => assert_scalar(item, "u8", Some(SupportedScalar::U8(5))),
            _ => panic!("3 items expected"),
        })
    });
    assert_struct_n(
        &args[3],
        "box_arr",
        "alloc::boxed::Box<[u8], alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "data_ptr", |val| assert_pointer(val, "*u8"));
                let deref = read_single_arg(&debugger, "*box_arr.data_ptr");
                assert_scalar(&deref, "u8", Some(SupportedScalar::U8(6)));
            }
            1 => assert_member(member, "length", |val| {
                assert_scalar(val, "usize", Some(SupportedScalar::Usize(3)))
            }),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 244).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(244));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct_n(&vars[0], "union", "Union1", |i, member| match i {
        0 => assert_member(member, "f1", |val| {
            assert_scalar(val, "f32", Some(SupportedScalar::F32(1.1)))
        }),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 290).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(290));

    let rust_version = rust_version(VARS_APP).unwrap();
    let hash_map_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<bool, i64, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<bool, i64, std::hash::random::RandomState>",
    ).unwrap();

    let vars = debugger.read_local_variables().unwrap();
    assert_hashmap_n(&vars[0], "hm1", hash_map_type, |items| {
        assert_eq!(items.len(), 2);
        assert_scalar(&items[0].0, "bool", Some(SupportedScalar::Bool(false)));
        assert_scalar(&items[0].1, "i64", Some(SupportedScalar::I64(5)));
        assert_scalar(&items[1].0, "bool", Some(SupportedScalar::Bool(true)));
        assert_scalar(&items[1].1, "i64", Some(SupportedScalar::I64(3)));
    });

    let hash_map_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<&str, alloc::vec::Vec<i32, alloc::alloc::Global>, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<&str, alloc::vec::Vec<i32, alloc::alloc::Global>, std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap_n(&vars[1], "hm2", hash_map_type, |items| {
        assert_eq!(items.len(), 2);
        assert_str(&items[0].0, "abc");
        assert_vec(&items[0].1, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
            assert_array(buf, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            })
        });
        assert_str(&items[1].0, "efg");
        assert_vec(&items[1].1, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
            assert_array(buf, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(11))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(12))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(13))),
                _ => panic!("3 items expected"),
            })
        });
    });

    let hash_map_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<i32, i32, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<i32, i32, std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap_n(&vars[2], "hm3", hash_map_type, |items| {
        assert_eq!(items.len(), 100);

        let mut exp_items = (0..100).collect::<Vec<_>>();
        exp_items.sort_by_key(|i1| i1.to_string());

        for i in 0..100 {
            assert_scalar(&items[i].0, "i32", Some(SupportedScalar::I32(exp_items[i])));
        }
        for i in 0..100 {
            assert_scalar(&items[i].1, "i32", Some(SupportedScalar::I32(exp_items[i])));
        }
    });

    let hash_map_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<alloc::string::String, std::collections::hash::map::HashMap<i32, i32, std::collections::hash::map::RandomState>, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<alloc::string::String, std::collections::hash::map::HashMap<i32, i32, std::hash::random::RandomState>, std::hash::random::RandomState>",
    ).unwrap();
    let inner_hash_map_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<i32, i32, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<i32, i32, std::hash::random::RandomState>",
    ).unwrap();

    assert_hashmap_n(&vars[3], "hm4", hash_map_type, |items| {
        assert_eq!(items.len(), 2);
        assert_string(&items[0].0, "1");
        assert_hashmap(&items[0].1, inner_hash_map_type, |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[0].1, "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[1].0, "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[1].1, "i32", Some(SupportedScalar::I32(2)));
        });

        assert_string(&items[1].0, "3");
        assert_hashmap(&items[1].1, inner_hash_map_type, |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "i32", Some(SupportedScalar::I32(3)));
            assert_scalar(&items[0].1, "i32", Some(SupportedScalar::I32(3)));
            assert_scalar(&items[1].0, "i32", Some(SupportedScalar::I32(4)));
            assert_scalar(&items[1].1, "i32", Some(SupportedScalar::I32(4)));
        });
    });

    let make_idx_dqe = |var: &str, literal| {
        DQE::Index(DQE::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by bool key
    let dqe = make_idx_dqe("hm1", Literal::Bool(true));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "i64", Some(SupportedScalar::I64(3)));

    // get by string key
    let dqe = make_idx_dqe("hm2", Literal::String("efg".to_string()));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_vec(val, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(11))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(12))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(13))),
            _ => panic!("3 items expected"),
        })
    });

    // get by int key
    let dqe = make_idx_dqe("hm3", Literal::Int(99));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "i32", Some(SupportedScalar::I32(99)));

    // get by pointer key
    let key = debugger
        .read_variable(DQE::Variable(Selector::by_name("b", true)))
        .unwrap();
    assert_eq!(key.len(), 1);
    let VariableIR::Pointer(ptr) = &key[0] else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hm5", Literal::Address(ptr_val));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_str(val, "b");

    // get by complex object
    let dqe = make_idx_dqe(
        "hm6",
        Literal::AssocArray(HashMap::from([
            (
                "field_1".to_string(),
                LiteralOrWildcard::Literal(Literal::Int(1)),
            ),
            (
                "field_2".to_string(),
                LiteralOrWildcard::Literal(Literal::Array(Box::new([
                    LiteralOrWildcard::Literal(Literal::String("a".to_string())),
                    LiteralOrWildcard::Wildcard,
                ]))),
            ),
            (
                "field_3".to_string(),
                LiteralOrWildcard::Literal(Literal::EnumVariant(
                    "Some".to_string(),
                    Some(Box::new(Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::Bool(true)),
                    ])))),
                )),
            ),
        ])),
    );
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "i32", Some(SupportedScalar::I32(1)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_hashset() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 307).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(307));

    let rust_version = rust_version(VARS_APP).unwrap();
    let hashset_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashSet<i32, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashSet<i32, std::hash::random::RandomState>",
    ).unwrap();

    let vars = debugger.read_local_variables().unwrap();
    assert_hashset(&vars[0], "hs1", hashset_type, |items| {
        assert_eq!(items.len(), 4);
        assert_scalar(&items[0], "i32", Some(SupportedScalar::I32(1)));
        assert_scalar(&items[1], "i32", Some(SupportedScalar::I32(2)));
        assert_scalar(&items[2], "i32", Some(SupportedScalar::I32(3)));
        assert_scalar(&items[3], "i32", Some(SupportedScalar::I32(4)));
    });
    assert_hashset(&vars[1], "hs2", hashset_type, |items| {
        assert_eq!(items.len(), 100);
        let mut exp_items = (0..100).collect::<Vec<_>>();
        exp_items.sort_by_key(|i1| i1.to_string());

        for i in 0..100 {
            assert_scalar(&items[i], "i32", Some(SupportedScalar::I32(exp_items[i])));
        }
    });

    let hashset_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashSet<alloc::vec::Vec<i32, alloc::alloc::Global>, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashSet<alloc::vec::Vec<i32, alloc::alloc::Global>, std::hash::random::RandomState>",
    ).unwrap();
    assert_hashset(&vars[2], "hs3", hashset_type, |items| {
        assert_eq!(items.len(), 1);
        assert_vec(&items[0], "Vec<i32, alloc::alloc::Global>", 2, |buf| {
            assert_array(buf, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("2 items expected"),
            })
        });
    });

    let make_idx_dqe = |var: &str, literal| {
        DQE::Index(DQE::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by int key
    let dqe = make_idx_dqe("hs1", Literal::Int(2));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs1", Literal::Int(5));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(false)));

    // get by pointer key
    let key = debugger
        .read_variable(DQE::Variable(Selector::by_name("b", true)))
        .unwrap();
    assert_eq!(key.len(), 1);
    let VariableIR::Pointer(ptr) = &key[0] else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hs4", Literal::Address(ptr_val));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs4", Literal::Address(0));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(false)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_circular_ref_types() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 334).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(334));

    let vars = debugger.read_local_variables().unwrap();
    assert_rc_n(
        &vars[0],
        "a_circ",
        "Rc<vars::circular::List, alloc::alloc::Global>",
    );
    assert_rc_n(
        &vars[1],
        "b_circ",
        "Rc<vars::circular::List, alloc::alloc::Global>",
    );

    let deref = read_single_var(&debugger, "*a_circ");
    let rust_version = rust_version(VARS_APP).unwrap();
    let deref_type = version_switch!(
        rust_version,
        .. (1 . 84) => "RcBox<vars::circular::List>",
        (1 . 84) .. => "RcInner<vars::circular::List>",
    )
    .unwrap();
    assert_struct(&deref, deref_type, |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(1)))
            })
        }),
        2 => {
            assert_member(member, "value", |val| {
                assert_rust_enum(val, "List", |enum_member| {
                    assert_struct(enum_member, "Cons", |i, cons_member| match i {
                        0 => assert_member(cons_member, "__0", |val| {
                            assert_scalar(val, "i32", Some(SupportedScalar::I32(5)))
                        }),
                        1 => assert_member(cons_member, "__1", |val| {
                            assert_refcell(
                                    val,
                            "RefCell<alloc::rc::Rc<vars::circular::List, alloc::alloc::Global>>",
                            0,
                            |inner| {
                                assert_rc(
                                    inner,
                                    "Rc<vars::circular::List, alloc::alloc::Global>",
                                )
                            },
                        )
                        }),
                        _ => panic!("2 members expected"),
                    });
                })
            });
        }
        _ => panic!("3 members expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_lexical_blocks() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 340).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(340));

    let vars = debugger.read_local_variables().unwrap();
    // WAITFORFIX: https://github.com/rust-lang/rust/issues/113819
    // expected:     assert_eq!(vars.len(), 1);
    // through this bug there is uninitialized variable here
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name().unwrap(), "alpha");

    debugger.set_breakpoint_at_line("vars.rs", 342).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(342));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name().unwrap(), "alpha");
    assert_eq!(vars[1].name().unwrap(), "beta");

    debugger.set_breakpoint_at_line("vars.rs", 343).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(343));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 3);
    assert_eq!(vars[0].name().unwrap(), "alpha");
    assert_eq!(vars[1].name().unwrap(), "beta");
    assert_eq!(vars[2].name().unwrap(), "gama");

    debugger.set_breakpoint_at_line("vars.rs", 349).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(349));

    let vars = debugger.read_local_variables().unwrap();
    assert_eq!(vars.len(), 2);
    assert_eq!(vars[0].name().unwrap(), "alpha");
    assert_eq!(vars[1].name().unwrap(), "delta");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_btree_map() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 396).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(396));

    let vars = debugger.read_local_variables().unwrap();
    assert_btree_map_n(
        &vars[0],
        "hm1",
        "BTreeMap<bool, i64, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "bool", Some(SupportedScalar::Bool(false)));
            assert_scalar(&items[0].1, "i64", Some(SupportedScalar::I64(5)));
            assert_scalar(&items[1].0, "bool", Some(SupportedScalar::Bool(true)));
            assert_scalar(&items[1].1, "i64", Some(SupportedScalar::I64(3)));
        },
    );
    assert_btree_map_n(
        &vars[1],
        "hm2",
        "BTreeMap<&str, alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_str(&items[0].0, "abc");
            assert_vec(&items[0].1, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                assert_array(buf, "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                    2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                    _ => panic!("3 items expected"),
                })
            });
            assert_str(&items[1].0, "efg");
            assert_vec(&items[1].1, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                assert_array(buf, "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(11))),
                    1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(12))),
                    2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(13))),
                    _ => panic!("3 items expected"),
                })
            });
        },
    );
    assert_btree_map_n(
        &vars[2],
        "hm3",
        "BTreeMap<i32, i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 100);

            let exp_items = (0..100).collect::<Vec<_>>();

            for i in 0..100 {
                assert_scalar(&items[i].0, "i32", Some(SupportedScalar::I32(exp_items[i])));
            }
            for i in 0..100 {
                assert_scalar(&items[i].1, "i32", Some(SupportedScalar::I32(exp_items[i])));
            }
        },
    );
    assert_btree_map_n(
        &vars[3],
        "hm4",
        "BTreeMap<alloc::string::String, alloc::collections::btree::map::BTreeMap<i32, i32, alloc::alloc::Global>, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_string(&items[0].0,  "1");
            assert_btree_map(
                &items[0].1,
                "BTreeMap<i32, i32, alloc::alloc::Global>",
                |items| {
                    assert_eq!(items.len(), 2);
                    assert_scalar(&items[0].0,  "i32", Some(SupportedScalar::I32(1)));
                    assert_scalar(&items[0].1,  "i32", Some(SupportedScalar::I32(1)));
                    assert_scalar(&items[1].0,  "i32", Some(SupportedScalar::I32(2)));
                    assert_scalar(&items[1].1,  "i32", Some(SupportedScalar::I32(2)));
                },
            );

            assert_string(
                &items[1].0,
                "3",
            );
            assert_btree_map(
                &items[1].1,
                "BTreeMap<i32, i32, alloc::alloc::Global>",
                |items| {
                    assert_eq!(items.len(), 2);
                    assert_scalar(&items[0].0,  "i32", Some(SupportedScalar::I32(3)));
                    assert_scalar(&items[0].1,  "i32", Some(SupportedScalar::I32(3)));
                    assert_scalar(&items[1].0,  "i32", Some(SupportedScalar::I32(4)));
                    assert_scalar(&items[1].1,  "i32", Some(SupportedScalar::I32(4)));
                },
            );
        },
    );

    let make_idx_dqe = |var: &str, literal| {
        DQE::Index(DQE::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by bool key
    let dqe = make_idx_dqe("hm1", Literal::Bool(true));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "i64", Some(SupportedScalar::I64(3)));

    // get by string key
    let dqe = make_idx_dqe("hm2", Literal::String("efg".to_string()));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_vec(val, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(11))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(12))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(13))),
            _ => panic!("3 items expected"),
        })
    });

    // get by int key
    let dqe = make_idx_dqe("hm3", Literal::Int(99));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "i32", Some(SupportedScalar::I32(99)));

    // get by pointer key
    let key = debugger
        .read_variable(DQE::Variable(Selector::by_name("b", true)))
        .unwrap();
    assert_eq!(key.len(), 1);
    let VariableIR::Pointer(ptr) = &key[0] else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hm5", Literal::Address(ptr_val));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_str(val, "b");

    // get by complex object
    let dqe = make_idx_dqe(
        "hm6",
        Literal::AssocArray(HashMap::from([
            ("field_1".to_string(), LiteralOrWildcard::Wildcard),
            (
                "field_2".to_string(),
                LiteralOrWildcard::Literal(Literal::Array(Box::new([
                    LiteralOrWildcard::Literal(Literal::String("c".to_string())),
                    LiteralOrWildcard::Wildcard,
                    LiteralOrWildcard::Wildcard,
                ]))),
            ),
            (
                "field_3".to_string(),
                LiteralOrWildcard::Literal(Literal::EnumVariant("None".to_string(), None)),
            ),
        ])),
    );
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "i32", Some(SupportedScalar::I32(2)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_btree_set() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 413).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(413));

    let vars = debugger.read_local_variables().unwrap();
    assert_btree_set(
        &vars[0],
        "hs1",
        "BTreeSet<i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 4);
            assert_scalar(&items[0], "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[1], "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[2], "i32", Some(SupportedScalar::I32(3)));
            assert_scalar(&items[3], "i32", Some(SupportedScalar::I32(4)));
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
                assert_scalar(&items[i], "i32", Some(SupportedScalar::I32(exp_items[i])));
            }
        },
    );
    assert_btree_set(
        &vars[2],
        "hs3",
        "BTreeSet<alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_vec(&items[0], "Vec<i32, alloc::alloc::Global>", 2, |buf| {
                assert_array(buf, "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                    _ => panic!("2 items expected"),
                })
            });
        },
    );

    let make_idx_dqe = |var: &str, literal| {
        DQE::Index(DQE::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by int key
    let dqe = make_idx_dqe("hs1", Literal::Int(2));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs1", Literal::Int(5));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(false)));

    // get by pointer key
    let key = debugger
        .read_variable(DQE::Variable(Selector::by_name("b", true)))
        .unwrap();
    assert_eq!(key.len(), 1);
    let VariableIR::Pointer(ptr) = &key[0] else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hs4", Literal::Address(ptr_val));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs4", Literal::Address(0));
    let val = debugger.read_variable(dqe).unwrap();
    assert_eq!(val.len(), 1);
    let val = &val[0];
    assert_scalar(val, "bool", Some(SupportedScalar::Bool(false)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_vec_deque() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 431).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(431));

    let vars = debugger.read_local_variables().unwrap();
    assert_vec_deque_n(
        &vars[0],
        "vd1",
        "VecDeque<i32, alloc::alloc::Global>",
        8,
        |buf| {
            assert_array(buf, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(9))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(10))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(0))),
                3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("5 items expected"),
            })
        },
    );

    assert_vec_deque_n(
        &vars[1],
        "vd2",
        "VecDeque<alloc::collections::vec_deque::VecDeque<i32, alloc::alloc::Global>, alloc::alloc::Global>",
        4,
        |buf| {
            assert_array(buf, "[VecDeque<i32, alloc::alloc::Global>]", |i, item| match i {
                0 => assert_vec_deque(item,  "VecDeque<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf,  "[i32]", |i, item| match i {
                        0 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(-2))),
                        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
                        2 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(0))),
                        _ => panic!("3 items expected"),
                    })
                }),
                1 => assert_vec_deque(item,  "VecDeque<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf,  "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    })
                }),
                2 => assert_vec_deque(item,  "VecDeque<i32, alloc::alloc::Global>", 3, |buf| {
                    assert_array(buf,  "[i32]", |i, item| match i {
                        0 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(4))),
                        1 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(5))),
                        2 => assert_scalar(item,  "i32", Some(SupportedScalar::I32(6))),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 441).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(441));

    let vars = debugger.read_local_variables().unwrap();
    assert_struct_n(&vars[0], "int32_atomic", "AtomicI32", |i, member| match i {
        0 => assert_member(member, "v", |val| {
            assert_struct(val, "UnsafeCell<i32>", |i, member| match i {
                0 => assert_member(member, "value", |val| {
                    assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
                }),
                _ => panic!("1 members expected"),
            })
        }),
        _ => panic!("1 members expected"),
    });

    assert_struct_n(
        &vars[2],
        "int32_atomic_ptr",
        "AtomicPtr<i32>",
        |i, member| match i {
            0 => assert_member(member, "p", |val| {
                assert_struct(val, "UnsafeCell<*mut i32>", |i, member| match i {
                    0 => assert_member(member, "value", |val| assert_pointer(val, "*mut i32")),
                    _ => panic!("1 members expected"),
                })
            }),
            _ => panic!("1 members expected"),
        },
    );

    let deref = read_single_var(&debugger, "*int32_atomic_ptr.p.value");
    assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_cell() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 453).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(453));

    let vars = debugger.read_local_variables().unwrap();
    assert_cell_n(&vars[0], "a_cell", "Cell<i32>", |value| {
        assert_scalar(value, "i32", Some(SupportedScalar::I32(1)))
    });

    assert_refcell_n(
        &vars[1],
        "b_refcell",
        "RefCell<alloc::vec::Vec<i32, alloc::alloc::Global>>",
        2,
        |value| {
            assert_vec(value, "Vec<i32, alloc::alloc::Global>", 3, |buf| {
                assert_array(buf, "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                    2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();
    let rust_version: Version = rust_version(VARS_APP).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 475).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(475));

    let vars = debugger.read_local_variables().unwrap();
    assert_rc_n(&vars[0], "rc0", "Rc<i32, alloc::alloc::Global>");
    let deref = read_single_var(&debugger, "*rc0");
    let rust_version: bugstalker::version::RustVersion = rust_version(VARS_APP).unwrap();
    let deref_type = version_switch!(
        rust_version,
        .. (1 . 84) => "RcBox<vars::circular::List>",
        (1 . 84) .. => "RcInner<vars::circular::List>",
    )
    .unwrap();
    assert_struct(&deref, deref_type, |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        2 => assert_member(member, "value", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
        _ => panic!("3 members expected"),
    });
    assert_rc_n(&vars[1], "rc1", "Rc<i32, alloc::alloc::Global>");
    let deref = read_single_var(&debugger, "*rc1");
    assert_struct(&deref, deref_type, |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        2 => assert_member(member, "value", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
        _ => panic!("3 members expected"),
    });
    assert_rc_n(&vars[2], "weak_rc2", "Weak<i32, alloc::alloc::Global>");
    let deref = read_single_var(&debugger, "*weak_rc2");
    assert_struct(&deref, deref_type, |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_cell(val, "Cell<usize>", |inner| {
                assert_scalar(inner, "usize", Some(SupportedScalar::Usize(2)))
            })
        }),
        2 => assert_member(member, "value", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
        _ => panic!("3 members expected"),
    });

    assert_arc_n(&vars[3], "arc0", "Arc<i32, alloc::alloc::Global>");
    let deref = read_single_var(&debugger, "*arc0");
    assert_struct(&deref, "ArcInner<i32>", |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_struct(val, "AtomicUsize", |i, member| match i {
                0 => assert_member(member, "v", |val| {
                    assert_struct(val, "UnsafeCell<usize>", |_, member| {
                        assert_member(member, "value", |val| {
                            assert_scalar(val, "usize", Some(SupportedScalar::Usize(2)))
                        })
                    })
                }),
                _ => panic!("1 member expected"),
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_struct(val, "AtomicUsize", |i, member| match i {
                0 => assert_member(member, "v", |val| {
                    assert_struct(val, "UnsafeCell<usize>", |_, member| {
                        assert_member(member, "value", |val| {
                            assert_scalar(val, "usize", Some(SupportedScalar::Usize(2)))
                        })
                    })
                }),
                _ => panic!("1 member expected"),
            })
        }),
        2 => assert_member(member, "data", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(2)))
        }),
        _ => panic!("3 members expected"),
    });
    assert_arc_n(&vars[4], "arc1", "Arc<i32, alloc::alloc::Global>");
    let deref = read_single_var(&debugger, "*arc1");
    assert_struct(&deref, "ArcInner<i32>", |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_struct(val, "AtomicUsize", |i, member| match i {
                0 => assert_member(member, "v", |val| {
                    assert_struct(val, "UnsafeCell<usize>", |_, member| {
                        assert_member(member, "value", |val| {
                            assert_scalar(val, "usize", Some(SupportedScalar::Usize(2)))
                        })
                    })
                }),
                _ => panic!("1 member expected"),
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_struct(val, "AtomicUsize", |i, member| match i {
                0 => assert_member(member, "v", |val| {
                    assert_struct(val, "UnsafeCell<usize>", |_, member| {
                        assert_member(member, "value", |val| {
                            assert_scalar(val, "usize", Some(SupportedScalar::Usize(2)))
                        })
                    })
                }),
                _ => panic!("1 member expected"),
            })
        }),
        2 => assert_member(member, "data", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(2)))
        }),
        _ => panic!("3 members expected"),
    });
    assert_arc_n(&vars[5], "weak_arc2", "Weak<i32, alloc::alloc::Global>");
    let deref = read_single_var(&debugger, "*weak_arc2");
    assert_struct(&deref, "ArcInner<i32>", |i, member| match i {
        0 => assert_member(member, "strong", |val| {
            assert_struct(val, "AtomicUsize", |i, member| match i {
                0 => assert_member(member, "v", |val| {
                    assert_struct(val, "UnsafeCell<usize>", |_, member| {
                        assert_member(member, "value", |val| {
                            assert_scalar(val, "usize", Some(SupportedScalar::Usize(2)))
                        })
                    })
                }),
                _ => panic!("1 member expected"),
            })
        }),
        1 => assert_member(member, "weak", |val| {
            assert_struct(val, "AtomicUsize", |i, member| match i {
                0 => assert_member(member, "v", |val| {
                    assert_struct(val, "UnsafeCell<usize>", |_, member| {
                        assert_member(member, "value", |val| {
                            assert_scalar(val, "usize", Some(SupportedScalar::Usize(2)))
                        })
                    })
                }),
                _ => panic!("1 member expected"),
            })
        }),
        2 => assert_member(member, "data", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(2)))
        }),
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 496).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(496));

    let vars = debugger.read_local_variables().unwrap();

    assert_pointer_n(&vars[0], "ptr_zst", "&()");
    let deref = read_single_var(&debugger, "*ptr_zst");
    assert_scalar(&deref, "()", Some(SupportedScalar::Empty()));

    assert_array_n(&vars[1], "array_zst", "[()]", |i, item| match i {
        0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        _ => panic!("2 members expected"),
    });

    assert_vec_n(
        &vars[2],
        "vec_zst",
        "Vec<(), alloc::alloc::Global>",
        0,
        |buf| {
            assert_array(buf, "[()]", |i, item| match i {
                0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                2 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                _ => panic!("3 members expected"),
            })
        },
    );

    assert_vec_n(
        &vars[2],
        "vec_zst",
        "Vec<(), alloc::alloc::Global>",
        0,
        |buf| {
            assert_array(buf, "[()]", |i, item| match i {
                0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                2 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                _ => panic!("3 members expected"),
            })
        },
    );

    assert_pointer_n(&vars[3], "slice_zst", "&[(); 4]");
    let deref = read_single_var(&debugger, "*slice_zst");
    assert_array(&deref, "[()]", |i, item| match i {
        0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        2 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        3 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        _ => panic!("4 members expected"),
    });

    assert_struct_n(&vars[4], "struct_zst", "StructZst", |i, member| match i {
        0 => assert_member(member, "__0", |val| {
            assert_scalar(val, "()", Some(SupportedScalar::Empty()))
        }),
        _ => panic!("1 member expected"),
    });

    assert_rust_enum_n(&vars[5], "enum_zst", "Option<()>", |member| {
        assert_struct(member, "Some", |i, member| match i {
            0 => assert_member(member, "__0", |val| {
                assert_scalar(val, "()", Some(SupportedScalar::Empty()))
            }),
            _ => panic!("1 member expected"),
        })
    });

    assert_vec_deque_n(
        &vars[6],
        "vecdeque_zst",
        "VecDeque<(), alloc::alloc::Global>",
        0,
        |buf| {
            assert_array(buf, "[()]", |i, item| match i {
                0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                2 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                3 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                4 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
                _ => panic!("5 members expected"),
            })
        },
    );

    let rust_version = rust_version(VARS_APP).unwrap();
    let hashmap_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<(), i32, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<(), i32, std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap_n(&vars[7], "hash_map_zst_key", hashmap_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
        assert_scalar(&items[0].1, "i32", Some(SupportedScalar::I32(1)));
    });

    let hashmap_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<i32, (), std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<i32, (), std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap_n(&vars[8], "hash_map_zst_val", hashmap_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0].0, "i32", Some(SupportedScalar::I32(1)));
        assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
    });

    let hashmap_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<(), (), std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<(), (), std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap_n(&vars[9], "hash_map_zst", hashmap_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
        assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
    });

    let hashset_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashSet<(), std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashSet<(), std::hash::random::RandomState>",
    ).unwrap();
    assert_hashset(&vars[10], "hash_set_zst", hashset_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0], "()", Some(SupportedScalar::Empty()));
    });

    assert_btree_map_n(
        &vars[11],
        "btree_map_zst_key",
        "BTreeMap<(), i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "i32", Some(SupportedScalar::I32(1)));
        },
    );
    assert_btree_map_n(
        &vars[12],
        "btree_map_zst_val",
        "BTreeMap<i32, (), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[1].0, "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[1].1, "()", Some(SupportedScalar::Empty()));
        },
    );
    assert_btree_map_n(
        &vars[13],
        "btree_map_zst",
        "BTreeMap<(), (), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
        },
    );
    assert_btree_set(
        &vars[14],
        "btree_set_zst",
        "BTreeSet<(), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0], "()", Some(SupportedScalar::Empty()));
        },
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_static_in_fn_variable() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    // brkpt in function where static is declared
    debugger.set_breakpoint_at_line("vars.rs", 504).unwrap();
    // brkpt outside function where static is declared
    debugger.set_breakpoint_at_line("vars.rs", 570).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(504));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name("INNER_STATIC", false)))
        .unwrap();
    assert_scalar_n(
        &vars[0],
        "vars::inner_static::INNER_STATIC",
        "u32",
        Some(SupportedScalar::U32(1)),
    );

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(570));

    let vars = debugger
        .read_variable(DQE::Variable(Selector::by_name("INNER_STATIC", false)))
        .unwrap();
    assert_scalar_n(
        &vars[0],
        "vars::inner_static::INNER_STATIC",
        "u32",
        Some(SupportedScalar::U32(1)),
    );

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_slice_operator() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 61).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(61));

    let vars = debugger
        .read_variable(DQE::Slice(
            DQE::Variable(Selector::by_name("arr_1", true)).boxed(),
            None,
            None,
        ))
        .unwrap();
    assert_array_n(&vars[0], "arr_1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("5 items expected"),
    });

    let vars = debugger
        .read_variable(DQE::Slice(
            DQE::Variable(Selector::by_name("arr_1", true)).boxed(),
            Some(3),
            None,
        ))
        .unwrap();
    assert_array_n(&vars[0], "arr_1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("2 items expected"),
    });

    let vars = debugger
        .read_variable(DQE::Slice(
            DQE::Variable(Selector::by_name("arr_1", true)).boxed(),
            None,
            Some(2),
        ))
        .unwrap();
    assert_array_n(&vars[0], "arr_1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        _ => panic!("2 items expected"),
    });

    let vars = debugger
        .read_variable(DQE::Slice(
            DQE::Variable(Selector::by_name("arr_1", true)).boxed(),
            Some(1),
            Some(4),
        ))
        .unwrap();
    assert_array_n(&vars[0], "arr_1", "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        _ => panic!("3 items expected"),
    });

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_cast_pointers() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 119).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(119));

    let vars = debugger.read_local_variables().unwrap();
    assert_scalar_n(&vars[0], "a", "i32", Some(SupportedScalar::I32(2)));

    let VariableIR::Pointer(pointer) = &vars[1] else {
        panic!("expect a pointer");
    };

    let raw_ptr = pointer.value.unwrap();

    let var = debugger
        .read_variable(DQE::Deref(
            DQE::PtrCast(raw_ptr as usize, "*const i32".to_string()).boxed(),
        ))
        .unwrap();
    assert_scalar(&var[0], "i32", Some(SupportedScalar::I32(2)));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_uuid() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 519).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(519));

    let vars = debugger.read_local_variables().unwrap();
    assert_uuid_n(&vars[0], "uuid_v4", "Uuid");
    assert_uuid_n(&vars[1], "uuid_v7", "Uuid");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_address_operator() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 119).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(119));

    fn addr_of(name: &str, loc: bool) -> DQE {
        DQE::Address(DQE::Variable(Selector::by_name(name, loc)).boxed())
    }
    fn addr_of_index(name: &str, index: i32) -> DQE {
        DQE::Address(
            DQE::Index(
                DQE::Variable(Selector::by_name(name, true)).boxed(),
                Literal::Int(index as i64),
            )
            .boxed(),
        )
    }
    fn addr_of_field(name: &str, field: &str) -> DQE {
        DQE::Address(
            DQE::Field(
                DQE::Variable(Selector::by_name(name, true)).boxed(),
                field.to_string(),
            )
            .boxed(),
        )
    }

    // get address of scalar variable and deref it
    let addr_a_dqe = addr_of("a", true);
    let addr_a = debugger.read_variable(addr_a_dqe.clone()).unwrap();
    assert_pointer(&addr_a[0], "&i32");
    let a = debugger
        .read_variable(DQE::Deref(addr_a_dqe.boxed()))
        .unwrap();
    assert_scalar(&a[0], "i32", Some(SupportedScalar::I32(2)));

    let addr_ptr_a = debugger.read_variable(addr_of("ref_a", true)).unwrap();
    assert_pointer(&addr_ptr_a[0], "&&i32");
    let a = debugger
        .read_variable(DQE::Deref(
            DQE::Deref(addr_of("ref_a", true).boxed()).boxed(),
        ))
        .unwrap();
    assert_scalar(&a[0], "i32", Some(SupportedScalar::I32(2)));

    // get address of structure field and deref it
    let addr_f = debugger.read_variable(addr_of("f", true)).unwrap();
    assert_pointer(&addr_f[0], "&Foo");
    let f = debugger
        .read_variable(DQE::Deref(addr_of("f", true).boxed()))
        .unwrap();
    assert_struct(&f[0], "Foo", |i, member| match i {
        0 => assert_member(member, "bar", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(1)))
        }),
        1 => assert_member(member, "baz", |val| {
            assert_array(val, "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("2 items expected"),
            })
        }),
        2 => {
            assert_member(member, "foo", |val| assert_pointer(val, "&i32"));
            let deref = read_single_var(&debugger, "*f.foo");
            assert_scalar(&deref, "i32", Some(SupportedScalar::I32(2)));
        }
        _ => panic!("3 members expected"),
    });
    let addr_f_bar = debugger.read_variable(addr_of_field("f", "bar")).unwrap();
    assert_pointer(&addr_f_bar[0], "&i32");
    let f_bar = debugger
        .read_variable(DQE::Deref(addr_of_field("f", "bar").boxed()))
        .unwrap();
    assert_scalar(&f_bar[0], "i32", Some(SupportedScalar::I32(1)));

    // get address of an array element and deref it
    debugger.set_breakpoint_at_line("vars.rs", 151).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(151));

    let addr_vec1 = debugger.read_variable(addr_of("vec1", true)).unwrap();
    assert_pointer(&addr_vec1[0], "&Vec<i32, alloc::alloc::Global>");

    let addr_el_1 = debugger.read_variable(addr_of_index("vec1", 1)).unwrap();
    assert_pointer(&addr_el_1[0], "&i32");
    let el_1 = debugger
        .read_variable(DQE::Deref(addr_of_index("vec1", 1).boxed()))
        .unwrap();
    assert_scalar(&el_1[0], "i32", Some(SupportedScalar::I32(2)));

    // get an address of a hashmap element and deref it
    debugger.set_breakpoint_at_line("vars.rs", 290).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(290));

    let addr_hm3 = debugger.read_variable(addr_of("hm3", true)).unwrap();
    let inner_hash_map_type = version_switch!(
            rust_version(VARS_APP).unwrap(),
            (1, 0, 0) ..= (1, 75, u32::MAX) => "&HashMap<i32, i32, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "&HashMap<i32, i32, std::hash::random::RandomState>",
    ).unwrap();
    assert_pointer(&addr_hm3[0], inner_hash_map_type);

    let addr_el_11 = debugger.read_variable(addr_of_index("hm3", 11)).unwrap();
    assert_pointer(&addr_el_11[0], "&i32");
    let el_11 = debugger
        .read_variable(DQE::Deref(addr_of_index("hm3", 11).boxed()))
        .unwrap();
    assert_scalar(&el_11[0], "i32", Some(SupportedScalar::I32(11)));

    // get address of global variable and deref it
    let addr_glob_1 = debugger.read_variable(addr_of("GLOB_1", false)).unwrap();
    assert_pointer(&addr_glob_1[0], "&&str");
    let glob_1 = debugger
        .read_variable(DQE::Deref(addr_of("GLOB_1", false).boxed()))
        .unwrap();
    assert_str(&glob_1[0], "glob_1");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_read_time() {
    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 529).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(529));

    let vars = debugger.read_local_variables().unwrap();
    assert_system_time_n(&vars[0], "system_time", (0, 0));
    assert_instant_n(&vars[1], "instant");

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
