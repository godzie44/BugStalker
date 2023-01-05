use assert_cmd::cargo::CommandCargoExt;
use rexpect::session::PtySession;
use std::ops::Add;
use std::process::Command;

#[test]
fn test_multithreaded_app_running() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("continue").unwrap();

    session.exp_string("thread 1 spawn").unwrap();
    session.exp_string("thread 2 spawn").unwrap();
    session.exp_string("sum2: 199990000").unwrap();
    session.exp_string("sum1: 49995000").unwrap();
    session.exp_string("total 249985000").unwrap();
    session.exp_string("Program exit with code: 0").unwrap();
}

#[test]
fn test_multithreaded_breakpoints() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    // set breakpoint at program start.
    session.send_line("break mt.rs:6").unwrap();
    session.exp_string("break mt.rs:6").unwrap();
    // set breakpoints at thread 1 code.
    session.send_line("break mt.rs:21").unwrap();
    session.exp_string("break mt.rs:21").unwrap();
    // set breakpoint at thread 2 code.
    session.send_line("break mt.rs:31").unwrap();
    session.exp_string("break mt.rs:31").unwrap();
    // set breakpoint at program ends.
    session.send_line("break mt.rs:14").unwrap();
    session.exp_string("break mt.rs:14").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hit breakpoint at address").unwrap();
    session
        .exp_string(">    let jh1 = thread::spawn(sum1);")
        .unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hit breakpoint at address").unwrap();
    session.exp_string(">    let mut sum2 = 0;").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hit breakpoint at address").unwrap();
    session.exp_string(">    let mut sum = 0;").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hit breakpoint at address").unwrap();
    session
        .exp_string(">    println!(\"total {}\", sum1 + sum2);")
        .unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("total 249985000").unwrap();
    session.exp_string("Program exit with code: 0").unwrap();
}

#[test]
fn test_multithreaded_backtrace() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break mt.rs:21").unwrap();
    session.exp_string("break mt.rs:21").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hit breakpoint at address").unwrap();
    session.exp_string(">    let mut sum = 0;").unwrap();

    session.send_line("backtrace").unwrap();
    session.exp_string("0x005555555618D9 - mt::sum1").unwrap();
    session
        .exp_string("std::sys::unix::thread::Thread::new::thread_start")
        .unwrap();
}

#[test]
fn test_multithreaded_trace() {
    let mut session = setup_vars_debugee();
    session.exp_string("No previous history.").unwrap();

    session.send_line("break mt.rs:31").unwrap();
    session.exp_string("break mt.rs:31").unwrap();

    session.send_line("continue").unwrap();
    session.exp_string("Hit breakpoint at address").unwrap();
    session.exp_string(">    let mut sum2 = 0;").unwrap();

    session.send_line("trace").unwrap();
    session.exp_string("thread 1").unwrap();
    session.exp_string("mt::main").unwrap();
    session.exp_string("thread 2").unwrap();
    session.exp_string("std::thread::sleep").unwrap();
    session.exp_string("thread 3").unwrap();
    session.exp_string("mt::sum2").unwrap();
}

fn setup_vars_debugee() -> PtySession {
    let mut cmd = Command::cargo_bin("bugstalker").unwrap();
    cmd.arg("./target/debug/mt");
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
