use assert_cmd::cargo::CommandCargoExt;
use rexpect::session::PtySession;
use std::ops::Add;
use std::process::Command;

#[test]
fn test_read_scalar_variables() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:32").unwrap();
    session.exp_string("break vars.rs:32").unwrap();

    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("int8 = i8(1)").unwrap();
    session.exp_string("int16 = i16(-1)").unwrap();
    session.exp_string("int32 = i32(2)").unwrap();
    session.exp_string("int64 = i64(-2)").unwrap();
    session.exp_string("int128 = i128(3)").unwrap();
    session.exp_string("isize = isize(-3)").unwrap();
    session.exp_string("uint8 = u8(1)").unwrap();
    session.exp_string("uint16 = u16(2)").unwrap();
    session.exp_string("uint32 = u32(3)").unwrap();
    session.exp_string("uint64 = u64(4)").unwrap();
    session.exp_string("uint128 = u128(5)").unwrap();
    session.exp_string("usize = usize(6)").unwrap();
    session.exp_string("f32 = f32(1.1)").unwrap();
    session.exp_string("f64 = f64(1.2)").unwrap();
    session.exp_string("boolean_true = bool(true)").unwrap();
    session.exp_string("boolean_false = bool(false)").unwrap();
    session.exp_string("char_ascii = char(a)").unwrap();
    session
        .exp_string("char_non_ascii = char(ð\u{9f}\u{98}\u{8a})")
        .unwrap(); // char(😊)
}

#[test]
fn test_read_scalar_variables_at_place() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:13").unwrap();
    session.exp_string("break vars.rs:13").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string(">    let int128 = 3_i128;").unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("int8 = i8(1)").unwrap();
    session.exp_string("int16 = i16(-1)").unwrap();
    session.exp_string("int32 = i32(2)").unwrap();
    session.exp_string("int64 = i64(-2)").unwrap();
    assert!(session.exp_string("int128 = i128(3)").is_err());
}

#[test]
fn test_read_struct() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:55").unwrap();
    session.exp_string("break vars.rs:55").unwrap();

    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("tuple_1 = (f64, f64) {").unwrap();
    session.exp_string("__0: f64(0)").unwrap();
    session.exp_string("__1: f64(1.1)").unwrap();
    session.exp_string("}").unwrap();

    session
        .exp_string("tuple_2 = (u64, i64, char, bool) {")
        .unwrap();
    session.exp_string("__0: u64(1)").unwrap();
    session.exp_string("__1: i64(-1)").unwrap();
    session.exp_string("__2: char(a)").unwrap();
    session.exp_string("__3: bool(false)").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("foo = Foo {").unwrap();
    session.exp_string("bar: i32(100)").unwrap();
    session.exp_string("baz: char(9)").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("foo2 = Foo2 {").unwrap();
    session.exp_string("foo: Foo {").unwrap();
    session.exp_string("bar: i32(100)").unwrap();
    session.exp_string("baz: char(9)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("additional: bool(true)").unwrap();
    session.exp_string("}").unwrap();
}

#[test]
fn test_read_array() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:64").unwrap();
    session.exp_string("break vars.rs:64").unwrap();

    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("arr_1 = [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(-1)").unwrap();
    session.exp_string("2: i32(2)").unwrap();
    session.exp_string("3: i32(-2)").unwrap();
    session.exp_string("4: i32(3)").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("arr_2 = [[i32]] {").unwrap();
    session.exp_string("0: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(-1)").unwrap();
    session.exp_string("2: i32(2)").unwrap();
    session.exp_string("4: i32(3)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("1: [i32] {").unwrap();
    session.exp_string("0: i32(0)").unwrap();
    session.exp_string("2: i32(2)").unwrap();
    session.exp_string("3: i32(3)").unwrap();
    session.exp_string("4: i32(4)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("2: [i32] {").unwrap();
    session.exp_string("0: i32(0)").unwrap();
    session.exp_string("1: i32(-1)").unwrap();
    session.exp_string("2: i32(-2)").unwrap();
    session.exp_string("3: i32(-3)").unwrap();
    session.exp_string("4: i32(-4)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();
}

fn setup_vars_debugee() -> PtySession {
    let mut cmd = Command::cargo_bin("bugstalker").unwrap();
    cmd.arg("./target/debug/vars");
    let program = cmd.get_program().to_string_lossy().to_string()
        + cmd
            .get_args()
            .into_iter()
            .fold("".to_string(), |res: String, a| {
                res.add(" ").add(a.to_string_lossy().as_ref())
            })
            .as_str();

    rexpect::spawn(&program, Some(2000)).unwrap()
}
