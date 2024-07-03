use crate::common::TestHooks;
use crate::common::{rust_version, TestInfo};
use crate::VARS_APP;
use crate::{assert_no_proc, prepare_debugee_process};
use bugstalker::debugger::variable::dqe::{Dqe, Literal, LiteralOrWildcard, PointerCast, Selector};
use bugstalker::debugger::variable::render::RenderValue;
use bugstalker::debugger::variable::value::{Member, SpecializedValue, SupportedScalar, Value};
use bugstalker::debugger::DebuggerBuilder;
use bugstalker::version::Version;
use bugstalker::version_switch;
use serial_test::serial;
use std::collections::HashMap;

pub fn assert_scalar(value: &Value, exp_type: &str, exp_val: Option<SupportedScalar>) {
    let Value::Scalar(scalar) = value else {
        panic!("not a scalar");
    };
    assert_eq!(value.r#type().name_fmt(), exp_type);
    assert_eq!(scalar.value, exp_val);
}

fn assert_struct(val: &Value, exp_type: &str, for_each_member: impl Fn(usize, &Member)) {
    let Value::Struct(structure) = val else {
        panic!("not a struct");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    for (i, member) in structure.members.iter().enumerate() {
        for_each_member(i, member)
    }
}

fn assert_member(member: &Member, expected_field_name: &str, with_value: impl Fn(&Value)) {
    assert_eq!(member.field_name.as_deref(), Some(expected_field_name));
    with_value(&member.value);
}

fn assert_array(val: &Value, exp_type: &str, for_each_item: impl Fn(usize, &Value)) {
    let Value::Array(array) = val else {
        panic!("not an array");
    };
    assert_eq!(array.type_ident.name_fmt(), exp_type);
    for (i, item) in array.items.as_ref().unwrap_or(&vec![]).iter().enumerate() {
        for_each_item(i, &item.value)
    }
}

fn assert_c_enum(val: &Value, exp_type: &str, exp_value: Option<String>) {
    let Value::CEnum(c_enum) = val else {
        panic!("not a c_enum");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    assert_eq!(c_enum.value, exp_value);
}

fn assert_rust_enum(val: &Value, exp_type: &str, with_value: impl FnOnce(&Value)) {
    let Value::RustEnum(rust_enum) = val else {
        panic!("not a c_enum");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    with_value(&rust_enum.value.as_ref().unwrap().value);
}

fn assert_pointer(val: &Value, exp_type: &str) {
    let Value::Pointer(ptr) = val else {
        panic!("not a pointer");
    };
    assert_eq!(ptr.type_ident.name_fmt(), exp_type);
}

fn assert_vec(val: &Value, exp_type: &str, exp_cap: usize, with_buf: impl FnOnce(&Value)) {
    let Value::Specialized {
        value: Some(SpecializedValue::Vector(vector)),
        ..
    } = val
    else {
        panic!("not a vector");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    let Value::Scalar(capacity) = &vector.structure.members[1].value else {
        panic!("no capacity");
    };
    assert_eq!(capacity.value, Some(SupportedScalar::Usize(exp_cap)));
    with_buf(&vector.structure.members[0].value);
}

fn assert_string(val: &Value, exp_value: &str) {
    let Value::Specialized {
        value: Some(SpecializedValue::String(string)),
        ..
    } = val
    else {
        panic!("not a string");
    };
    assert_eq!(string.value, exp_value);
}

fn assert_str(val: &Value, exp_value: &str) {
    let Value::Specialized {
        value: Some(SpecializedValue::Str(str)),
        ..
    } = val
    else {
        panic!("not a &str");
    };
    assert_eq!(str.value, exp_value);
}

fn assert_init_tls(val: &Value, exp_type: &str, with_inner: impl FnOnce(&Value)) {
    let Value::Specialized {
        value: Some(SpecializedValue::Tls(tls)),
        ..
    } = val
    else {
        panic!("not a tls");
    };
    assert_eq!(tls.inner_type.name_fmt(), exp_type);
    with_inner(tls.inner_value.as_ref().unwrap());
}

fn assert_uninit_tls(val: &Value, exp_type: &str) {
    let Value::Specialized {
        value: Some(SpecializedValue::Tls(tls)),
        ..
    } = val
    else {
        panic!("not a tls");
    };
    assert_eq!(tls.inner_type.name_fmt(), exp_type);
    assert!(tls.inner_value.is_none());
}

fn assert_hashmap(val: &Value, exp_type: &str, with_kv_items: impl FnOnce(&Vec<(Value, Value)>)) {
    let Value::Specialized {
        value: Some(SpecializedValue::HashMap(map)),
        ..
    } = val
    else {
        panic!("not a hashmap");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    let mut items = map.kv_items.clone();
    items.sort_by(|v1, v2| {
        let k1_render = format!("{:?}", v1.0.value_layout());
        let k2_render = format!("{:?}", v2.0.value_layout());
        k1_render.cmp(&k2_render)
    });
    with_kv_items(&items);
}

fn assert_hashset(val: &Value, exp_type: &str, with_items: impl FnOnce(&Vec<Value>)) {
    let Value::Specialized {
        value: Some(SpecializedValue::HashSet(set)),
        ..
    } = val
    else {
        panic!("not a hashset");
    };
    assert_eq!(set.type_ident.name_fmt(), exp_type);
    let mut items = set.items.clone();
    items.sort_by(|v1, v2| {
        let k1_render = format!("{:?}", v1.value_layout());
        let k2_render = format!("{:?}", v2.value_layout());
        k1_render.cmp(&k2_render)
    });
    with_items(&items);
}

fn assert_btree_map(val: &Value, exp_type: &str, with_kv_items: impl FnOnce(&Vec<(Value, Value)>)) {
    let Value::Specialized {
        value: Some(SpecializedValue::BTreeMap(map)),
        ..
    } = val
    else {
        panic!("not a BTreeMap");
    };
    assert_eq!(map.type_ident.name_fmt(), exp_type);
    with_kv_items(&map.kv_items);
}

fn assert_btree_set(val: &Value, exp_type: &str, with_items: impl FnOnce(&Vec<Value>)) {
    let Value::Specialized {
        value: Some(SpecializedValue::BTreeSet(set)),
        ..
    } = val
    else {
        panic!("not a BTreeSet");
    };
    assert_eq!(set.type_ident.name_fmt(), exp_type);
    with_items(&set.items);
}

fn assert_vec_deque(val: &Value, exp_type: &str, exp_cap: usize, with_buf: impl FnOnce(&Value)) {
    let Value::Specialized {
        value: Some(SpecializedValue::VecDeque(vector)),
        ..
    } = val
    else {
        panic!("not a VecDeque");
    };
    assert_eq!(vector.structure.type_ident.name_fmt(), exp_type);
    let Value::Scalar(capacity) = &vector.structure.members[1].value else {
        panic!("no capacity");
    };
    assert_eq!(capacity.value, Some(SupportedScalar::Usize(exp_cap)));
    with_buf(&vector.structure.members[0].value);
}

fn assert_cell(val: &Value, exp_type: &str, with_value: impl FnOnce(&Value)) {
    let Value::Specialized {
        value: Some(SpecializedValue::Cell(value)),
        ..
    } = val
    else {
        panic!("not a Cell");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    with_value(value.as_ref());
}

fn assert_refcell(val: &Value, exp_type: &str, exp_borrow: isize, with_inner: impl FnOnce(&Value)) {
    let Value::Specialized {
        value: Some(SpecializedValue::RefCell(value)),
        ..
    } = val
    else {
        panic!("not a Cell");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
    let Value::Struct(as_struct) = value.as_ref() else {
        panic!("not a struct")
    };

    let Value::Scalar(borrow) = &as_struct.members[0].value else {
        panic!("no borrow flag");
    };
    assert_eq!(
        borrow.value.as_ref().unwrap(),
        &SupportedScalar::Isize(exp_borrow)
    );
    with_inner(&as_struct.members[1].value);
}

fn assert_rc(val: &Value, exp_type: &str) {
    let Value::Specialized {
        value: Some(SpecializedValue::Rc(_)),
        ..
    } = val
    else {
        panic!("not an rc");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
}

fn assert_arc(val: &Value, exp_type: &str) {
    let Value::Specialized {
        value: Some(SpecializedValue::Arc(_)),
        ..
    } = val
    else {
        panic!("not an arc");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
}

fn assert_uuid(val: &Value, exp_type: &str) {
    let Value::Specialized {
        value: Some(SpecializedValue::Uuid(_)),
        ..
    } = val
    else {
        panic!("not an uuid");
    };
    assert_eq!(val.r#type().name_fmt(), exp_type);
}

fn assert_system_time(val: &Value, exp_value: (i64, u32)) {
    let Value::Specialized {
        value: Some(SpecializedValue::SystemTime(value)),
        ..
    } = val
    else {
        panic!("not a SystemTime");
    };
    assert_eq!(*value, exp_value);
}

fn assert_instant(val: &Value) {
    let Value::Specialized {
        value: Some(SpecializedValue::Instant(_)),
        ..
    } = val
    else {
        panic!("not an Instant");
    };
}

macro_rules! read_locals {
    ($debugger: expr => $($var: ident),*) => {
        let vars = $debugger.read_local_variables().unwrap();
        let &[$($var),*] = &vars.as_slice() else {
            panic!("Invalid variables count")
        };
    };
}

macro_rules! read_var_dqe {
    ($debugger: expr, $dqe: expr => $($var: ident),*) => {
        let vars = $debugger.read_variable($dqe).unwrap();
        let &[$($var),*] = &vars.as_slice() else {
            panic!("Invalid variables count")
        };
    };
}

macro_rules! read_arg_dqe {
    ($debugger: expr, $dqe: expr => $($var: ident),*) => {
        let args = $debugger.read_argument($dqe).unwrap();
        let &[$($var),*] = &args.as_slice() else {
            panic!("Invalid variables count")
        };
    };
}

macro_rules! read_var_dqe_type_order {
    ($debugger: expr, $dqe: expr => $($var: ident),*) => {
        let mut vars = $debugger.read_variable($dqe).unwrap();
        vars.sort_by(|v1, v2| v1.value().r#type().cmp(v2.value().r#type()));
        let &[$($var),*] = &vars.as_slice() else {
            panic!("Invalid variables count")
        };
    };
}

macro_rules! assert_idents {
    ($($var: ident => $name: literal),*) => {
        $(
            assert_eq!($var.identity().to_string(), $name);
        )*
    };
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

    read_locals!(debugger => int8, int16, int32, int64, int128, isize, uint8, uint16, uint32, uint64, uint128, usize, f32, f64, b_true, b_false, ch_ascii, c_n_ascii);
    assert_idents!(
        int8 => "int8", int16 => "int16", int32 => "int32", int64 => "int64", int128 => "int128",
        isize => "isize", uint8 => "uint8", uint16 => "uint16", uint32 => "uint32", uint64 => "uint64",
        uint128 => "uint128", usize => "usize", f32 => "f32", f64 => "f64", b_true => "boolean_true",
        b_false => "boolean_false", ch_ascii => "char_ascii", c_n_ascii => "char_non_ascii"
    );

    assert_scalar(int8.value(), "i8", Some(SupportedScalar::I8(1)));
    assert_scalar(int16.value(), "i16", Some(SupportedScalar::I16(-1)));
    assert_scalar(int32.value(), "i32", Some(SupportedScalar::I32(2)));
    assert_scalar(int64.value(), "i64", Some(SupportedScalar::I64(-2)));
    assert_scalar(int128.value(), "i128", Some(SupportedScalar::I128(3)));
    assert_scalar(isize.value(), "isize", Some(SupportedScalar::Isize(-3)));
    assert_scalar(uint8.value(), "u8", Some(SupportedScalar::U8(1)));
    assert_scalar(uint16.value(), "u16", Some(SupportedScalar::U16(2)));
    assert_scalar(uint32.value(), "u32", Some(SupportedScalar::U32(3)));
    assert_scalar(uint64.value(), "u64", Some(SupportedScalar::U64(4)));
    assert_scalar(uint128.value(), "u128", Some(SupportedScalar::U128(5)));
    assert_scalar(usize.value(), "usize", Some(SupportedScalar::Usize(6)));
    assert_scalar(f32.value(), "f32", Some(SupportedScalar::F32(1.1)));
    assert_scalar(f64.value(), "f64", Some(SupportedScalar::F64(1.2)));
    assert_scalar(b_true.value(), "bool", Some(SupportedScalar::Bool(true)));
    assert_scalar(b_false.value(), "bool", Some(SupportedScalar::Bool(false)));
    assert_scalar(ch_ascii.value(), "char", Some(SupportedScalar::Char('a')));
    assert_scalar(c_n_ascii.value(), "char", Some(SupportedScalar::Char('😊')));

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

    read_locals!(debugger => tuple_0, tuple_1, tuple_2, foo, foo2);
    assert_idents!(tuple_0 => "tuple_0", tuple_1 => "tuple_1", tuple_2 => "tuple_2", foo => "foo", foo2 => "foo2");

    assert_scalar(tuple_0.value(), "()", Some(SupportedScalar::Empty()));
    assert_struct(tuple_1.value(), "(f64, f64)", |i, member| match i {
        0 => assert_member(member, "__0", |val| {
            assert_scalar(val, "f64", Some(SupportedScalar::F64(0f64)))
        }),
        1 => assert_member(member, "__1", |val| {
            assert_scalar(val, "f64", Some(SupportedScalar::F64(1.1f64)))
        }),
        _ => panic!("2 members expected"),
    });
    assert_struct(
        tuple_2.value(),
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
    assert_struct(foo.value(), "Foo", |i, member| match i {
        0 => assert_member(member, "bar", |val| {
            assert_scalar(val, "i32", Some(SupportedScalar::I32(100)))
        }),
        1 => assert_member(member, "baz", |val| {
            assert_scalar(val, "char", Some(SupportedScalar::Char('9')))
        }),
        _ => panic!("2 members expected"),
    });
    assert_struct(foo2.value(), "Foo2", |i, member| match i {
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

    read_locals!(debugger => arr_1, arr_2);
    assert_idents!(arr_1 => "arr_1", arr_2 => "arr_2");

    assert_array(arr_1.value(), "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("5 items expected"),
    });
    assert_array(arr_2.value(), "[[i32]]", |i, item| match i {
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

    read_locals!(debugger => enum_1, enum_2, enum_3, enum_4, enum_5, enum_6, enum_7);
    assert_idents!(
        enum_1 => "enum_1", enum_2 => "enum_2", enum_3 => "enum_3", enum_4 => "enum_4",
        enum_5 => "enum_5", enum_6 => "enum_6", enum_7 => "enum_7"
    );

    assert_c_enum(enum_1.value(), "EnumA", Some("B".to_string()));
    assert_rust_enum(enum_2.value(), "EnumC", |enum_val| {
        assert_struct(enum_val, "C", |_, member| {
            assert_member(member, "__0", |val| {
                assert_scalar(val, "char", Some(SupportedScalar::Char('b')))
            })
        });
    });
    assert_rust_enum(enum_3.value(), "EnumC", |enum_val| {
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
    assert_rust_enum(enum_4.value(), "EnumC", |enum_val| {
        assert_struct(enum_val, "E", |_, _| {
            panic!("expected empty struct");
        });
    });
    assert_rust_enum(enum_5.value(), "EnumF", |enum_val| {
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
    assert_rust_enum(enum_6.value(), "EnumF", |enum_val| {
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
    assert_rust_enum(enum_7.value(), "EnumF", |enum_val| {
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

    read_locals!(debugger => a, ref_a, ptr_a, ptr_ptr_a, b, mut_ref_b, c, mut_ptr_c, box_d, f, ref_f);
    assert_idents!(
        a => "a", ref_a => "ref_a", ptr_a => "ptr_a", ptr_ptr_a => "ptr_ptr_a", b => "b",
        mut_ref_b => "mut_ref_b",c => "c", mut_ptr_c => "mut_ptr_c", box_d => "box_d", f => "f", ref_f => "ref_f"
    );

    assert_scalar(a.value(), "i32", Some(SupportedScalar::I32(2)));
    assert_pointer(ref_a.value(), "&i32");
    let deref = ref_a.clone().modify_value(|ctx, val| val.deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
    assert_pointer(ptr_a.value(), "*const i32");
    let deref = ptr_a.clone().modify_value(|ctx, val| val.deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
    assert_pointer(ptr_ptr_a.value(), "*const *const i32");
    let deref = ptr_ptr_a.clone().modify_value(|ctx, val| val.deref(ctx));
    assert_pointer(deref.unwrap().value(), "*const i32");
    let deref = ptr_ptr_a
        .clone()
        .modify_value(|ctx, v| v.deref(ctx).and_then(|v| v.deref(ctx)));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
    assert_scalar(b.value(), "i32", Some(SupportedScalar::I32(2)));
    assert_pointer(mut_ref_b.value(), "&mut i32");
    let deref = mut_ref_b.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
    assert_scalar(c.value(), "i32", Some(SupportedScalar::I32(2)));
    assert_pointer(mut_ptr_c.value(), "*mut i32");
    let deref = mut_ptr_c.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
    assert_pointer(
        box_d.value(),
        "alloc::boxed::Box<i32, alloc::alloc::Global>",
    );
    let deref = box_d.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
    assert_struct(f.value(), "Foo", |i, member| match i {
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
            let foo_val = member.value.clone();
            let deref = f.clone().modify_value(|ctx, _| foo_val.deref(ctx));
            assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
        }
        _ => panic!("3 members expected"),
    });
    assert_pointer(ref_f.value(), "&vars::references::Foo");
    let deref = ref_f.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(deref.unwrap().value(), "Foo", |i, member| match i {
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
            let foo_val = member.value.clone();
            let deref = ref_f.clone().modify_value(|ctx, _| foo_val.deref(ctx));
            assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
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

    read_locals!(debugger => a_alias);
    assert_idents!(a_alias => "a_alias");
    assert_scalar(a_alias.value(), "i32", Some(SupportedScalar::I32(1)));

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

    read_locals!(debugger => a);
    assert_idents!(a => "a");
    assert_struct(a.value(), "Foo<i32>", |i, member| match i {
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

    read_locals!(debugger => vec1, vec2, vec3, slice1, slice2);
    assert_idents!(vec1 => "vec1", vec2 => "vec2", vec3 => "vec3", slice1 => "slice1", slice2 => "slice2");

    assert_vec(vec1.value(), "Vec<i32, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
            _ => panic!("3 items expected"),
        })
    });

    assert_vec(
        vec2.value(),
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

    assert_vec(
        vec3.value(),
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

    assert_pointer(slice1.value(), "&[i32; 3]");
    let deref = slice1.clone().modify_value(|ctx, val| val.deref(ctx));
    assert_array(deref.unwrap().value(), "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("3 items expected"),
    });

    assert_pointer(slice2.value(), "&[&[i32; 3]; 2]");
    let deref = slice2.clone().modify_value(|ctx, val| val.deref(ctx));
    assert_array(deref.unwrap().value(), "[&[i32; 3]]", |i, item| match i {
        0 => {
            assert_pointer(item, "&[i32; 3]");
            let item_val = item.clone();
            let deref = slice2.clone().modify_value(|ctx, _| item_val.deref(ctx));
            assert_array(deref.unwrap().value(), "[i32]", |i, item| match i {
                0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            });
        }
        1 => {
            assert_pointer(item, "&[i32; 3]");
            let item_val = item.clone();
            let deref = slice2.clone().modify_value(|ctx, _| item_val.deref(ctx));
            assert_array(deref.unwrap().value(), "[i32]", |i, item| match i {
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

    read_locals!(debugger => s1, s2, s3);
    assert_idents!(s1 => "s1", s2 => "s2", s3 => "s3");

    assert_string(s1.value(), "hello world");
    assert_str(s2.value(), "hello world");
    assert_str(s3.value(), "hello world");

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

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name("GLOB_1", false)) => glob_1);
    assert_idents!(glob_1 => "vars::GLOB_1");
    assert_str(glob_1.value(), "glob_1");

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name("GLOB_2", false)) => glob_2);
    assert_idents!(glob_2 => "vars::GLOB_2");
    assert_scalar(glob_2.value(), "i32", Some(SupportedScalar::I32(2)));

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
        .read_variable(Dqe::Variable(Selector::by_name("GLOB_1", true)))
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

    read_var_dqe_type_order!(debugger, Dqe::Variable(Selector::by_name("GLOB_3", false)) => glob_3_1, glob_3_2);
    assert_idents!(glob_3_1 => "vars::ns_1::GLOB_3");
    assert_str(glob_3_1.value(), "glob_3");

    assert_idents!(glob_3_2 => "vars::GLOB_3");
    assert_scalar(glob_3_2.value(), "i32", Some(SupportedScalar::I32(3)));

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
    let rust_version = rust_version(VARS_APP).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 194).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(194));

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_1",
            false,
        )) => tls_var_1);

    version_switch!(
        rust_version,
        (1, 0, 0) ..= (1, 79, u32::MAX) => {
            assert_idents!(tls_var_1 => "vars::THREAD_LOCAL_VAR_1::__getit::__KEY");
        },
        (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => {
            assert_idents!(tls_var_1 => "vars::THREAD_LOCAL_VAR_1::{constant#0}::{closure#1}::VAL");
        }
    );
    assert_init_tls(tls_var_1.value(), "Cell<i32>", |inner| {
        assert_cell(inner, "Cell<i32>", |value| {
            assert_scalar(value, "i32", Some(SupportedScalar::I32(2)))
        })
    });

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_2",
            false,
        )) => tls_var_2);
    version_switch!(
        rust_version,
        (1, 0, 0) ..= (1, 79, u32::MAX) => {
            assert_idents!(tls_var_2 => "vars::THREAD_LOCAL_VAR_2::__getit::__KEY");
        },
        (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => {
            assert_idents!(tls_var_2 => "vars::THREAD_LOCAL_VAR_2::{constant#0}::{closure#1}::VAL");
        }
    );
    assert_init_tls(tls_var_2.value(), "Cell<&str>", |inner| {
        assert_cell(inner, "Cell<&str>", |value| assert_str(value, "2"))
    });

    // assert uninit tls variables
    debugger.set_breakpoint_at_line("vars.rs", 199).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(199));

    version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 79, u32::MAX) => {
                read_var_dqe!(debugger, Dqe::Variable(Selector::by_name(
                    "THREAD_LOCAL_VAR_1",
                    false,
                )) => tls_var_1);
                assert_idents!(tls_var_1 => "vars::THREAD_LOCAL_VAR_1::__getit::__KEY");
                assert_uninit_tls(tls_var_1.value(), "Cell<i32>");
            },
            (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => {
                let vars = debugger.read_variable(Dqe::Variable(Selector::by_name(
                    "THREAD_LOCAL_VAR_1",
                    false,
                ))).unwrap();
                assert!(vars.is_empty());
            },
    );

    // assert tls variables changes in another thread
    debugger.set_breakpoint_at_line("vars.rs", 203).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(203));

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name(
            "THREAD_LOCAL_VAR_1",
            false,
        )) => tls_var_1);
    assert_init_tls(tls_var_1.value(), "Cell<i32>", |inner| {
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
    let rust_version = rust_version(VARS_APP).unwrap();
    if rust_version < Version((1, 79, 0)) {
        return;
    }

    let process = prepare_debugee_process(VARS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 538).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(538));

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name(
            "CONSTANT_THREAD_LOCAL",
            false,
        )) => const_tls);
    version_switch!(
        rust_version,
        (1, 0, 0) ..= (1, 79, u32::MAX) => {
            assert_idents!(const_tls => "vars::thread_local_const_init::CONSTANT_THREAD_LOCAL::__getit::VAL");
        },
        (1, 80, 0) ..= (1, u32::MAX, u32::MAX) => {
            assert_idents!(const_tls => "vars::thread_local_const_init::CONSTANT_THREAD_LOCAL::{constant#0}::{closure#0}::VAL");
        }
    );
    assert_init_tls(const_tls.value(), "i32", |value| {
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

    read_locals!(debugger => inc, inc_mut, _outer, closure, _a, _b, _c, trait_once, trait_mut, trait_fn, fn_ptr);
    assert_idents!(
        inc => "inc", inc_mut => "inc_mut", closure => "closure", trait_once => "trait_once",
        trait_mut => "trait_mut", trait_fn => "trait_fn", fn_ptr => "fn_ptr"
    );

    assert_struct(inc.value(), "{closure_env#0}", |_, _| {
        panic!("no members expected")
    });
    assert_struct(inc_mut.value(), "{closure_env#1}", |_, _| {
        panic!("no members expected")
    });
    assert_struct(closure.value(), "{closure_env#2}", |_, member| {
        assert_member(member, "outer", |val| assert_string(val, "outer val"))
    });
    let rust_version = rust_version(VARS_APP).unwrap();
    assert_struct(
        trait_once.value(),
        "alloc::boxed::Box<dyn core::ops::function::FnOnce<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "pointer", |val| {
                    assert_pointer(val, "*dyn core::ops::function::FnOnce<(), Output=()>")
                });
                let member_val = member.value.clone();
                let deref = trait_once
                    .clone()
                    .modify_value(|ctx, _| member_val.deref(ctx));
                assert_struct(
                    deref.unwrap().value(),
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
                let member_val = member.value.clone();
                let deref = trait_once
                    .clone()
                    .modify_value(|ctx, _| member_val.deref(ctx));
                assert_array(deref.unwrap().value(), "[usize]", |_, _| {});
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_struct(
        trait_mut.value(),
        "alloc::boxed::Box<dyn core::ops::function::FnMut<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "pointer", |val| {
                    assert_pointer(val, "*dyn core::ops::function::FnMut<(), Output=()>")
                });
                let member_val = member.value.clone();
                let deref = trait_mut
                    .clone()
                    .modify_value(|ctx, _| member_val.deref(ctx));
                assert_struct(
                    deref.unwrap().value(),
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
                let member_val = member.value.clone();
                let deref = trait_mut
                    .clone()
                    .modify_value(|ctx, _| member_val.deref(ctx));
                assert_array(deref.unwrap().value(), "[usize]", |_, _| {});
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_struct(
        trait_fn.value(),
        "alloc::boxed::Box<dyn core::ops::function::Fn<(), Output=()>, alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "pointer", |val| {
                    assert_pointer(val, "*dyn core::ops::function::Fn<(), Output=()>")
                });
                let member_val = member.value.clone();
                let deref = trait_fn
                    .clone()
                    .modify_value(|ctx, _| member_val.deref(ctx));
                assert_struct(
                    deref.unwrap().value(),
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
                assert_member(member, "vtable", |val| assert_pointer(val, exp_type));
                let member_val = member.value.clone();
                let deref = trait_fn
                    .clone()
                    .modify_value(|ctx, _| member_val.deref(ctx));
                assert_array(deref.unwrap().value(), "[usize]", |_, _| {});
            }
            _ => panic!("2 members expected"),
        },
    );
    assert_pointer(fn_ptr.value(), "fn() -> u8");

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

    read_arg_dqe!(debugger, Dqe::Variable(Selector::Any) => by_val, by_ref, vec, box_arr);
    assert_idents!(by_val => "by_val", by_ref => "by_ref", vec => "vec", box_arr => "box_arr");

    assert_scalar(by_val.value(), "i32", Some(SupportedScalar::I32(1)));

    assert_pointer(by_ref.value(), "&i32");
    let deref = by_ref.clone().modify_value(|ctx, value| value.deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));

    assert_vec(vec.value(), "Vec<u8, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[u8]", |i, item| match i {
            0 => assert_scalar(item, "u8", Some(SupportedScalar::U8(3))),
            1 => assert_scalar(item, "u8", Some(SupportedScalar::U8(4))),
            2 => assert_scalar(item, "u8", Some(SupportedScalar::U8(5))),
            _ => panic!("3 items expected"),
        })
    });

    assert_struct(
        box_arr.value(),
        "alloc::boxed::Box<[u8], alloc::alloc::Global>",
        |i, member| match i {
            0 => {
                assert_member(member, "data_ptr", |val| assert_pointer(val, "*u8"));
                let data_ptr_val = member.value.clone();
                let deref = box_arr
                    .clone()
                    .modify_value(|ctx, _| data_ptr_val.deref(ctx));
                assert_scalar(deref.unwrap().value(), "u8", Some(SupportedScalar::U8(6)));
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

    read_locals!(debugger => union);
    assert_idents!(union => "union");
    assert_struct(union.value(), "Union1", |i, member| match i {
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

    read_locals!(debugger => hm1, hm2, hm3, hm4, _a, b, _hm5, _hm6);
    assert_idents!(hm1 => "hm1", hm2 => "hm2", hm3 => "hm3", hm4 => "hm4");

    assert_hashmap(hm1.value(), hash_map_type, |items| {
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
    assert_hashmap(hm2.value(), hash_map_type, |items| {
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
    assert_hashmap(hm3.value(), hash_map_type, |items| {
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
    assert_hashmap(hm4.value(), hash_map_type, |items| {
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
        Dqe::Index(Dqe::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by bool key
    let dqe = make_idx_dqe("hm1", Literal::Bool(true));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "i64", Some(SupportedScalar::I64(3)));

    // get by string key
    let dqe = make_idx_dqe("hm2", Literal::String("efg".to_string()));
    read_var_dqe!(debugger, dqe => val);
    assert_vec(val.value(), "Vec<i32, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(11))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(12))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(13))),
            _ => panic!("3 items expected"),
        })
    });

    // get by int key
    let dqe = make_idx_dqe("hm3", Literal::Int(99));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "i32", Some(SupportedScalar::I32(99)));

    // get by pointer key
    let Value::Pointer(ptr) = &b.value() else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hm5", Literal::Address(ptr_val));
    read_var_dqe!(debugger, dqe => val);
    assert_str(val.value(), "b");

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
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "i32", Some(SupportedScalar::I32(1)));

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

    read_locals!(debugger => hs1, hs2, hs3, _a, b, _hs4);
    assert_idents!(hs1 => "hs1", hs2 => "hs2", hs3 => "hs3");

    assert_hashset(hs1.value(), hashset_type, |items| {
        assert_eq!(items.len(), 4);
        assert_scalar(&items[0], "i32", Some(SupportedScalar::I32(1)));
        assert_scalar(&items[1], "i32", Some(SupportedScalar::I32(2)));
        assert_scalar(&items[2], "i32", Some(SupportedScalar::I32(3)));
        assert_scalar(&items[3], "i32", Some(SupportedScalar::I32(4)));
    });
    assert_hashset(hs2.value(), hashset_type, |items| {
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
    assert_hashset(hs3.value(), hashset_type, |items| {
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
        Dqe::Index(Dqe::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by int key
    let dqe = make_idx_dqe("hs1", Literal::Int(2));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs1", Literal::Int(5));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(false)));

    // get by pointer key
    let Value::Pointer(ptr) = &b.value() else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hs4", Literal::Address(ptr_val));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs4", Literal::Address(0));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(false)));

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

    read_locals!(debugger => a_circ, b_circ);
    assert_idents!(a_circ => "a_circ", b_circ => "b_circ");

    assert_rc(
        a_circ.value(),
        "Rc<vars::circular::List, alloc::alloc::Global>",
    );
    assert_rc(
        b_circ.value(),
        "Rc<vars::circular::List, alloc::alloc::Global>",
    );

    let deref = a_circ.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(
        deref.unwrap().value(),
        "RcBox<vars::circular::List>",
        |i, member| match i {
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
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("vars.rs", 340).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(340));

    read_locals!(debugger => alpha, _beta);
    // WAITFORFIX: https://github.com/rust-lang/rust/issues/113819
    // expected:     assert_eq!(vars.len(), 1);
    // through this bug there is uninitialized variable here
    assert_idents!(alpha => "alpha");

    debugger.set_breakpoint_at_line("vars.rs", 342).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(342));

    read_locals!(debugger => alpha, beta);
    assert_idents!(alpha => "alpha", beta => "beta");

    debugger.set_breakpoint_at_line("vars.rs", 343).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(343));

    read_locals!(debugger => alpha, beta, gama);
    assert_idents!(alpha => "alpha", beta => "beta", gama => "gama");

    debugger.set_breakpoint_at_line("vars.rs", 349).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(349));

    read_locals!(debugger => alpha, delta);
    assert_idents!(alpha => "alpha", delta => "delta");

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

    read_locals!(debugger => hm1, hm2, hm3, hm4, _a, b, _hm5, _hm6);
    assert_idents!(hm1 => "hm1", hm2 => "hm2", hm3 => "hm3", hm4 => "hm4");

    assert_btree_map(
        hm1.value(),
        "BTreeMap<bool, i64, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "bool", Some(SupportedScalar::Bool(false)));
            assert_scalar(&items[0].1, "i64", Some(SupportedScalar::I64(5)));
            assert_scalar(&items[1].0, "bool", Some(SupportedScalar::Bool(true)));
            assert_scalar(&items[1].1, "i64", Some(SupportedScalar::I64(3)));
        },
    );

    assert_btree_map(
        hm2.value(),
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

    assert_btree_map(
        hm3.value(),
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

    assert_btree_map(
        hm4.value(),
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
        });

    let make_idx_dqe = |var: &str, literal| {
        Dqe::Index(Dqe::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by bool key
    let dqe = make_idx_dqe("hm1", Literal::Bool(true));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "i64", Some(SupportedScalar::I64(3)));

    // get by string key
    let dqe = make_idx_dqe("hm2", Literal::String("efg".to_string()));
    read_var_dqe!(debugger, dqe => val);
    assert_vec(val.value(), "Vec<i32, alloc::alloc::Global>", 3, |buf| {
        assert_array(buf, "[i32]", |i, item| match i {
            0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(11))),
            1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(12))),
            2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(13))),
            _ => panic!("3 items expected"),
        })
    });

    // get by int key
    let dqe = make_idx_dqe("hm3", Literal::Int(99));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "i32", Some(SupportedScalar::I32(99)));

    // get by pointer key
    let Value::Pointer(ptr) = b.value() else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hm5", Literal::Address(ptr_val));
    read_var_dqe!(debugger, dqe => val);
    assert_str(val.value(), "b");

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
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "i32", Some(SupportedScalar::I32(2)));

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

    read_locals!(debugger => hs1, hs2, hs3, _a, b, _hs4);
    assert_idents!(hs1 => "hs1", hs2 => "hs2", hs3 => "hs3");

    assert_btree_set(
        hs1.value(),
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
        hs2.value(),
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
        hs3.value(),
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
        Dqe::Index(Dqe::Variable(Selector::by_name(var, true)).boxed(), literal)
    };

    // get by int key
    let dqe = make_idx_dqe("hs1", Literal::Int(2));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs1", Literal::Int(5));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(false)));

    // get by pointer key
    let Value::Pointer(ptr) = b.value() else {
        panic!("not a pointer")
    };
    let ptr_val = ptr.value.unwrap() as usize;

    let dqe = make_idx_dqe("hs4", Literal::Address(ptr_val));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(true)));

    let dqe = make_idx_dqe("hs4", Literal::Address(0));
    read_var_dqe!(debugger, dqe => val);
    assert_scalar(val.value(), "bool", Some(SupportedScalar::Bool(false)));

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

    read_locals!(debugger => vd1, vd2);
    assert_idents!(vd1 => "vd1", vd2 => "vd2");

    assert_vec_deque(
        vd1.value(),
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

    assert_vec_deque(
        vd2.value(),
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

    read_locals!(debugger => int32_atomic, _int32, int32_atomic_ptr);
    assert_idents!(int32_atomic => "int32_atomic", int32_atomic_ptr => "int32_atomic_ptr");

    assert_struct(int32_atomic.value(), "AtomicI32", |i, member| match i {
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

    assert_struct(
        int32_atomic_ptr.value(),
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

    let deref = int32_atomic_ptr
        .clone()
        .modify_value(|ctx, v| v.field("p").unwrap().field("value").unwrap().deref(ctx));
    assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));

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

    read_locals!(debugger => a_cell, b_refcell, _b_refcell_borrow_1, _b_refcell_borrow_2);
    assert_idents!(a_cell => "a_cell", b_refcell => "b_refcell");

    assert_cell(a_cell.value(), "Cell<i32>", |value| {
        assert_scalar(value, "i32", Some(SupportedScalar::I32(1)))
    });

    assert_refcell(
        b_refcell.value(),
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

    debugger.set_breakpoint_at_line("vars.rs", 475).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(475));

    read_locals!(debugger => rc0, rc1, weak_rc2, arc0, arc1, weak_arc2);
    assert_idents!(
        rc0 => "rc0", rc1 => "rc1", weak_rc2 => "weak_rc2", arc0 => "arc0", arc1 => "arc1", weak_arc2 => "weak_arc2"
    );

    assert_rc(rc0.value(), "Rc<i32, alloc::alloc::Global>");
    let deref = rc0.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(deref.unwrap().value(), "RcBox<i32>", |i, member| match i {
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

    assert_rc(rc1.value(), "Rc<i32, alloc::alloc::Global>");
    let deref = rc1.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(deref.unwrap().value(), "RcBox<i32>", |i, member| match i {
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

    assert_rc(weak_rc2.value(), "Weak<i32, alloc::alloc::Global>");
    let deref = weak_rc2.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(deref.unwrap().value(), "RcBox<i32>", |i, member| match i {
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

    assert_arc(arc0.value(), "Arc<i32, alloc::alloc::Global>");
    let deref = arc0.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(
        deref.unwrap().value(),
        "ArcInner<i32>",
        |i, member| match i {
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
        },
    );

    assert_arc(arc1.value(), "Arc<i32, alloc::alloc::Global>");
    let deref = arc1.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_struct(
        deref.unwrap().value(),
        "ArcInner<i32>",
        |i, member| match i {
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
        },
    );

    assert_arc(weak_arc2.value(), "Weak<i32, alloc::alloc::Global>");
    let deref = weak_arc2
        .clone()
        .modify_value(|ctx, v| v.deref(ctx))
        .unwrap();
    assert_struct(deref.value(), "ArcInner<i32>", |i, member| match i {
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

    read_locals!(
        debugger => ptr_zst, array_zst, vec_zst, slice_zst, struct_zst, enum_zst, vecdeque_zst,
        hash_map_zst_key, hash_map_zst_val, hash_map_zst, hash_set_zst, btree_map_zst_key,
        btree_map_zst_val, btree_map_zst, btree_set_zst
    );
    assert_idents!(
        ptr_zst => "ptr_zst", array_zst => "array_zst", vec_zst => "vec_zst",
        slice_zst => "slice_zst", struct_zst => "struct_zst", enum_zst => "enum_zst",
        vecdeque_zst => "vecdeque_zst", hash_map_zst_key => "hash_map_zst_key",
        hash_map_zst_val => "hash_map_zst_val", hash_map_zst => "hash_map_zst",
        hash_set_zst => "hash_set_zst", btree_map_zst_key => "btree_map_zst_key",
        btree_map_zst_val => "btree_map_zst_val", btree_map_zst => "btree_map_zst",
        btree_set_zst => "btree_set_zst"
    );

    assert_pointer(ptr_zst.value(), "&()");
    let deref = ptr_zst.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_scalar(deref.unwrap().value(), "()", Some(SupportedScalar::Empty()));

    assert_array(array_zst.value(), "[()]", |i, item| match i {
        0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        _ => panic!("2 members expected"),
    });

    assert_vec(vec_zst.value(), "Vec<(), alloc::alloc::Global>", 0, |buf| {
        assert_array(buf, "[()]", |i, item| match i {
            0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
            1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
            2 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
            _ => panic!("3 members expected"),
        })
    });

    assert_pointer(slice_zst.value(), "&[(); 4]");
    let deref = slice_zst.clone().modify_value(|ctx, v| v.deref(ctx));
    assert_array(deref.unwrap().value(), "[()]", |i, item| match i {
        0 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        1 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        2 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        3 => assert_scalar(item, "()", Some(SupportedScalar::Empty())),
        _ => panic!("4 members expected"),
    });

    assert_struct(struct_zst.value(), "StructZst", |i, member| match i {
        0 => assert_member(member, "__0", |val| {
            assert_scalar(val, "()", Some(SupportedScalar::Empty()))
        }),
        _ => panic!("1 member expected"),
    });

    assert_rust_enum(enum_zst.value(), "Option<()>", |member| {
        assert_struct(member, "Some", |i, member| match i {
            0 => assert_member(member, "__0", |val| {
                assert_scalar(val, "()", Some(SupportedScalar::Empty()))
            }),
            _ => panic!("1 member expected"),
        })
    });

    assert_vec_deque(
        vecdeque_zst.value(),
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
    assert_hashmap(hash_map_zst_key.value(), hashmap_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
        assert_scalar(&items[0].1, "i32", Some(SupportedScalar::I32(1)));
    });

    let hashmap_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<i32, (), std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<i32, (), std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap(hash_map_zst_val.value(), hashmap_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0].0, "i32", Some(SupportedScalar::I32(1)));
        assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
    });

    let hashmap_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashMap<(), (), std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashMap<(), (), std::hash::random::RandomState>",
    ).unwrap();
    assert_hashmap(hash_map_zst.value(), hashmap_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
        assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
    });

    let hashset_type = version_switch!(
            rust_version,
            (1, 0, 0) ..= (1, 75, u32::MAX) => "HashSet<(), std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "HashSet<(), std::hash::random::RandomState>",
    ).unwrap();
    assert_hashset(hash_set_zst.value(), hashset_type, |items| {
        assert_eq!(items.len(), 1);
        assert_scalar(&items[0], "()", Some(SupportedScalar::Empty()));
    });

    assert_btree_map(
        btree_map_zst_key.value(),
        "BTreeMap<(), i32, alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "i32", Some(SupportedScalar::I32(1)));
        },
    );

    assert_btree_map(
        btree_map_zst_val.value(),
        "BTreeMap<i32, (), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 2);
            assert_scalar(&items[0].0, "i32", Some(SupportedScalar::I32(1)));
            assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[1].0, "i32", Some(SupportedScalar::I32(2)));
            assert_scalar(&items[1].1, "()", Some(SupportedScalar::Empty()));
        },
    );

    assert_btree_map(
        btree_map_zst.value(),
        "BTreeMap<(), (), alloc::alloc::Global>",
        |items| {
            assert_eq!(items.len(), 1);
            assert_scalar(&items[0].0, "()", Some(SupportedScalar::Empty()));
            assert_scalar(&items[0].1, "()", Some(SupportedScalar::Empty()));
        },
    );

    assert_btree_set(
        btree_set_zst.value(),
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

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name("INNER_STATIC", false)) => inner_static);
    assert_idents!(inner_static => "vars::inner_static::INNER_STATIC");
    assert_scalar(inner_static.value(), "u32", Some(SupportedScalar::U32(1)));

    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(570));

    read_var_dqe!(debugger, Dqe::Variable(Selector::by_name("INNER_STATIC", false)) => inner_static);
    assert_idents!(inner_static => "vars::inner_static::INNER_STATIC");
    assert_scalar(inner_static.value(), "u32", Some(SupportedScalar::U32(1)));

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

    read_var_dqe!(debugger, Dqe::Slice(
            Dqe::Variable(Selector::by_name("arr_1", true)).boxed(),
            None,
            None,
        ) => arr_1);
    assert_idents!(arr_1 => "arr_1");
    assert_array(arr_1.value(), "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        2 => assert_scalar(item, "i32", Some(SupportedScalar::I32(2))),
        3 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        4 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("5 items expected"),
    });

    read_var_dqe!(debugger, Dqe::Slice(
            Dqe::Variable(Selector::by_name("arr_1", true)).boxed(),
            Some(3),
            None,
        ) => arr_1);
    assert_idents!(arr_1 => "arr_1");
    assert_array(arr_1.value(), "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-2))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(3))),
        _ => panic!("2 items expected"),
    });

    read_var_dqe!(debugger, Dqe::Slice(
            Dqe::Variable(Selector::by_name("arr_1", true)).boxed(),
            None,
            Some(2),
        ) => arr_1);
    assert_idents!(arr_1 => "arr_1");
    assert_array(arr_1.value(), "[i32]", |i, item| match i {
        0 => assert_scalar(item, "i32", Some(SupportedScalar::I32(1))),
        1 => assert_scalar(item, "i32", Some(SupportedScalar::I32(-1))),
        _ => panic!("2 items expected"),
    });

    read_var_dqe!(debugger, Dqe::Slice(
            Dqe::Variable(Selector::by_name("arr_1", true)).boxed(),
            Some(1),
            Some(4),
        ) => arr_1);
    assert_idents!(arr_1 => "arr_1");
    assert_array(arr_1.value(), "[i32]", |i, item| match i {
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

    read_locals!(debugger => a, ref_a, _ptr_a, _ptr_ptr_a, _b, _mut_ref_b, _c, _mut_ptr_c, _box_d, _f, _ref_f);

    assert_scalar(a.value(), "i32", Some(SupportedScalar::I32(2)));
    let Value::Pointer(pointer) = ref_a.value() else {
        panic!("expect a pointer");
    };

    let raw_ptr = pointer.value.unwrap();

    read_var_dqe!(debugger, Dqe::Deref(
            Dqe::PtrCast(PointerCast::new(raw_ptr as usize, "*const i32")).boxed(),
        ) => val);
    assert_scalar(val.value(), "i32", Some(SupportedScalar::I32(2)));

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

    read_locals!(debugger => uuid_v4, uuid_v7);
    assert_idents!(uuid_v4 => "uuid_v4", uuid_v7 => "uuid_v7");
    assert_uuid(uuid_v4.value(), "Uuid");
    assert_uuid(uuid_v7.value(), "Uuid");

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

    fn addr_of(name: &str, loc: bool) -> Dqe {
        Dqe::Address(Dqe::Variable(Selector::by_name(name, loc)).boxed())
    }
    fn addr_of_index(name: &str, index: i32) -> Dqe {
        Dqe::Address(
            Dqe::Index(
                Dqe::Variable(Selector::by_name(name, true)).boxed(),
                Literal::Int(index as i64),
            )
            .boxed(),
        )
    }
    fn addr_of_field(name: &str, field: &str) -> Dqe {
        Dqe::Address(
            Dqe::Field(
                Dqe::Variable(Selector::by_name(name, true)).boxed(),
                field.to_string(),
            )
            .boxed(),
        )
    }

    // get address of scalar variable and deref it
    let addr_a_dqe = addr_of("a", true);
    read_var_dqe!(debugger, addr_a_dqe.clone() => a);
    assert_pointer(a.value(), "&i32");
    read_var_dqe!(debugger, Dqe::Deref(addr_a_dqe.boxed()) => a);
    assert_scalar(a.value(), "i32", Some(SupportedScalar::I32(2)));

    read_var_dqe!(debugger, addr_of("ref_a", true) => addr_ptr_a);
    assert_pointer(addr_ptr_a.value(), "&&i32");
    read_var_dqe!(debugger, Dqe::Deref(
            Dqe::Deref(addr_of("ref_a", true).boxed()).boxed(),
        ) => a);
    assert_scalar(a.value(), "i32", Some(SupportedScalar::I32(2)));

    // get address of structure field and deref it
    read_var_dqe!(debugger, addr_of("f", true) => addr_f);
    assert_pointer(addr_f.value(), "&Foo");
    read_var_dqe!(debugger, Dqe::Deref(addr_of("f", true).boxed()) => f);
    assert_struct(f.value(), "Foo", |i, member| match i {
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
            let member_val = member.value.clone();
            let deref = f.clone().modify_value(|ctx, _| member_val.deref(ctx));
            assert_scalar(deref.unwrap().value(), "i32", Some(SupportedScalar::I32(2)));
        }
        _ => panic!("3 members expected"),
    });

    read_var_dqe!(debugger, addr_of_field("f", "bar") => addr_f_bar);
    assert_pointer(addr_f_bar.value(), "&i32");
    read_var_dqe!(debugger, Dqe::Deref(addr_of_field("f", "bar").boxed()) => f_bar);
    assert_scalar(f_bar.value(), "i32", Some(SupportedScalar::I32(1)));

    // get address of an array element and deref it
    debugger.set_breakpoint_at_line("vars.rs", 151).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(151));

    read_var_dqe!(debugger, addr_of("vec1", true) => addr_vec1);
    assert_pointer(addr_vec1.value(), "&Vec<i32, alloc::alloc::Global>");
    read_var_dqe!(debugger, addr_of_index("vec1", 1) => addr_el_1);
    assert_pointer(addr_el_1.value(), "&i32");
    read_var_dqe!(debugger, Dqe::Deref(addr_of_index("vec1", 1).boxed()) => el_1);
    assert_scalar(el_1.value(), "i32", Some(SupportedScalar::I32(2)));

    // get an address of a hashmap element and deref it
    debugger.set_breakpoint_at_line("vars.rs", 290).unwrap();
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(290));

    read_var_dqe!(debugger, addr_of("hm3", true) => addr_hm3);
    let inner_hash_map_type = version_switch!(
            rust_version(VARS_APP).unwrap(),
            (1, 0, 0) ..= (1, 75, u32::MAX) => "&HashMap<i32, i32, std::collections::hash::map::RandomState>",
            (1, 76, 0) ..= (1, u32::MAX, u32::MAX) => "&HashMap<i32, i32, std::hash::random::RandomState>",
    ).unwrap();
    assert_pointer(addr_hm3.value(), inner_hash_map_type);

    read_var_dqe!(debugger, addr_of_index("hm3", 11) => addr_el_11);
    assert_pointer(addr_el_11.value(), "&i32");
    read_var_dqe!(debugger, Dqe::Deref(addr_of_index("hm3", 11).boxed()) => el_11);
    assert_scalar(el_11.value(), "i32", Some(SupportedScalar::I32(11)));

    // get address of global variable and deref it
    read_var_dqe!(debugger, addr_of("GLOB_1", false) => addr_glob_1);
    assert_pointer(addr_glob_1.value(), "&&str");
    read_var_dqe!(debugger, Dqe::Deref(addr_of("GLOB_1", false).boxed()) => glob_1);
    assert_str(glob_1.value(), "glob_1");

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

    read_locals!(debugger => system_time, instant);
    assert_idents!(system_time => "system_time", instant => "instant");
    assert_system_time(system_time.value(), (0, 0));
    assert_instant(instant.value());

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
