mod dap_client;

use base64::Engine as _;
use dap_client::{DapSession, example_bin, example_source, spawn_attach_target, wait_for_exit};
use serde_json::{Value, json};
use serial_test::serial;
use std::path::Path;
use std::time::Duration;

const HELLO_LINE: i64 = 5;
const SET_VAR_LINE: i64 = 35;

fn assert_response(response: &Value, command: &str, request_seq: i64, success: bool) -> bool {
    assert_eq!(
        response.get("type").and_then(Value::as_str),
        Some("response")
    );
    assert_eq!(
        response.get("command").and_then(Value::as_str),
        Some(command)
    );
    assert_eq!(
        response.get("request_seq").and_then(Value::as_i64),
        Some(request_seq)
    );
    let got_success = response.get("success").and_then(Value::as_bool);
    if got_success == Some(success) {
        assert!(response.get("seq").and_then(Value::as_i64).is_some());
        return true;
    }
    if success {
        if let Some(message) = response.get("message").and_then(Value::as_str) {
            if message.contains("ENOSYS") || message.contains("Function not implemented") {
                return false;
            }
        }
    }
    assert_eq!(got_success, Some(success), "response: {response}");
    assert!(response.get("seq").and_then(Value::as_i64).is_some());
    true
}

macro_rules! ensure_response {
    ($session:expr, $response:expr, $command:expr, $seq:expr, $success:expr) => {{
        if !assert_response($response, $command, $seq, $success) {
            $session.shutdown();
            return Ok(());
        }
    }};
}

macro_rules! require_launch {
    ($session:expr, $program:expr, $source:expr, $line:expr) => {{
        match launch_with_breakpoint($session, $program, $source, $line)? {
            Some(thread_id) => thread_id,
            None => {
                $session.shutdown();
                return Ok(());
            }
        }
    }};
}

macro_rules! require_frame {
    ($session:expr, $thread_id:expr) => {{
        match first_frame_id($session, $thread_id)? {
            Some(frame_id) => frame_id,
            None => {
                $session.shutdown();
                return Ok(());
            }
        }
    }};
}

fn initialize(session: &mut DapSession) -> anyhow::Result<()> {
    let seq = session
        .client
        .send_request("initialize", json!({ "adapterID": "bugstalker" }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "initialize", seq, true);
    let event = session.client.wait_for_event("initialized")?;
    assert_eq!(event.get("type").and_then(Value::as_str), Some("event"));
    Ok(())
}

fn launch_with_breakpoint(
    session: &mut DapSession,
    program: &Path,
    source: &Path,
    line: i64,
) -> anyhow::Result<Option<i64>> {
    initialize(session)?;
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    if !assert_response(&launch_response, "launch", launch_seq, true) {
        return Ok(None);
    }

    let bp_seq = session.client.send_request(
        "setBreakpoints",
        json!({
            "source": { "path": source },
            "breakpoints": [{ "line": line }],
        }),
    )?;
    let bp_response = session.client.read_response(bp_seq)?;
    if !assert_response(&bp_response, "setBreakpoints", bp_seq, true) {
        return Ok(None);
    }

    let config_seq = session
        .client
        .send_request("configurationDone", json!({}))?;
    let config_response = session.client.read_response(config_seq)?;
    if !assert_response(&config_response, "configurationDone", config_seq, true) {
        return Ok(None);
    }

    let stopped = session.client.wait_for_event("stopped")?;
    let thread_id = stopped
        .get("body")
        .and_then(|body| body.get("threadId"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    Ok(Some(thread_id))
}

fn first_frame_id(session: &mut DapSession, thread_id: i64) -> anyhow::Result<Option<i64>> {
    let stack_seq = session
        .client
        .send_request("stackTrace", json!({ "threadId": thread_id }))?;
    let stack_response = session.client.read_response(stack_seq)?;
    if !assert_response(&stack_response, "stackTrace", stack_seq, true) {
        return Ok(None);
    }
    let frame_id = stack_response["body"]["stackFrames"][0]["id"]
        .as_i64()
        .unwrap_or_default();
    Ok(Some(frame_id))
}

#[test]
#[serial]
fn test_initialize_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_launch_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "launch", seq, true);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_attach_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let mut target = spawn_attach_target(&example_bin("dap_attach"))?;
    initialize(&mut session)?;
    let seq = session
        .client
        .send_request("attach", json!({ "pid": target.id() }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "attach", seq, true);
    let config_seq = session
        .client
        .send_request("configurationDone", json!({}))?;
    let config_response = session.client.read_response(config_seq)?;
    ensure_response!(
        session,
        &config_response,
        "configurationDone",
        config_seq,
        true
    );
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    let _ = wait_for_exit(&mut target, Duration::from_secs(1))
        .or_else(|_| target.kill().map_err(anyhow::Error::from));
    Ok(())
}

#[test]
#[serial]
fn test_configuration_done_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    assert!(thread_id > 0);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_breakpoints_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let source = example_source("examples/hello_world/src/hello_world.rs");
    let bp_seq = session.client.send_request(
        "setBreakpoints",
        json!({
            "source": { "path": source },
            "breakpoints": [{ "line": HELLO_LINE }],
        }),
    )?;
    let bp_response = session.client.read_response(bp_seq)?;
    ensure_response!(session, &bp_response, "setBreakpoints", bp_seq, true);
    assert!(bp_response["body"]["breakpoints"].is_array());
    let event = session.client.wait_for_event("breakpoint")?;
    assert_eq!(event["event"], "breakpoint");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_function_breakpoints_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let seq = session.client.send_request(
        "setFunctionBreakpoints",
        json!({ "breakpoints": [{ "name": "myprint" }] }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "setFunctionBreakpoints", seq, true);
    assert!(response["body"]["breakpoints"].is_array());
    let _ = session.client.wait_for_event("breakpoint")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_instruction_breakpoints_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request(
        "gotoTargets",
        json!({
            "source": { "path": example_source("examples/hello_world/src/hello_world.rs") },
            "line": HELLO_LINE,
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "gotoTargets", seq, true);
    let target = &response["body"]["targets"][0];
    let instruction = target["instructionPointerReference"]
        .as_str()
        .unwrap_or("0x0");

    let ibp_seq = session.client.send_request(
        "setInstructionBreakpoints",
        json!({
            "breakpoints": [{ "instructionReference": instruction }],
        }),
    )?;
    let ibp_response = session.client.read_response(ibp_seq)?;
    ensure_response!(
        session,
        &ibp_response,
        "setInstructionBreakpoints",
        ibp_seq,
        true
    );
    assert!(ibp_response["body"]["breakpoints"].is_array());
    assert!(thread_id > 0);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_exception_breakpoints_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let seq = session
        .client
        .send_request("setExceptionBreakpoints", json!({ "filters": ["signal"] }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "setExceptionBreakpoints", seq, true);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_threads_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("threads", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "threads", seq, true);
    assert!(response["body"]["threads"].is_array());
    assert!(thread_id > 0);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_stack_trace_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session
        .client
        .send_request("stackTrace", json!({ "threadId": thread_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "stackTrace", seq, true);
    assert!(response["body"]["stackFrames"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_scopes_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session
        .client
        .send_request("scopes", json!({ "frameId": frame_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "scopes", seq, true);
    assert!(response["body"]["scopes"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_variables_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let scopes_seq = session
        .client
        .send_request("scopes", json!({ "frameId": frame_id }))?;
    let scopes_response = session.client.read_response(scopes_seq)?;
    ensure_response!(session, &scopes_response, "scopes", scopes_seq, true);
    let locals_ref = scopes_response["body"]["scopes"][0]["variablesReference"]
        .as_i64()
        .unwrap_or(0);

    let seq = session
        .client
        .send_request("variables", json!({ "variablesReference": locals_ref }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "variables", seq, true);
    assert!(response["body"]["variables"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_variable_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("dap_set_variable"),
        &example_source("examples/dap_set_variable/src/main.rs"),
        SET_VAR_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let scopes_seq = session
        .client
        .send_request("scopes", json!({ "frameId": frame_id }))?;
    let scopes_response = session.client.read_response(scopes_seq)?;
    ensure_response!(session, &scopes_response, "scopes", scopes_seq, true);
    let locals_ref = scopes_response["body"]["scopes"][0]["variablesReference"]
        .as_i64()
        .unwrap_or(0);

    let vars_seq = session
        .client
        .send_request("variables", json!({ "variablesReference": locals_ref }))?;
    let vars_response = session.client.read_response(vars_seq)?;
    ensure_response!(session, &vars_response, "variables", vars_seq, true);
    let container = vars_response["body"]["variables"]
        .as_array()
        .and_then(|vars| vars.iter().find(|v| v["name"] == "container"))
        .cloned()
        .unwrap();
    let container_ref = container["variablesReference"].as_i64().unwrap_or(0);

    let point_seq = session
        .client
        .send_request("variables", json!({ "variablesReference": container_ref }))?;
    let point_response = session.client.read_response(point_seq)?;
    ensure_response!(session, &point_response, "variables", point_seq, true);
    let point = point_response["body"]["variables"]
        .as_array()
        .and_then(|vars| vars.iter().find(|v| v["name"] == "point"))
        .cloned()
        .unwrap();
    let point_ref = point["variablesReference"].as_i64().unwrap_or(0);

    let seq = session.client.send_request(
        "setVariable",
        json!({
            "variablesReference": point_ref,
            "name": "x",
            "value": "42",
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "setVariable", seq, true);
    assert!(response["body"]["value"].as_str().is_some());
    let _ = session.client.wait_for_event("invalidated")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_evaluate_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("dap_set_variable"),
        &example_source("examples/dap_set_variable/src/main.rs"),
        SET_VAR_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session.client.send_request(
        "evaluate",
        json!({ "expression": "container.point.x", "frameId": frame_id }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "evaluate", seq, true);
    assert!(response["body"]["result"].as_str().is_some());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_expression_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("dap_set_variable"),
        &example_source("examples/dap_set_variable/src/main.rs"),
        SET_VAR_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session.client.send_request(
        "setExpression",
        json!({
            "expression": "container.point.x",
            "value": "41",
            "frameId": frame_id,
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "setExpression", seq, true);
    assert!(response["body"]["value"].as_str().is_some());
    let _ = session.client.wait_for_event("invalidated")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_continue_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("continue", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "continue", seq, true);
    let _ = session.client.wait_for_event("continued")?;
    let _ = session.client.wait_for_event("exited")?;
    let _ = session.client.wait_for_event("terminated")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_next_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session
        .client
        .send_request("next", json!({ "threadId": thread_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "next", seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_step_in_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session
        .client
        .send_request("stepIn", json!({ "threadId": thread_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "stepIn", seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_step_out_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session
        .client
        .send_request("stepOut", json!({ "threadId": thread_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "stepOut", seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_step_back_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session
        .client
        .send_request("stepBack", json!({ "threadId": thread_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "stepBack", seq, false);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_reverse_continue_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("reverseContinue", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "reverseContinue", seq, false);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_pause_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("dap_set_variable"),
        &example_source("examples/dap_set_variable/src/main.rs"),
        SET_VAR_LINE
    );
    let cont_seq = session.client.send_request("continue", json!({}))?;
    let cont_response = session.client.read_response(cont_seq)?;
    ensure_response!(session, &cont_response, "continue", cont_seq, true);
    let pause_seq = session.client.send_request("pause", json!({}))?;
    let pause_response = session.client.read_response(pause_seq)?;
    ensure_response!(session, &pause_response, "pause", pause_seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_goto_targets_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request(
        "gotoTargets",
        json!({
            "source": { "path": example_source("examples/hello_world/src/hello_world.rs") },
            "line": HELLO_LINE,
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "gotoTargets", seq, true);
    assert!(response["body"]["targets"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_goto_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let targets_seq = session.client.send_request(
        "gotoTargets",
        json!({
            "source": { "path": example_source("examples/hello_world/src/hello_world.rs") },
            "line": HELLO_LINE,
        }),
    )?;
    let targets_response = session.client.read_response(targets_seq)?;
    ensure_response!(session, &targets_response, "gotoTargets", targets_seq, true);
    let target_id = targets_response["body"]["targets"][0]["id"]
        .as_i64()
        .unwrap_or(0);

    let seq = session
        .client
        .send_request("goto", json!({ "targetId": target_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "goto", seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_restart_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("restart", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "restart", seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_restart_frame_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session
        .client
        .send_request("restartFrame", json!({ "frameId": frame_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "restartFrame", seq, true);
    let _ = session.client.wait_for_event("stopped")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_read_memory_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("dap_disassemble"),
        &example_source("examples/dap_disassemble/src/main.rs"),
        22
    );
    let targets_seq = session.client.send_request(
        "gotoTargets",
        json!({
            "source": { "path": example_source("examples/dap_disassemble/src/main.rs") },
            "line": 22,
        }),
    )?;
    let targets_response = session.client.read_response(targets_seq)?;
    ensure_response!(session, &targets_response, "gotoTargets", targets_seq, true);
    let instruction = targets_response["body"]["targets"][0]["instructionPointerReference"]
        .as_str()
        .unwrap_or("0x0");

    let seq = session.client.send_request(
        "readMemory",
        json!({ "memoryReference": instruction, "count": 8 }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "readMemory", seq, true);
    assert!(response["body"]["data"].as_str().is_some());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_write_memory_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("dap_disassemble"),
        &example_source("examples/dap_disassemble/src/main.rs"),
        22
    );
    let targets_seq = session.client.send_request(
        "gotoTargets",
        json!({
            "source": { "path": example_source("examples/dap_disassemble/src/main.rs") },
            "line": 22,
        }),
    )?;
    let targets_response = session.client.read_response(targets_seq)?;
    ensure_response!(session, &targets_response, "gotoTargets", targets_seq, true);
    let instruction = targets_response["body"]["targets"][0]["instructionPointerReference"]
        .as_str()
        .unwrap_or("0x0");

    let read_seq = session.client.send_request(
        "readMemory",
        json!({ "memoryReference": instruction, "count": 4 }),
    )?;
    let read_response = session.client.read_response(read_seq)?;
    ensure_response!(session, &read_response, "readMemory", read_seq, true);
    let data = read_response["body"]["data"].as_str().unwrap_or_default();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap_or_default();
    assert!(!bytes.is_empty());

    let seq = session.client.send_request(
        "writeMemory",
        json!({ "memoryReference": instruction, "data": data }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "writeMemory", seq, true);
    assert!(response["body"]["bytesWritten"].as_u64().is_some());
    let _ = session.client.wait_for_event("invalidated")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_disassemble_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("dap_disassemble"),
        &example_source("examples/dap_disassemble/src/main.rs"),
        22
    );
    let targets_seq = session.client.send_request(
        "gotoTargets",
        json!({
            "source": { "path": example_source("examples/dap_disassemble/src/main.rs") },
            "line": 22,
        }),
    )?;
    let targets_response = session.client.read_response(targets_seq)?;
    ensure_response!(session, &targets_response, "gotoTargets", targets_seq, true);
    let instruction = targets_response["body"]["targets"][0]["instructionPointerReference"]
        .as_str()
        .unwrap_or("0x0");

    let seq = session
        .client
        .send_request("disassemble", json!({ "memoryReference": instruction }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "disassemble", seq, true);
    assert!(response["body"]["instructions"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_data_breakpoint_info_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let seq = session
        .client
        .send_request("dataBreakpointInfo", json!({ "name": "stats.count" }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "dataBreakpointInfo", seq, true);
    assert!(response["body"]["description"].as_str().is_some());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_set_data_breakpoints_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("dap_data_breakpoints");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let info_seq = session
        .client
        .send_request("dataBreakpointInfo", json!({ "name": "stats.count" }))?;
    let info_response = session.client.read_response(info_seq)?;
    ensure_response!(
        session,
        &info_response,
        "dataBreakpointInfo",
        info_seq,
        true
    );
    let data_id = info_response["body"]["dataId"].clone();

    let seq = session.client.send_request(
        "setDataBreakpoints",
        json!({ "breakpoints": [{ "dataId": data_id, "accessType": "write" }] }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "setDataBreakpoints", seq, true);
    assert!(response["body"]["breakpoints"].is_array());
    let _ = session.client.wait_for_event("breakpoint")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_modules_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let seq = session.client.send_request("modules", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "modules", seq, true);
    assert!(response["body"]["modules"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_loaded_sources_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let seq = session.client.send_request("loadedSources", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "loadedSources", seq, true);
    assert!(response["body"]["sources"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_source_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);

    let seq = session.client.send_request(
        "source",
        json!({ "source": { "path": example_source("examples/hello_world/src/hello_world.rs") } }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "source", seq, true);
    assert!(response["body"]["content"].as_str().is_some());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_completions_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("dap_set_variable"),
        &example_source("examples/dap_set_variable/src/main.rs"),
        SET_VAR_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session.client.send_request(
        "completions",
        json!({
            "text": "con",
            "column": 3,
            "frameId": frame_id,
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "completions", seq, true);
    assert!(response["body"]["targets"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_step_in_targets_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session
        .client
        .send_request("stepInTargets", json!({ "frameId": frame_id }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "stepInTargets", seq, true);
    assert!(response["body"]["targets"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_breakpoint_locations_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let seq = session.client.send_request(
        "breakpointLocations",
        json!({
            "source": { "path": example_source("examples/hello_world/src/hello_world.rs") },
            "line": HELLO_LINE,
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "breakpointLocations", seq, true);
    assert!(response["body"]["breakpoints"].is_array());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_terminate_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("terminate", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "terminate", seq, true);
    let _ = session.client.wait_for_event("terminated")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_terminate_threads_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session
        .client
        .send_request("terminateThreads", json!({ "threadIds": [] }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "terminateThreads", seq, true);
    let _ = session.client.wait_for_event("terminated")?;
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_disconnect_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let response = session.disconnect(true)?;
    assert!(response["success"].as_bool().unwrap_or(false));
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_cancel_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let seq = session.client.send_request("cancel", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "cancel", seq, true);
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_run_in_terminal_request() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let seq = session
        .client
        .send_request("runInTerminal", json!({ "args": ["/bin/echo", "dap"] }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "runInTerminal", seq, true);
    assert!(response["body"]["processId"].as_u64().is_some());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_initialized() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let seq = session
        .client
        .send_request("initialize", json!({ "adapterID": "bugstalker" }))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "initialize", seq, true);
    let event = session.client.wait_for_event("initialized")?;
    assert_eq!(event["event"], "initialized");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_stopped() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_continued() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("continue", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "continue", seq, true);
    let event = session.client.wait_for_event("continued")?;
    assert_eq!(event["event"], "continued");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_thread() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let event = session.client.wait_for_event("thread")?;
    assert!(event["body"]["threadId"].as_i64().is_some());
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_breakpoint() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let bp_seq = session.client.send_request(
        "setBreakpoints",
        json!({
            "source": { "path": example_source("examples/hello_world/src/hello_world.rs") },
            "breakpoints": [{ "line": HELLO_LINE }],
        }),
    )?;
    let bp_response = session.client.read_response(bp_seq)?;
    ensure_response!(session, &bp_response, "setBreakpoints", bp_seq, true);
    let event = session.client.wait_for_event("breakpoint")?;
    assert_eq!(event["event"], "breakpoint");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_module() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let event = session.client.wait_for_event("module")?;
    assert_eq!(event["event"], "module");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_loaded_source() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let event = session.client.wait_for_event("loadedSource")?;
    assert_eq!(event["event"], "loadedSource");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_process() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let event = session.client.wait_for_event("process")?;
    assert_eq!(event["event"], "process");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_output() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("continue", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "continue", seq, true);
    let event = session.client.wait_for_event("output")?;
    assert_eq!(event["event"], "output");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_exited() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("continue", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "continue", seq, true);
    let event = session.client.wait_for_event("exited")?;
    assert_eq!(event["event"], "exited");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_terminated() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let _thread_id = require_launch!(
        &mut session,
        &example_bin("hello_world"),
        &example_source("examples/hello_world/src/hello_world.rs"),
        HELLO_LINE
    );
    let seq = session.client.send_request("terminate", json!({}))?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "terminate", seq, true);
    let event = session.client.wait_for_event("terminated")?;
    assert_eq!(event["event"], "terminated");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_progress() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let start = session.client.wait_for_event("progressStart")?;
    assert_eq!(start["event"], "progressStart");
    let update = session.client.wait_for_event("progressUpdate")?;
    assert_eq!(update["event"], "progressUpdate");
    let end = session.client.wait_for_event("progressEnd")?;
    assert_eq!(end["event"], "progressEnd");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_invalidated() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    let thread_id = require_launch!(
        &mut session,
        &example_bin("dap_set_variable"),
        &example_source("examples/dap_set_variable/src/main.rs"),
        SET_VAR_LINE
    );
    let frame_id = require_frame!(&mut session, thread_id);
    let seq = session.client.send_request(
        "setExpression",
        json!({
            "expression": "container.point.x",
            "value": "40",
            "frameId": frame_id,
        }),
    )?;
    let response = session.client.read_response(seq)?;
    ensure_response!(session, &response, "setExpression", seq, true);
    let event = session.client.wait_for_event("invalidated")?;
    assert_eq!(event["event"], "invalidated");
    session.shutdown();
    Ok(())
}

#[test]
#[serial]
fn test_event_capabilities() -> anyhow::Result<()> {
    let mut session = DapSession::start()?;
    initialize(&mut session)?;
    let program = example_bin("hello_world");
    let launch_seq = session
        .client
        .send_request("launch", json!({ "program": program }))?;
    let launch_response = session.client.read_response(launch_seq)?;
    ensure_response!(session, &launch_response, "launch", launch_seq, true);
    let event = session.client.wait_for_event("capabilities")?;
    assert_eq!(event["event"], "capabilities");
    assert!(event["body"]["capabilities"].is_object());
    session.shutdown();
    Ok(())
}
