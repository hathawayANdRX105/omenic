use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use jobs::LocalJobRegistry;
use serde_json::{Value, json};
use terminal::TerminalRegistry;
use tools::Tool;
use tools_harness::jobs_terminal::{
    JobsKill, JobsList, JobsStart, JobsWait, TerminalCreate, TerminalKill, TerminalRead,
    TerminalWrite,
};
fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// Pull the first word out of a tool's reply after `prefix`.
///
/// Every tool answers in prose ("started job job-1 ..."), so a test has to
/// recover the id the tool itself chose. Whitespace-delimited so `job-1` never
/// matches a slice of `job-12`.
fn word_after(text: &str, prefix: &str) -> String {
    let i = text
        .find(prefix)
        .unwrap_or_else(|| panic!("`{prefix}` not in: {text}"));
    let rest = &text[i + prefix.len()..];
    rest.split_whitespace()
        .next()
        .unwrap_or_else(|| panic!("nothing after `{prefix}` in: {text}"))
        .to_string()
}

/// Poll a condition until it holds or a short budget runs out.
///
/// The tools are synchronous but the pty reader delivers bytes asynchronously,
/// so `terminal_read` can legitimately be one read too early. Polling turns
/// that into a deterministic wait; a fixed sleep would race or stall.
fn wait_for<F: FnMut() -> bool>(mut cond: F, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn jobs_start_wait_list_kill_round_trip() {
    let jobs = Arc::new(LocalJobRegistry::new());
    let start = JobsStart::new(Arc::clone(&jobs));
    let wait = JobsWait::new(Arc::clone(&jobs));
    let list = JobsList::new(Arc::clone(&jobs));
    let kill = JobsKill::new(Arc::clone(&jobs));

    // echo exits on its own, so jobs_wait observes a real completion rather
    // than a timeout the test would have to interpret.
    let reply = start
        .execute(
            &json!({"command": "echo hello", "label": "greeter"}),
            &never(),
        )
        .expect("jobs_start must accept a command");
    assert!(reply.contains("started job"), "got: {reply}");
    let id = word_after(&reply, "started job ");
    assert!(id.starts_with("job-"), "parsed id `{id}` from: {reply}");

    let waited = wait
        .execute(&json!({"id": id, "timeout_ms": 5_000}), &never())
        .expect("jobs_wait must return the finished job's output");
    assert!(waited.contains("hello"), "jobs_wait output: {waited}");

    // The job is done; the listing still shows it as a record.
    let listed = list
        .execute(&json!({}), &never())
        .expect("jobs_list must answer");
    assert!(listed.contains(&id), "jobs_list missing {id}: {listed}");

    // Killing a finished job is a documented no-op that reports the state.
    let after = kill
        .execute(&json!({"id": id}), &never())
        .expect("jobs_kill must answer");
    assert!(
        after.contains("already") || after.contains("completed"),
        "expected a no-op report, got: {after}"
    );
}

#[test]
fn jobs_wait_unknown_id_is_an_error_not_a_panic() {
    let jobs = Arc::new(LocalJobRegistry::new());
    let wait = JobsWait::new(Arc::clone(&jobs));
    let err = wait
        .execute(&json!({"id": "job-9999"}), &never())
        .expect_err("unknown id must be an error");
    let msg = err.to_string();
    assert!(msg.contains("job-9999"), "error must name the id: {msg}");
}

#[test]
fn jobs_kill_stops_a_running_job() {
    let jobs = Arc::new(LocalJobRegistry::new());
    let start = JobsStart::new(Arc::clone(&jobs));
    let kill = JobsKill::new(Arc::clone(&jobs));
    let list = JobsList::new(jobs);

    // A job that outlives the call: kill must actually stop it.
    let reply = start
        .execute(
            &json!({"command": "sleep 60", "label": "sleeper"}),
            &never(),
        )
        .expect("jobs_start must accept a command");
    let id = word_after(&reply, "started job ");

    let killed = kill
        .execute(&json!({"id": id}), &never())
        .expect("jobs_kill must answer");
    assert_eq!(killed, format!("killed job {id}"), "got: {killed}");

    // The state must be visible in the listing immediately.
    wait_for(
        || {
            let rows = list
                .execute(&json!({}), &never())
                .expect("jobs_list must answer");
            rows.contains(&format!("{id}")) && rows.contains("killed")
        },
        "job to reach killed state",
    );
}

#[test]
fn terminal_create_write_read_kill_round_trip() {
    let terminals = Arc::new(TerminalRegistry::new());
    let create = TerminalCreate::new(Arc::clone(&terminals));
    let write = TerminalWrite::new(Arc::clone(&terminals));
    let read = TerminalRead::new(Arc::clone(&terminals));
    let kill = TerminalKill::new(terminals);

    // `sh` rather than `bash`: it exists everywhere and boots with no rc file.
    let reply = create
        .execute(&json!({"shell": "sh"}), &never())
        .expect("terminal_create must open a session");
    assert!(reply.contains("terminal "), "got: {reply}");
    let id = word_after(&reply, "terminal ");
    assert!(id.starts_with("term-"), "parsed id `{id}` from: {reply}");

    write
        .execute(&json!({"id": id, "data": "echo term-ok\n"}), &never())
        .expect("terminal_write must accept input");

    // The reader thread delivers asynchronously; poll until the marker lands.
    wait_for(
        || {
            let out = read
                .execute(&json!({"id": id, "timeout_ms": 500}), &never())
                .expect("terminal_read must answer");
            out.contains("term-ok")
        },
        "terminal to echo term-ok",
    );

    let closed = kill
        .execute(&json!({"id": id}), &never())
        .expect("terminal_kill must answer");
    assert!(closed.contains(&id), "kill must name the id: {closed}");
}

#[test]
fn terminal_create_rejects_cols_out_of_range() {
    let terminals = Arc::new(TerminalRegistry::new());
    let create = TerminalCreate::new(Arc::clone(&terminals));

    let err = create
        .execute(&json!({"cols": 70_000}), &never())
        .expect_err("cols above u16 must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("cols"), "error must name cols: {msg}");

    // In-range values still open a session, and are reported back.
    let ok = create
        .execute(&json!({"cols": 120, "rows": 40}), &never())
        .expect("cols in range must open a session");
    assert!(ok.contains("120x40"), "got: {ok}");
}

#[test]
fn tool_names_and_specs_are_wired() {
    // The daemon dispatches by name, so a struct whose `name()` disagrees with
    // the documented tool name is a silent break.
    let jobs = Arc::new(LocalJobRegistry::new());
    let terminals = Arc::new(TerminalRegistry::new());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(JobsStart::new(Arc::clone(&jobs))),
        Box::new(JobsWait::new(Arc::clone(&jobs))),
        Box::new(JobsList::new(Arc::clone(&jobs))),
        Box::new(JobsKill::new(jobs)),
        Box::new(TerminalCreate::new(Arc::clone(&terminals))),
        Box::new(TerminalWrite::new(Arc::clone(&terminals))),
        Box::new(TerminalRead::new(Arc::clone(&terminals))),
        Box::new(TerminalKill::new(terminals)),
    ];
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        [
            "jobs_start",
            "jobs_wait",
            "jobs_list",
            "jobs_kill",
            "terminal_create",
            "terminal_write",
            "terminal_read",
            "terminal_kill",
        ]
    );
    // An empty-object schema would make every call a validation failure.
    for t in &tools {
        assert!(
            t.parameters().get("type").and_then(Value::as_str) == Some("object"),
            "{} has no object schema",
            t.name()
        );
    }
}

#[test]
fn session_tools_registrates_every_tool() {
    use tools_harness::jobs_terminal::{SESSION_TOOL_NAMES, session_tools};

    let jobs = Arc::new(LocalJobRegistry::new());
    let terminals = Arc::new(TerminalRegistry::new());
    let tools = session_tools(jobs, terminals);
    assert_eq!(tools.len(), SESSION_TOOL_NAMES.len());
    for (tool, name) in tools.iter().zip(SESSION_TOOL_NAMES.iter()) {
        assert_eq!(tool.name(), *name);
    }
}
