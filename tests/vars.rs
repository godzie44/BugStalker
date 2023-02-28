use assert_cmd::cargo::CommandCargoExt;
use rexpect::process::signal::SIGINT;
use rexpect::session::PtySession;
use std::ops::Add;
use std::process::Command;

#[test]
fn test_read_scalar_variables() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:26").unwrap();
    session.exp_string("break vars.rs:26").unwrap();

    session.send_line("run").unwrap();
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

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_scalar_variables_at_place() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:7").unwrap();
    session.exp_string("break vars.rs:7").unwrap();

    session.send_line("run").unwrap();
    session.exp_string(">    let int128 = 3_i128;").unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("int8 = i8(1)").unwrap();
    session.exp_string("int16 = i16(-1)").unwrap();
    session.exp_string("int32 = i32(2)").unwrap();
    session.exp_string("int64 = i64(-2)").unwrap();
    assert!(session.exp_string("int128 = i128(3)").is_err());

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_struct() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:50").unwrap();
    session.exp_string("break vars.rs:50").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("tuple_0 = ()").unwrap();
    session.exp_string("tuple_1 = (f64, f64) {").unwrap();
    session.exp_string("0: f64(0)").unwrap();
    session.exp_string("1: f64(1.1)").unwrap();
    session.exp_string("}").unwrap();

    session
        .exp_string("tuple_2 = (u64, i64, char, bool) {")
        .unwrap();
    session.exp_string("0: u64(1)").unwrap();
    session.exp_string("1: i64(-1)").unwrap();
    session.exp_string("2: char(a)").unwrap();
    session.exp_string("3: bool(false)").unwrap();
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

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_array() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:59").unwrap();
    session.exp_string("break vars.rs:59").unwrap();

    session.send_line("run").unwrap();
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

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_enum() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:92").unwrap();
    session.exp_string("break vars.rs:92").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("enum_1 = EnumA::B").unwrap();

    session.exp_string("enum_2 = EnumC::C {").unwrap();
    session.exp_string("0: char(b)").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("enum_3 = EnumC::D {").unwrap();
    session.exp_string("0: f64(1.1)").unwrap();
    session.exp_string("1: f32(1.2)").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("enum_4 = EnumC::E {").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("enum_5 = EnumF::F {").unwrap();
    session.exp_string("0: EnumC::C {").unwrap();
    session.exp_string("0: char(f)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("enum_6 = EnumF::G {").unwrap();
    session.exp_string("0: Foo {").unwrap();
    session.exp_string("a: i32(1)").unwrap();
    session.exp_string("b: char(1)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();

    session.exp_string("enum_7 = EnumF::J {").unwrap();
    session.exp_string("0: EnumA::A").unwrap();
    session.exp_string("}").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_pointers() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:119").unwrap();
    session.exp_string("break vars.rs:119").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("a = i32(2)").unwrap();
    session.exp_regex(r"ref_a = &i32 \[0x.*\]").unwrap();
    session.exp_regex(r"ptr_a = \*const i32 \[0x.*\]").unwrap();
    session
        .exp_regex(r"ptr_ptr_a = \*const \*const i32 \[0x.*\]")
        .unwrap();

    session.exp_string("b = i32(2)").unwrap();
    session.exp_regex(r"mut_ref_b = &mut i32 \[0x.*\]").unwrap();

    session.exp_string("c = i32(2)").unwrap();
    session
        .exp_regex(r"mut_ptr_c = \*mut i32 \[0x.*\]")
        .unwrap();

    session
        .exp_regex(r"box_d = alloc::boxed::Box<i32, alloc::alloc::Global> \[0x.*\]")
        .unwrap();

    session.exp_string("f = Foo {").unwrap();
    session.exp_string("bar: i32(1)").unwrap();
    session.exp_string("baz: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_regex(r"foo: &i32 \[0x.*\]").unwrap();
    session.exp_string("}").unwrap();

    session
        .exp_regex(r"ref_f = &vars::references::Foo \[0x.*\]")
        .unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_deref_pointers() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:119").unwrap();
    session.exp_string("break vars.rs:119").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars *ref_a").unwrap();
    session.exp_string("*ref_a = i32(2)").unwrap();

    session.send_line("vars *ptr_a").unwrap();
    session.exp_string("*ptr_a = i32(2)").unwrap();

    session.send_line("vars *ptr_ptr_a").unwrap();
    session
        .exp_regex(r"\*ptr_ptr_a = \*const i32 \[0x.*\]")
        .unwrap();

    session.send_line("vars **ptr_ptr_a").unwrap();
    session.exp_string("**ptr_ptr_a = i32(2)").unwrap();

    session.send_line("vars *mut_ref_b").unwrap();
    session.exp_string("*mut_ref_b = i32(2)").unwrap();

    session.send_line("vars *mut_ptr_c").unwrap();
    session.exp_string("*mut_ptr_c = i32(2)").unwrap();

    session.send_line("vars *box_d").unwrap();
    session.exp_string("*box_d = i32(2)").unwrap();

    session.send_line("vars *ref_f").unwrap();
    session.exp_string("*ref_f = Foo {").unwrap();
    session.exp_string("bar: i32(1)").unwrap();
    session.exp_string("baz: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_regex(r"foo: &i32 \[0x.*\]").unwrap();
    session.exp_string("}").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_type_alias() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:127").unwrap();
    session.exp_string("break vars.rs:127").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("a_alias = i32(1)").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_type_parameters() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:137").unwrap();
    session.exp_string("break vars.rs:137").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("a = Foo<i32> {").unwrap();
    session.exp_string("bar: i32(1)").unwrap();
    session.exp_string("}").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_vec_and_slice() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:154").unwrap();
    session.exp_string("break vars.rs:154").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session
        .exp_string("vec1 = Vec<i32, alloc::alloc::Global> {")
        .unwrap();
    session.exp_string("buf: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("cap: usize(3)").unwrap();
    session.exp_string("}").unwrap();

    session
        .exp_string("vec2 = Vec<vars::vec_and_slice_types::Foo, alloc::alloc::Global> {")
        .unwrap();
    session.exp_string("buf: [Foo] {").unwrap();
    session.exp_string("0: Foo {").unwrap();
    session.exp_string("foo: i32(1)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("1: Foo {").unwrap();
    session.exp_string("foo: i32(2)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("cap: usize(2)").unwrap();
    session.exp_string("}").unwrap();

    session
        .exp_string(
            "vec3 = Vec<alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global> {",
        )
        .unwrap();
    session
        .exp_string("buf: [Vec<i32, alloc::alloc::Global>] {")
        .unwrap();
    session
        .exp_string("0: Vec<i32, alloc::alloc::Global> {")
        .unwrap();
    session.exp_string("buf: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("cap: usize(3)").unwrap();
    session.exp_string("}").unwrap();
    session
        .exp_string("1: Vec<i32, alloc::alloc::Global> {")
        .unwrap();
    session.exp_string("buf: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("cap: usize(3)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("cap: usize(2)").unwrap();
    session.exp_string("}").unwrap();

    session.exp_regex(r"slice1 = &\[i32; 3\] \[0x.*]").unwrap();
    session
        .exp_regex(r"slice2 = &\[&\[i32; 3\]; 2\] \[0x.*\]")
        .unwrap();

    session.send_line("vars *slice1").unwrap();
    session.exp_string("*slice1 = [i32]").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();

    session.send_line("vars *slice2").unwrap();
    session.exp_string("*slice2 = [&[i32; 3]] {").unwrap();
    session.exp_regex(r"0: &\[i32; 3\] \[0x.*]").unwrap();
    session.exp_regex(r"1: &\[i32; 3\] \[0x.*]").unwrap();
    session.exp_string("}").unwrap();

    session.send_line("vars *(*slice2)[0]").unwrap();
    session.exp_string("*0 = [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();

    session.send_line("vars *(*slice2)[1]").unwrap();
    session.exp_string("*1 = [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_strings() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:163").unwrap();
    session.exp_string("break vars.rs:163").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("s1 = String(hello world)").unwrap();
    session.exp_string("s2 = &str(hello world)").unwrap();
    session.exp_string("s3 = &str(hello world)").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_static_variables() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:173").unwrap();
    session.exp_string("break vars.rs:173").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars GLOB_1").unwrap();
    session.exp_string("GLOB_1 = &str(glob_1)").unwrap();
    session.send_line("vars GLOB_2").unwrap();
    session.exp_string("GLOB_2 = i32(2)").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_static_variables_different_modules() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:185").unwrap();
    session.exp_string("break vars.rs:185").unwrap();

    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars GLOB_3").unwrap();
    session.exp_string("GLOB_3").unwrap();
    session.exp_string("GLOB_3").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_read_tls_variables() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:201").unwrap();
    session.exp_string("break vars.rs:201").unwrap();

    // assert tls variables values
    session.send_line("run").unwrap();
    session
        .exp_string(">        let nop: Option<u8> = None;")
        .unwrap();
    session.send_line("vars THREAD_LOCAL_VAR_1").unwrap();
    session
        .exp_string("THREAD_LOCAL_VAR_1 = Cell<i32> {")
        .unwrap();
    session.exp_string("value: UnsafeCell<i32> {").unwrap();
    session.exp_string("value: i32(2)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();
    session.send_line("vars THREAD_LOCAL_VAR_2").unwrap();
    session
        .exp_string("THREAD_LOCAL_VAR_2 = Cell<&str> {")
        .unwrap();
    session.exp_string("value: UnsafeCell<&str> {").unwrap();
    session.exp_string("value: &str(2)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();

    // assert uninit tls variables
    session.send_line("break vars.rs:206").unwrap();
    session.exp_string("break vars.rs:206").unwrap();
    session.send_line("continue").unwrap();
    session
        .exp_string(">        let nop: Option<u8> = None;")
        .unwrap();
    session.send_line("vars THREAD_LOCAL_VAR_1").unwrap();
    session
        .exp_string("THREAD_LOCAL_VAR_1 = Cell<i32>(uninit)")
        .unwrap();

    // assert tls variables changes in another thread
    session.send_line("break vars.rs:210").unwrap();
    session.exp_string("break vars.rs:210").unwrap();
    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();
    session.send_line("vars THREAD_LOCAL_VAR_1").unwrap();
    session
        .exp_string("THREAD_LOCAL_VAR_1 = Cell<i32> {")
        .unwrap();
    session.exp_string("value: UnsafeCell<i32> {").unwrap();
    session.exp_string("value: i32(1)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("}").unwrap();

    session.process.kill(SIGINT).unwrap();
}

#[test]
fn test_custom_select() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break vars.rs:59").unwrap();
    session.exp_string("break vars.rs:59").unwrap();
    session.send_line("run").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars arr_2[0][2]").unwrap();
    session.exp_string("2 = i32(2)").unwrap();

    session.send_line("break vars.rs:92").unwrap();
    session.exp_string("break vars.rs:92").unwrap();
    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars enum_6.0.a").unwrap();
    session.exp_string("a = i32(1)").unwrap();

    session.send_line("break vars.rs:119").unwrap();
    session.exp_string("break vars.rs:119").unwrap();
    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars *((*ref_f).foo)").unwrap();
    session.exp_string("*foo = i32(2)").unwrap();

    session.send_line("break vars.rs:267").unwrap();
    session.exp_string("break vars.rs:267").unwrap();
    session.send_line("continue").unwrap();
    session
        .exp_string(">    let nop: Option<u8> = None;")
        .unwrap();

    session.send_line("vars hm2.abc").unwrap();
    // todo fix '1 = ... '
    session
        .exp_string("1 = Vec<i32, alloc::alloc::Global> {")
        .unwrap();
    session.exp_string("buf: [i32] {").unwrap();
    session.exp_string("0: i32(1)").unwrap();
    session.exp_string("1: i32(2)").unwrap();
    session.exp_string("2: i32(3)").unwrap();
    session.exp_string("}").unwrap();
    session.exp_string("cap: usize(3)").unwrap();
    session.exp_string("}").unwrap();

    session.process.kill(SIGINT).unwrap();
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
