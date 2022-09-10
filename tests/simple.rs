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
fn test_breakpoint_set() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 55555555BC13").unwrap();
    session.exp_string("break 55555555BC13").unwrap();
    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session
        .exp_string("Hit breakpoint at address 0x55555555BC13")
        .unwrap();
    session.exp_string("myprint(\"bye!\")").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_multiple_breakpoint_set() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 55555555BBE4").unwrap();
    session.exp_string("break 55555555BBE4").unwrap();
    session.send_line("break 55555555BC13").unwrap();
    session.exp_string("break 55555555BC13").unwrap();

    session.send_line("continue").unwrap();

    session
        .exp_string("Hit breakpoint at address 0x55555555BBE4")
        .unwrap();
    session.exp_string("myprint(\"Hello, world!\")").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();

    session
        .exp_string("Hit breakpoint at address 0x55555555BC13")
        .unwrap();
    session.exp_string("myprint(\"bye!\")").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_read_write_register() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();

    session.send_line("break 55555555BC1D").unwrap();
    session.exp_string("break 55555555BC1D").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string("bye!").unwrap();

    session
        .send_line("register write rip 55555555BBD0")
        .unwrap();
    session
        .exp_string("register write rip 55555555BBD0")
        .unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hello, world!").unwrap();
    session.exp_string("bye!").unwrap();
}

#[test]
fn test_step_in() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 55555555BBD0").unwrap();
    session.exp_string("break 55555555BBD0").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string(">fn main()").unwrap();

    session.send_line("step").unwrap();
    session
        .exp_string(">    myprint(\"Hello, world!\");")
        .unwrap();

    session.send_line("step").unwrap();
    session.exp_string(">fn myprint(s: &str)").unwrap();

    session.send_line("step").unwrap();
    session.exp_string(">    println!(\"{}\",s)").unwrap();
}

#[test]
fn test_step_out() {
    let mut session = setup_hello_world_debugee();

    session.exp_string("No previous history.").unwrap();
    session.send_line("break 55555555BBE4").unwrap();
    session.exp_string("break 55555555BBE4").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("myprint(\"Hello, world!\");").unwrap();

    session.send_line("step").unwrap();
    session.exp_string(">fn myprint(s: &str)").unwrap();
    session.send_line("step").unwrap();
    session.exp_string(">    println!(\"{}\",s)").unwrap();

    session.send_line("stepout").unwrap();
    session
        .exp_string(">    sleep(Duration::from_secs(1));")
        .unwrap();
}

fn setup_hello_world_debugee() -> PtySession {
    let mut cmd = Command::cargo_bin("bugstalker").unwrap();
    cmd.arg("../hello-world/target/debug/hello-world");
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
