//! Plan mode end-to-end through the real daemon socket.
//!
//! The observable contract, in order: `/plan` between turns flips plan mode
//! without an LLM round; a prompt under plan mode whose model turn calls
//! `exit_plan_mode` parks a plan-review question on the `user.question`
//! surface; `user.answer` with Approve resolves it, commits the exit, and
//! the follow-up request carries the tool result; plan mode reads back off.
//!
//! Only the full `Daemon::start` -> dispatch -> orbit engine -> plan-mode
//! plugin -> broker chain produces this sequence, so a pass here is the
//! batch's smoke: the feature works for a real client over a real socket.

mod common;

use std::thread;
use std::time::Duration;

use common::{MockOpenAi, daemon_cfg, drain_events, one_text_turn, prompt, sse_tool_call};
use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use tempfile::tempdir;

/// Poll `user.question.pending` until the plan-review question shows up.
/// The engine's run thread blocks inside `exit_plan_mode` until the broker
/// resolves it, so the question appears asynchronously relative to the
/// prompt ack.
fn wait_for_plan_review(client: &DaemonClient) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let pending: Vec<Value> = client
            .call(Command::UserQuestionPending, json!({}))
            .expect("pending query");
        if let Some(first) = pending.first() {
            return first.clone();
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("plan-review question never appeared");
}

#[test]
fn plan_mode_review_round_trip_through_the_daemon() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(dir.path().join("AGENTS.md"), "# smoke\n").expect("AGENTS.md");
    let mock = MockOpenAi::start(vec![
        // Turn 1: the model presents the plan through exit_plan_mode.
        sse_tool_call(
            "exit_plan_mode",
            &json!({ "plan": "# The plan\n\n1. do the thing" }).to_string(),
            true,
        ),
        // Turn 2 (after approval): the run wraps up.
        one_text_turn("done"),
    ]);
    let mut daemon = Daemon::start(daemon_cfg(dir.path(), &mock, 4)).expect("daemon start");
    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");

    // 1. `/plan` between turns: plan mode on, no LLM round spent.
    let on: Value = client
        .call(Command::WorkerPrompt, json!({ "message": "/plan" }))
        .expect("plan on ack");
    assert_eq!(on["plan_mode"], json!(true));
    assert_eq!(on["prompted"], json!(false));

    // 2. A real prompt under plan mode; the model calls exit_plan_mode.
    prompt(&client, "design the thing");
    let question = wait_for_plan_review(&client);
    // QuestionIntent is internally tagged: the tag lives inside the
    // `intent` object, not at the item's top level.
    assert_eq!(question["intent"]["type"], json!("plan_review"));
    let options = question["options"].as_array().expect("options array");
    assert_eq!(options.len(), 3);
    assert_eq!(options[0]["label"], json!("Approve"));
    assert_eq!(options[1]["label"], json!("Reject"));
    assert_eq!(options[2]["label"], json!("Dismiss"));

    // 3. Approve: the question resolves and the run completes.
    let answered: Value = client
        .call(
            Command::UserAnswer,
            json!({ "id": question["id"], "answer": { "kind": "select", "index": 0 } }),
        )
        .expect("approve");
    assert_eq!(answered["answered"], json!(true));
    drain_events(&mut sub);

    // 4. End state: plan mode reads back off (a bare `/plan` reports false).
    let off: Value = client
        .call(Command::WorkerPrompt, json!({ "message": "/plan" }))
        .expect("plan off ack");
    assert_eq!(off["plan_mode"], json!(false));
    let still_pending: Vec<Value> = client
        .call(Command::UserQuestionPending, json!({}))
        .expect("pending query");
    assert!(
        still_pending.is_empty(),
        "question not cleared: {still_pending:?}"
    );

    // 5. The tool result reached the follow-up request the loop sent.
    let bodies = mock.received();
    assert!(
        bodies.len() >= 2,
        "expected a follow-up request after exit_plan_mode; got {}",
        bodies.len()
    );
    assert!(
        bodies[1].contains("Plan approved"),
        "the approval result never reached the model: {}",
        bodies[1]
    );

    drop(client);
    daemon.shutdown();
}
