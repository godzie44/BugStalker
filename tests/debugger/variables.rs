use crate::assert_no_proc;
use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::{debugger_env, VARS_APP};
use bugstalker::debugger;
use bugstalker::debugger::variable;
use bugstalker::debugger::variable::render::RenderRepr;
use bugstalker::debugger::variable::VariableIR;
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
    assert_eq!(scalar.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(scalar.type_name.as_ref().unwrap(), exp_type);
    assert_eq!(scalar.value, exp_val);
}

fn assert_struct(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    for_each_member: fn(usize, &VariableIR),
) {
    let VariableIR::Struct(structure) = var else {
      panic!("not a struct");  
    };
    assert_eq!(structure.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(structure.type_name.as_ref().unwrap(), exp_type);
    for (i, member) in structure.members.iter().enumerate() {
        for_each_member(i, member)
    }
}

fn assert_array(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    for_each_item: fn(usize, &VariableIR),
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
    assert_eq!(c_enum.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(c_enum.type_name.as_ref().unwrap(), exp_type);
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
    assert_eq!(rust_enum.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(rust_enum.type_name.as_ref().unwrap(), exp_type);
    with_value(rust_enum.value.as_ref().unwrap());
}

fn assert_pointer(
    var: &VariableIR,
    exp_name: &str,
    exp_type: &str,
    with_deref: impl FnOnce(&VariableIR),
) {
    let VariableIR::Pointer(ptr) = var else {
        panic!("not a pointer");
    };
    assert_eq!(ptr.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(ptr.type_name.as_ref().unwrap(), exp_type);
    with_deref(ptr.deref.as_ref().unwrap());
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
    assert_eq!(vector.structure.identity.name.as_ref().unwrap(), exp_name);
    assert_eq!(vector.structure.type_name.as_ref().unwrap(), exp_type);
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
    assert_eq!(string.name.as_ref().unwrap(), exp_name);
    assert_eq!(string.value, exp_value);
}

fn assert_str(var: &VariableIR, exp_name: &str, exp_value: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Str {string: Some(str), ..}) = var else {
        panic!("not a &str");
    };
    assert_eq!(str.name.as_ref().unwrap(), exp_name);
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
    assert_eq!(tls.name.as_ref().unwrap(), exp_name);
    assert_eq!(tls.inner_type.as_ref().unwrap(), exp_type);
    with_var(tls.inner_value.as_ref().unwrap());
}

fn assert_uninit_tls(var: &VariableIR, exp_name: &str, exp_type: &str) {
    let VariableIR::Specialized(variable::SpecializedVariableIR::Tls {tls_var: Some(tls), ..}) = var else {
        panic!("not a tls");
    };
    assert_eq!(tls.name.as_ref().unwrap(), exp_name);
    assert_eq!(tls.inner_type.as_ref().unwrap(), exp_type);
    assert!(tls.inner_value.is_none());
}

#[test]
#[serial]
fn test_read_scalar_variables() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 26).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(26));

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
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_scalar_variables_at_place() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 7).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(7));

        let vars = debugger.read_local_variables().unwrap();
        assert_eq!(vars.len(), 4)
    });
}

#[test]
#[serial]
fn test_read_struct() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 50).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(50));

        let vars = debugger.read_local_variables().unwrap();
        assert_scalar(&vars[0], "tuple_0", "()", Some(SupportedScalar::Empty()));
        assert_struct(&vars[1], "tuple_1", "(f64, f64)", |i, member| match i {
            0 => assert_scalar(member, "__0", "f64", Some(SupportedScalar::F64(0f64))),
            1 => assert_scalar(member, "__1", "f64", Some(SupportedScalar::F64(1.1f64))),
            _ => panic!("2 members expected"),
        });
        assert_struct(
            &vars[2],
            "tuple_2",
            "(u64, i64, char, bool)",
            |i, member| match i {
                0 => assert_scalar(member, "__0", "u64", Some(SupportedScalar::U64(1))),
                1 => assert_scalar(member, "__1", "i64", Some(SupportedScalar::I64(-1))),
                2 => assert_scalar(member, "__2", "char", Some(SupportedScalar::Char('a'))),
                3 => assert_scalar(member, "__3", "bool", Some(SupportedScalar::Bool(false))),
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
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_array() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 59).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(59));

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
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_enum() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 92).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(92));

        let vars = debugger.read_local_variables().unwrap();
        assert_c_enum(&vars[0], "enum_1", "EnumA", Some("B".to_string()));
        assert_rust_enum(&vars[1], "enum_2", "EnumC", |enum_val| {
            assert_struct(enum_val, "C", "C", |_, member| {
                assert_scalar(member, "__0", "char", Some(SupportedScalar::Char('b')));
            });
        });
        assert_rust_enum(&vars[2], "enum_3", "EnumC", |enum_val| {
            assert_struct(enum_val, "D", "D", |i, member| {
                match i {
                    0 => assert_scalar(member, "__0", "f64", Some(SupportedScalar::F64(1.1))),
                    1 => assert_scalar(member, "__1", "f32", Some(SupportedScalar::F32(1.2))),
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
                    0 => assert_rust_enum(member, "__0", "EnumC", |enum_val| {
                        assert_struct(enum_val, "C", "C", |_, member| {
                            assert_scalar(member, "__0", "char", Some(SupportedScalar::Char('f')));
                        });
                    }),
                    _ => panic!("1 members expected"),
                };
            });
        });
        assert_rust_enum(&vars[5], "enum_6", "EnumF", |enum_val| {
            assert_struct(enum_val, "G", "G", |i, member| {
                match i {
                    0 => assert_struct(member, "__0", "Foo", |i, member| match i {
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
                    0 => assert_c_enum(member, "__0", "EnumA", Some("A".to_string())),
                    _ => panic!("1 members expected"),
                };
            });
        });

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_pointers() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 119).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(119));

        let vars = debugger.read_local_variables().unwrap();
        assert_scalar(&vars[0], "a", "i32", Some(SupportedScalar::I32(2)));
        assert_pointer(&vars[1], "ref_a", "&i32", |deref| {
            assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
        });
        assert_pointer(&vars[2], "ptr_a", "*const i32", |deref| {
            assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
        });
        assert_pointer(&vars[3], "ptr_ptr_a", "*const *const i32", |deref| {
            assert_pointer(deref, "*", "*const i32", |deref| {
                assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
            });
        });
        assert_scalar(&vars[4], "b", "i32", Some(SupportedScalar::I32(2)));
        assert_pointer(&vars[5], "mut_ref_b", "&mut i32", |deref| {
            assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
        });
        assert_scalar(&vars[6], "c", "i32", Some(SupportedScalar::I32(2)));
        assert_pointer(&vars[7], "mut_ptr_c", "*mut i32", |deref| {
            assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
        });
        assert_pointer(
            &vars[8],
            "box_d",
            "alloc::boxed::Box<i32, alloc::alloc::Global>",
            |deref| assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2))),
        );
        assert_struct(&vars[9], "f", "Foo", |i, member| match i {
            0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(1))),
            1 => assert_array(member, "baz", "[i32]", |i, item| match i {
                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                _ => panic!("2 items expected"),
            }),
            2 => assert_pointer(member, "foo", "&i32", |deref| {
                assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
            }),
            _ => panic!("3 members expected"),
        });
        assert_pointer(&vars[10], "ref_f", "&vars::references::Foo", |deref| {
            assert_struct(deref, "*", "Foo", |i, member| match i {
                0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(1))),
                1 => assert_array(member, "baz", "[i32]", |i, item| match i {
                    0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                    1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                    _ => panic!("2 items expected"),
                }),
                2 => assert_pointer(member, "foo", "&i32", |deref| {
                    assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
                }),
                _ => panic!("3 members expected"),
            });
        });

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_type_alias() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 127).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(127));

        let vars = debugger.read_local_variables().unwrap();
        assert_scalar(&vars[0], "a_alias", "i32", Some(SupportedScalar::I32(1)));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_type_parameters() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 137).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(137));

        let vars = debugger.read_local_variables().unwrap();
        assert_struct(&vars[0], "a", "Foo<i32>", |i, member| match i {
            0 => assert_scalar(member, "bar", "i32", Some(SupportedScalar::I32(1))),
            _ => panic!("1 members expected"),
        });

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_vec_and_slice() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 154).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(154));

        let vars = debugger.read_local_variables().unwrap();
        assert_vec(
            &vars[0],
            "vec1",
            "Vec<i32, alloc::alloc::Global>",
            3,
            |buff| {
                assert_array(buff, "buf", "[i32]", |i, item| match i {
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
            |buff| {
                assert_array(buff, "buf", "[Foo]", |i, item| match i {
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
            |buff| {
                assert_array(
                    buff,
                    "buf",
                    "[Vec<i32, alloc::alloc::Global>]",
                    |i, item| match i {
                        0 => assert_vec(item, "0", "Vec<i32, alloc::alloc::Global>", 3, |buff| {
                            assert_array(buff, "buf", "[i32]", |i, item| match i {
                                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                                2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                                _ => panic!("3 items expected"),
                            })
                        }),
                        1 => assert_vec(item, "1", "Vec<i32, alloc::alloc::Global>", 3, |buff| {
                            assert_array(buff, "buf", "[i32]", |i, item| match i {
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
        assert_pointer(&vars[3], "slice1", "&[i32; 3]", |deref| {
            assert_array(deref, "*", "[i32]", |i, item| match i {
                0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                _ => panic!("3 items expected"),
            });
        });
        assert_pointer(&vars[4], "slice2", "&[&[i32; 3]; 2]", |deref| {
            assert_array(deref, "*", "[&[i32; 3]]", |i, item| match i {
                0 => assert_pointer(item, "0", "&[i32; 3]", |deref| {
                    assert_array(deref, "*", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    });
                }),
                1 => assert_pointer(item, "1", "&[i32; 3]", |deref| {
                    assert_array(deref, "*", "[i32]", |i, item| match i {
                        0 => assert_scalar(item, "0", "i32", Some(SupportedScalar::I32(1))),
                        1 => assert_scalar(item, "1", "i32", Some(SupportedScalar::I32(2))),
                        2 => assert_scalar(item, "2", "i32", Some(SupportedScalar::I32(3))),
                        _ => panic!("3 items expected"),
                    });
                }),
                _ => panic!("2 items expected"),
            });
        });

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_strings() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 163).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(163));

        let vars = debugger.read_local_variables().unwrap();
        assert_string(&vars[0], "s1", "hello world");
        assert_str(&vars[1], "s2", "hello world");
        assert_str(&vars[2], "s3", "hello world");

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_static_variables() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 173).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(173));

        let vars = debugger.read_variable("GLOB_1").unwrap();
        assert_eq!(vars.len(), 1);
        assert_str(&vars[0], "GLOB_1", "glob_1");

        let vars = debugger.read_variable("GLOB_2").unwrap();
        assert_eq!(vars.len(), 1);
        assert_scalar(&vars[0], "GLOB_2", "i32", Some(SupportedScalar::I32(2)));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_static_variables_different_modules() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 185).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(185));

        let mut vars = debugger.read_variable("GLOB_3").unwrap();
        assert_eq!(vars.len(), 2);
        vars.sort_by(|v1, v2| v1.r#type().cmp(v2.r#type()));

        assert_str(&vars[0], "GLOB_3", "glob_3");
        assert_scalar(&vars[1], "GLOB_3", "i32", Some(SupportedScalar::I32(3)));

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_tls_variables() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 201).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(201));

        let vars = debugger.read_variable("THREAD_LOCAL_VAR_1").unwrap();
        assert_init_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>", |inner| {
            assert_struct(inner, "__0", "Cell<i32>", |_, member| {
                assert_struct(member, "value", "UnsafeCell<i32>", |_, member| {
                    assert_scalar(member, "value", "i32", Some(SupportedScalar::I32(2)))
                })
            })
        });

        let vars = debugger.read_variable("THREAD_LOCAL_VAR_2").unwrap();
        assert_init_tls(&vars[0], "THREAD_LOCAL_VAR_2", "Cell<&str>", |inner| {
            assert_struct(inner, "__0", "Cell<&str>", |_, member| {
                assert_struct(member, "value", "UnsafeCell<&str>", |_, member| {
                    assert_str(member, "value", "2")
                })
            })
        });

        // assert uninit tls variables
        debugger.set_breakpoint_at_line("vars.rs", 206).unwrap();
        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(206));

        let vars = debugger.read_variable("THREAD_LOCAL_VAR_1").unwrap();
        assert_uninit_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>");

        // assert tls variables changes in another thread
        debugger.set_breakpoint_at_line("vars.rs", 210).unwrap();
        debugger.continue_debugee().unwrap();
        assert_eq!(info.line.take(), Some(210));

        let vars = debugger.read_variable("THREAD_LOCAL_VAR_1").unwrap();
        assert_init_tls(&vars[0], "THREAD_LOCAL_VAR_1", "Cell<i32>", |inner| {
            assert_struct(inner, "__0", "Cell<i32>", |_, member| {
                assert_struct(member, "value", "UnsafeCell<i32>", |_, member| {
                    assert_scalar(member, "value", "i32", Some(SupportedScalar::I32(1)))
                })
            })
        });

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_closures() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 226).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(226));

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
        assert_struct(&vars[7], "trait_once", "alloc::boxed::Box<dyn core::ops::function::FnOnce<(), Output=()>, alloc::alloc::Global>", |i, member| {
            match i {
                0 => assert_pointer(member, "pointer", "*dyn core::ops::function::FnOnce<(), Output=()>", |deref| {
                    assert_struct(deref, "*", "dyn core::ops::function::FnOnce<(), Output=()>" ,|_, _| {});
                }),
                1 => assert_pointer(member, "vtable", "&[usize; 3]", |deref| {
                    assert_array(deref, "*", "[usize]", |i, _| match i {
                        0 | 1 | 2 => {},
                        _ => panic!("3 items expected"),
                    });
                 }),
                _ => panic!("2 members expected"),
            }
        });
        assert_struct(&vars[8], "trait_mut", "alloc::boxed::Box<dyn core::ops::function::FnMut<(), Output=()>, alloc::alloc::Global>", |i, member| {
            match i {
                0 => assert_pointer(member, "pointer", "*dyn core::ops::function::FnMut<(), Output=()>", |deref| {
                    assert_struct(deref, "*", "dyn core::ops::function::FnMut<(), Output=()>" ,|_, _| {});
                }),
                1 => assert_pointer(member, "vtable", "&[usize; 3]", |deref| {
                    assert_array(deref, "*", "[usize]", |i, _| match i {
                        0 | 1 | 2 => {},
                        _ => panic!("3 items expected"),
                    });
                 }),
                _ => panic!("2 members expected"),
            }
        });
        assert_struct(
            &vars[9],
            "trait_fn",
            "alloc::boxed::Box<dyn core::ops::function::Fn<(), Output=()>, alloc::alloc::Global>",
            |i, member| match i {
                0 => assert_pointer(
                    member,
                    "pointer",
                    "*dyn core::ops::function::Fn<(), Output=()>",
                    |deref| {
                        assert_struct(
                            deref,
                            "*",
                            "dyn core::ops::function::Fn<(), Output=()>",
                            |_, _| {},
                        );
                    },
                ),
                1 => assert_pointer(member, "vtable", "&[usize; 3]", |deref| {
                    assert_array(deref, "*", "[usize]", |i, _| match i {
                        0 | 1 | 2 => {}
                        _ => panic!("3 items expected"),
                    });
                }),
                _ => panic!("2 members expected"),
            },
        );

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_arguments() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 236).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(236));

        let args = debugger.read_arguments().unwrap();
        assert_scalar(&args[0], "by_val", "i32", Some(SupportedScalar::I32(1)));
        assert_pointer(&args[1], "by_ref", "&i32", |deref| {
            assert_scalar(deref, "*", "i32", Some(SupportedScalar::I32(2)))
        });
        assert_vec(
            &args[2],
            "vec",
            "Vec<u8, alloc::alloc::Global>",
            3,
            |buff| {
                assert_array(buff, "buf", "[u8]", |i, item| match i {
                    0 => assert_scalar(item, "0", "u8", Some(SupportedScalar::U8(3))),
                    1 => assert_scalar(item, "1", "u8", Some(SupportedScalar::U8(4))),
                    2 => assert_scalar(item, "2", "u8", Some(SupportedScalar::U8(5))),
                    _ => panic!("3 items expected"),
                })
            },
        );
        assert_struct(
            &args[3],
            "box_arr",
            "alloc::boxed::Box<[u8], alloc::alloc::Global>",
            |i, member| match i {
                0 => assert_pointer(member, "data_ptr", "*u8", |deref| {
                    assert_scalar(deref, "*", "u8", Some(SupportedScalar::U8(6)));
                }),
                1 => assert_scalar(member, "length", "usize", Some(SupportedScalar::Usize(3))),
                _ => panic!("2 members expected"),
            },
        );

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}

#[test]
#[serial]
fn test_read_union() {
    debugger_env!(VARS_APP, child, {
        let info = DebugeeRunInfo::default();
        let mut debugger = Debugger::new(VARS_APP, child, TestHooks::new(info.clone())).unwrap();
        debugger.set_breakpoint_at_line("vars.rs", 249).unwrap();

        debugger.run_debugee().unwrap();
        assert_eq!(info.line.take(), Some(249));

        let vars = debugger.read_local_variables().unwrap();
        assert_struct(&vars[0], "union", "Union1", |i, member| match i {
            0 => assert_scalar(member, "f1", "f32", Some(SupportedScalar::F32(1.1))),
            1 => assert_scalar(member, "u2", "u64", Some(SupportedScalar::U64(1066192077))),
            2 => assert_scalar(member, "u3", "u8", Some(SupportedScalar::U8(205))),
            _ => panic!("3 members expected"),
        });

        debugger.continue_debugee().unwrap();
        assert_no_proc!(child);
    });
}
