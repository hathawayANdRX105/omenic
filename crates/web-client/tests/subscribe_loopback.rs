//! C5.2b — WebDaemon 订阅回环测试。
//!
//! 起真 daemon + mock omp（python3，抄 `infra/daemon/tests/event_push.rs`
//! 的脚本），`WebDaemon::subscribe_worker` 建立专用推送连接后经
//! `worker_prompt` 跑一次完整 turn，断言 `WireTranslator` 翻译出的
//! AgentEvent 序列：turn_start → assistant_text → tool_call →
//! tool_result → turn_end。

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use daemon::{Daemon, DaemonConfig};
use serde_json::json;
use web_client::daemon::WebDaemon;
use web_state::convert::WireTranslator;
use web_state::ui_state::AgentEvent;

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// mock omp：对 prompt 应答 success 后按 wire 顺序吐一次完整 turn 事件。
fn mock_omp(dir: &std::path::Path) -> PathBuf {
    let script = r#"#!/usr/bin/env python3
import json, sys

def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

out({"type": "ready", "protocolVersion": 2,
     "supportedProtocolVersions": [2],
     "maxFrameBytes": 65536, "maxReassembledFrameBytes": 262144})

for line in sys.stdin:
    try:
        req = json.loads(line)
    except json.JSONDecodeError:
        continue
    t = req.get("type")
    out({"type": "response", "id": req.get("id"), "success": True})
    if t == "prompt":
        for ev in [
            {"type": "agent_start"},
            {"type": "message_update", "text": "hello"},
            {"type": "tool_execution_start", "toolName": "read", "input": {"path": "x"}},
            {"type": "tool_execution_end", "toolName": "read", "result": {"ok": True}},
            {"type": "agent_end"},
        ]:
            out(ev)
"#;
    let path = dir.join("mock-omp");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn subscribe_worker_receives_and_translates_full_turn() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());
    let socket = dir.path().join("daemon.sock");
    let db = dir.path().join("sessions.db");
    let mut daemon = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: omp.to_string_lossy().into_owned(),
        session_db_path: Some(db),
        orbit_model: None,
        cwd: dir.path().to_path_buf(),
        max_turns: 64,
        ..Default::default()
    })
    .expect("daemon start");

    let wd = WebDaemon::connect_to(&socket);
    assert!(wd.ping(), "daemon answers");

    // 先订阅再 prompt：worker 事件泵必须在 run 发出前就位
    let mut sub = wd.subscribe_worker().expect("subscribe worker");

    // prompt 阻塞调用放线程（与页面 on_send 同款调用形状）
    let prompter = {
        let wd = wd.clone();
        std::thread::spawn(move || {
            let _ = wd.worker_prompt("go");
        })
    };

    let mut translator = WireTranslator::new();
    let mut got: Vec<AgentEvent> = Vec::new();
    while got.len() < 5 {
        match sub
            .next_event(EVENT_TIMEOUT)
            .expect("subscription readable")
        {
            Some(frame) => {
                assert_eq!(frame.topic, "worker", "推送主题应为 worker");
                if let Some(ev) = translator.translate(&frame.event) {
                    got.push(ev);
                }
            }
            None => panic!("timed out after {got:?}, expected 5 events"),
        }
    }

    // daemon 转发层广播 WorkerEvent serde 形状；translator 输出与
    // mock::stream_reply 同款的 AgentEvent 序列，id 按同名 LIFO 配对合成
    assert_eq!(
        got,
        vec![
            AgentEvent::TurnStart,
            AgentEvent::AssistantText {
                delta: "hello".into()
            },
            AgentEvent::ToolCall {
                id: "read-1".into(),
                name: "read".into(),
                args: json!({ "path": "x" }),
            },
            AgentEvent::ToolResult {
                id: "read-1".into(),
                name: "read".into(),
                result: "{\n  \"ok\": true\n}".into(),
            },
            AgentEvent::TurnEnd {
                stop_reason: "end_turn".into()
            },
        ]
    );

    prompter.join().unwrap();
    daemon.shutdown();
}
