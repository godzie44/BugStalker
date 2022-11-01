use assert_cmd::cargo::CommandCargoExt;
use rexpect::session::PtySession;
use std::ops::Add;
use std::process::Command;

#[test]
fn test_debugee_execute() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_address_breakpoint_set() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 0x55555555BD33").unwrap();
    session.exp_string("break 0x55555555BD33").unwrap();
    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session
        .exp_string("Hit breakpoint at address 0x55555555BD33")
        .unwrap();
    session.exp_string("myprint(\"bye!\")").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_multiple_address_breakpoint_set() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 0x55555555BD00").unwrap();
    session.exp_string("break 0x55555555BD00").unwrap();
    session.send_line("break 0x55555555BD33").unwrap(); // zamenil
    session.exp_string("break 0x55555555BD33").unwrap(); // zamenil

    session.send_line("continue").unwrap();

    session
        .exp_string("Hit breakpoint at address 0x55555555BD00")
        .unwrap();
    session.exp_string("myprint(\"Hello, world!\")").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();

    session
        .exp_string("Hit breakpoint at address 0x55555555BD33")
        .unwrap();
    session.exp_string("myprint(\"bye!\")").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_read_write_register() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();

    session.send_line("break 0x55555555BD3C").unwrap();
    session.exp_string("break 0x55555555BD3C").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string("bye!").unwrap();

    session
        .send_line("register write rip 55555555BCF0")
        .unwrap();
    session
        .exp_string("register write rip 55555555BCF0")
        .unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_step_in() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 0x55555555BCF0").unwrap();
    session.exp_string("break 0x55555555BCF0").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string(">fn main()").unwrap();

    session.send_line("step").unwrap();
    session
        .exp_string(">    myprint(\"Hello, world!\");")
        .unwrap();

    session.send_line("step").unwrap();
    session.exp_string(">fn myprint(s: &str)").unwrap();

    session.send_line("step").unwrap();
    session.exp_string(">    println!(\"{}\", s)").unwrap();
}

#[test]
fn test_step_out() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 0x55555555BD00").unwrap();
    session.exp_string("break 0x55555555BD00").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("myprint(\"Hello, world!\");").unwrap();

    session.send_line("step").unwrap();
    session.exp_string(">fn myprint(s: &str)").unwrap();
    session.send_line("step").unwrap();
    session.exp_string(">    println!(\"{}\", s)").unwrap();

    session.send_line("stepout").unwrap();
    session
        .exp_string(">    sleep(Duration::from_secs(1));")
        .unwrap();
}

#[test]
fn test_step_over() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 0x55555555BD00").unwrap();
    session.exp_string("break 0x55555555BD00").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("myprint(\"Hello, world!\");").unwrap();

    session.send_line("next").unwrap();
    session
        .exp_string(">    sleep(Duration::from_secs(1));")
        .unwrap();
    session.send_line("next").unwrap();
    session.exp_string(">    myprint(\"bye!\")").unwrap();
    session.send_line("next").unwrap();
    session.exp_string(">}").unwrap();
}

#[test]
fn test_function_breakpoint() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break myprint").unwrap();
    session.exp_string("break myprint").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("fn myprint").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string("fn myprint").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_line_breakpoint() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break hello_world.rs:15").unwrap();
    session.exp_string("break hello_world.rs:15").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string(">    println!(\"{}\", s)").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string(">    println!(\"{}\", s)").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_symbol() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("symbol main").unwrap();
    session.exp_string("Text 0x7DB0").unwrap();

    session.send_line("symbol myprint").unwrap();
    session.exp_string("Text 0x7D40").unwrap();
}

#[test]
fn test_backtrace() {
    let mut session = setup_hello_world_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break hello_world.rs:15").unwrap();
    session.exp_string("break hello_world.rs:15").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string(">    println!(\"{}\", s)").unwrap();

    session.send_line("bt").unwrap();
    session.exp_string("myprint (0x0055555555bd40)").unwrap();
    session
        .exp_string("hello_world::main (0x0055555555bcf0)")
        .unwrap();
}

#[test]
fn test_read_value_u64() {
    let mut session = setup_calc_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break calc.rs:3").unwrap();
    session.exp_string("break calc.rs:3").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string(">    print(s);").unwrap();

    session.send_line("vars").unwrap();
    session.exp_string("s : 3").unwrap();
}

fn setup_hello_world_debugee() -> PtySession {
    let mut cmd = Command::cargo_bin("bugstalker").unwrap();
    // cmd.arg("../hello-world/target/debug/hello-world");
    cmd.arg("./target/debug/hello_world");
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

fn setup_calc_debugee() -> PtySession {
    let mut cmd = Command::cargo_bin("bugstalker").unwrap();
    // cmd.arg("../hello-world/target/debug/hello-world");
    cmd.arg("./target/debug/calc");
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
