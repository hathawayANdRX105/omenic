//! 后台作业完成经 aside 通道投给模型（端到端）。
//!
//! 链路人肉可查：jobs 完成 → `on_job_done` 钩子 → aside 队列 → orbit 在
//! step 边界 drain → 下一次 provider 请求体里出现 `[background] …`。
//!
//! Red when: 钩子没接上（模型永远不知道后台活干完了，只能自己轮询
//! `jobs_list`），或 aside 被接成 follow-up（模型说完又被逼一轮，turn 不收）。

mod common;

use std::time::Duration;

use common::{MockOpenAi, daemon_cfg, drain_events, one_text_turn, prompt, sse_tool_call};

#[test]
fn finished_job_shows_up_in_the_next_request() {
    let dir = tempfile::tempdir().expect("temp dir");
    // Round 1 of turn 1: the model starts a background job, then speaks.
    // Turn 2: the model must already see the aside.
    let mock = MockOpenAi::start(vec![
        format!(
            "{}{}",
            sse_tool_call(
                "jobs_start",
                r#"{"command":"sleep 0.2","label":"aside-probe"}"#,
                true
            ),
            one_text_turn("started it"),
        ),
        one_text_turn("noted"),
    ]);
    let mut server = daemon::Daemon::start(daemon_cfg(dir.path(), &mock, 8)).expect("start daemon");
    let client = daemon::DaemonClient::connect_to(&server.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");

    prompt(&client, "run `sleep 0.2` in the background");
    // Turn 1 must finish before turn 2: orbit prompts queue on the run thread.
    drain_events(&mut sub);
    // The job needs a moment to reach a terminal state.
    std::thread::sleep(Duration::from_millis(400));
    prompt(&client, "anything else?");
    drain_events(&mut sub);

    let bodies = mock.received();
    let second = bodies
        .iter()
        .find(|b| b.contains("anything else?"))
        .expect("a model request carrying the second user turn");
    assert!(
        second.contains("[background] job"),
        "the finished job must reach the model as an aside; second turn was: {second}"
    );
    assert!(
        second.contains("aside-probe"),
        "the aside should carry the job label; second turn was: {second}"
    );
    // The aside is a user-role message, not assistant text: a model must not
    // read its own completion notice as something it said.
    let parsed: serde_json::Value = serde_json::from_str(second).expect("request body is json");
    let aside = parsed["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|m| {
            m["role"] == serde_json::json!("user")
                && m["content"]
                    .as_str()
                    .is_some_and(|s| s.contains("[background] job"))
        })
        .expect("aside present as a user message");
    assert!(aside["content"].as_str().unwrap().contains("aside-probe"));

    drop(client);
    server.shutdown();
}
