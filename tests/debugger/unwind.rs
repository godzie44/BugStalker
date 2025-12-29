use crate::CALLS_APP;
use crate::common::{TestHooks, TestInfo};
use crate::{assert_no_proc, prepare_debugee_process};
use bugstalker::debugger::DebuggerBuilder;
use bugstalker::debugger::variable::render::{RenderValue, ValueLayout};
use serial_test::serial;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn build_debug_frame_only_binary() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be available")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("bugstalker-debug-frame-{nonce}"));
    fs::create_dir_all(&dir).expect("failed to create temp build directory");

    let source = r#"
        #include <stdio.h>

        static void inner(int value) {
            printf("%d\n", value);
        }

        void outer(void) {
            inner(42);
            __builtin_trap();
        }

        int main(void) {
            outer();
            return 0;
        }
    "#;

    let source_path = dir.join("debug_frame_only.c");
    fs::write(&source_path, source).expect("failed to write C source");

    let binary_path = dir.join("debug_frame_only");
    let status = Command::new("cc")
        .arg("-g")
        .arg("-fno-asynchronous-unwind-tables")
        .arg("-fno-unwind-tables")
        .arg("-o")
        .arg(&binary_path)
        .arg(&source_path)
        .status()
        .expect("failed to execute C compiler");
    assert!(status.success(), "C compiler returned non-zero status");

    let status = Command::new("objcopy")
        .arg("--remove-section")
        .arg(".eh_frame")
        .arg("--remove-section")
        .arg(".eh_frame_hdr")
        .arg(&binary_path)
        .status()
        .expect("failed to execute objcopy");
    assert!(status.success(), "objcopy returned non-zero status");

    binary_path
}

#[test]
#[serial]
fn test_unwind_restores_registers_for_caller_frame() {
    // NOTE:
    // This test relies on the fact that `arg1` and `arg2` in the caller frame (`main`)
    // are described by DWARF locations that are CFA/SP-relative.
    //
    // Correct evaluation of these variables therefore implicitly depends on the unwinder
    // restoring the caller frame registers (in particular SP/CFA) correctly.
    //
    // TODO: Provide a way for tests to express explicit variable location requirements
    // (e.g. asserting that a variable is CFA-relative), instead of relying on compiler behavior.    let process = prepare_debugee_process(CALLS_APP, &[]);
    let debugee_pid = process.pid();
    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.set_breakpoint_at_line("calls.rs", 30).unwrap();
    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(30));

    debugger.set_frame_into_focus(1).unwrap();
    let vars = debugger.read_local_variables().unwrap();

    let arg1 = vars
        .iter()
        .find(|var| var.identity().to_string() == "arg1")
        .expect("arg1 must be available in caller frame");
    let arg2 = vars
        .iter()
        .find(|var| var.identity().to_string() == "arg2")
        .expect("arg2 must be available in caller frame");

    match arg1.value().value_layout().unwrap() {
        ValueLayout::PreRendered(rendered) => assert_eq!(rendered.as_ref(), "100"),
        layout => panic!("unexpected arg1 layout: {layout:?}"),
    }

    match arg2.value().value_layout().unwrap() {
        ValueLayout::PreRendered(rendered) => assert_eq!(rendered.as_ref(), "101"),
        layout => panic!("unexpected arg2 layout: {layout:?}"),
    }

    drop(debugger);
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_unwind_uses_debug_frame_when_eh_frame_missing() {
    let binary_path = build_debug_frame_only_binary();
    let binary_str = binary_path
        .to_str()
        .expect("binary path should be valid utf-8")
        .to_string();
    let process = prepare_debugee_process(binary_str.as_str(), &[]);
    let debugee_pid = process.pid();

    let info = TestInfo::default();
    let builder = DebuggerBuilder::new().with_hooks(TestHooks::new(info.clone()));
    let mut debugger = builder.build(process).unwrap();

    debugger.start_debugee().unwrap();

    let backtrace = debugger.backtrace(debugee_pid).unwrap();
    assert!(
        !backtrace.is_empty(),
        "backtrace should be available from .debug_frame"
    );

    drop(debugger);
    assert_no_proc!(debugee_pid);
}
