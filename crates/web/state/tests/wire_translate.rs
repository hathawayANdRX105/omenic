//! C5.2b WireTranslator 测试：worker wire 事件 → AgentEvent 的配对与增量。
//!
//! 用例覆盖 omp 原始 wire 形状（`"type"` 标签 + `toolName`）；daemon R2 3.3
//! 转发层广播的 `WorkerEvent` serde 形状（`"event"` 标签 + `name` 字段）
//! 单独一条锁定——两种形状都必须能翻译，否则 G4 联调会静默丢事件。

use omenic_web_state::convert::WireTranslator;
use omenic_web_state::ui_state::AgentEvent;
use serde_json::json;

#[test]
fn start_end_pairs_share_synthetic_id() {
    let mut t = WireTranslator::new();
    assert_eq!(
        t.translate(&json!({ "type": "agent_start" })),
        Some(AgentEvent::TurnStart)
    );

    let call = t
        .translate(&json!({
            "type": "tool_execution_start",
            "toolName": "read",
            "input": { "path": "x" },
        }))
        .expect("start → ToolCall");
    let AgentEvent::ToolCall { id, name, args } = call else {
        panic!("expected ToolCall, got {call:?}");
    };
    assert_eq!(name, "read");
    assert_eq!(args, json!({ "path": "x" }));

    let res = t
        .translate(&json!({
            "type": "tool_execution_end",
            "toolName": "read",
            "result": { "ok": true },
        }))
        .expect("end → ToolResult");
    let AgentEvent::ToolResult {
        id: rid,
        name: rname,
        result,
    } = res
    else {
        panic!("expected ToolResult, got {res:?}");
    };
    // start/end 按 id 命中；result 序列化为 pretty JSON
    assert_eq!(rid, id);
    assert_eq!(rname, "read");
    assert_eq!(result, "{\n  \"ok\": true\n}");

    assert_eq!(
        t.translate(&json!({ "type": "agent_end" })),
        Some(AgentEvent::TurnEnd {
            stop_reason: "end_turn".into()
        })
    );
}

#[test]
fn unpaired_end_is_skipped() {
    let mut t = WireTranslator::new();
    // 没有任何在飞 start → end 直接跳过
    assert_eq!(
        t.translate(&json!({ "type": "tool_execution_end", "toolName": "read" })),
        None
    );
    // start → end 命中一次，再来的 end 无在飞记录 → 跳过
    let call = t
        .translate(&json!({
            "type": "tool_execution_start",
            "toolName": "bash",
            "input": { "command": "ls" },
        }))
        .unwrap();
    let AgentEvent::ToolCall { id, .. } = call else {
        panic!("expected ToolCall");
    };
    let res = t
        .translate(&json!({
            "type": "tool_execution_end",
            "toolName": "bash",
            "result": "src",
        }))
        .unwrap();
    let AgentEvent::ToolResult { id: rid, .. } = res else {
        panic!("expected ToolResult");
    };
    assert_eq!(rid, id);
    assert_eq!(
        t.translate(&json!({ "type": "tool_execution_end", "toolName": "bash" })),
        None
    );
}

#[test]
fn message_update_yields_incremental_deltas() {
    let mut t = WireTranslator::new();
    let a = t
        .translate(&json!({ "type": "message_update", "text": "你好" }))
        .expect("text delta");
    let b = t
        .translate(&json!({ "type": "message_update", "text": "，世界" }))
        .expect("text delta");
    assert_eq!(
        a,
        AgentEvent::AssistantText {
            delta: "你好".into()
        }
    );
    assert_eq!(
        b,
        AgentEvent::AssistantText {
            delta: "，世界".into()
        }
    );
    // 空增量不产生事件（避免 apply 侧补出空气泡）
    assert_eq!(
        t.translate(&json!({ "type": "message_update", "text": "" })),
        None
    );
}

#[test]
fn parallel_tools_do_not_cross_ids() {
    // 交错 start A / start B / end B / end A：LIFO 配对，各自命中自己的 id
    let mut t = WireTranslator::new();
    let a = t
        .translate(&json!({
            "type": "tool_execution_start",
            "toolName": "read",
            "input": { "path": "a" },
        }))
        .unwrap();
    let b = t
        .translate(&json!({
            "type": "tool_execution_start",
            "toolName": "grep",
            "input": {},
        }))
        .unwrap();
    let AgentEvent::ToolCall { id: id_a, .. } = a else {
        panic!("expected ToolCall");
    };
    let AgentEvent::ToolCall { id: id_b, .. } = b else {
        panic!("expected ToolCall");
    };
    assert_ne!(id_a, id_b, "并行工具的合成 id 不得相同");

    let rb = t
        .translate(&json!({
            "type": "tool_execution_end",
            "toolName": "grep",
            "result": "hit",
        }))
        .unwrap();
    let ra = t
        .translate(&json!({
            "type": "tool_execution_end",
            "toolName": "read",
            "result": "src",
        }))
        .unwrap();
    let AgentEvent::ToolResult { id: rid_b, .. } = rb else {
        panic!("expected ToolResult");
    };
    let AgentEvent::ToolResult { id: rid_a, .. } = ra else {
        panic!("expected ToolResult");
    };
    assert_eq!(rid_b, id_b, "后 start 的先 end（LIFO）");
    assert_eq!(rid_a, id_a);
}

#[test]
fn daemon_worker_event_shape_also_translates() {
    // daemon 转发层实际广播的 WorkerEvent serde 形状："event" 标签 + name 字段
    let mut t = WireTranslator::new();
    assert_eq!(
        t.translate(&json!({ "event": "agent_start" })),
        Some(AgentEvent::TurnStart)
    );
    assert_eq!(
        t.translate(&json!({ "event": "message", "text": "hi" })),
        Some(AgentEvent::AssistantText { delta: "hi".into() })
    );
    let call = t
        .translate(&json!({
            "event": "tool_execution_start",
            "name": "read",
            "input": { "path": "x" },
        }))
        .unwrap();
    let AgentEvent::ToolCall { id, name, args } = call else {
        panic!("expected ToolCall");
    };
    assert_eq!(name, "read");
    assert_eq!(args, json!({ "path": "x" }));
    let res = t
        .translate(&json!({
            "event": "tool_execution_end",
            "name": "read",
            "result": { "ok": true },
        }))
        .unwrap();
    let AgentEvent::ToolResult { id: rid, .. } = res else {
        panic!("expected ToolResult");
    };
    assert_eq!(rid, id);
    // 新一轮 start 后，result 缺失（None）的 end → 空串
    let call = t
        .translate(&json!({
            "event": "tool_execution_start",
            "name": "read",
            "input": {},
        }))
        .unwrap();
    let AgentEvent::ToolCall { id, .. } = call else {
        panic!("expected ToolCall");
    };
    let res = t
        .translate(&json!({
            "event": "tool_execution_end",
            "name": "read",
        }))
        .unwrap();
    let AgentEvent::ToolResult {
        id: rid, result, ..
    } = res
    else {
        panic!("expected ToolResult");
    };
    assert_eq!(rid, id);
    assert_eq!(result, "");
    assert_eq!(
        t.translate(&json!({ "event": "agent_end" })),
        Some(AgentEvent::TurnEnd {
            stop_reason: "end_turn".into()
        })
    );
}

#[test]
fn error_becomes_error_turn_end_and_unknown_ignored() {
    let mut t = WireTranslator::new();
    // error 帧（rpc 层合成的传输故障）：转成 error 停止原因的 TurnEnd，
    // 复用消费端收尾逻辑——静默吞掉会让 is_streaming 永久卡 true
    assert_eq!(
        t.translate(&json!({ "type": "error", "error": "boom" })),
        Some(AgentEvent::TurnEnd {
            stop_reason: "error".into()
        })
    );
    assert_eq!(
        t.translate(&json!({ "event": "error", "error": "boom" })),
        Some(AgentEvent::TurnEnd {
            stop_reason: "error".into()
        })
    );
    // 未知类型 / 空帧 / 缺工具名：丢弃
    assert_eq!(t.translate(&json!({ "type": "something_new" })), None);
    assert_eq!(t.translate(&json!({})), None);
    assert_eq!(
        t.translate(&json!({ "type": "tool_execution_start" })),
        None
    );
}

#[test]
fn same_name_nested_tools_pair_lifo() {
    // LIFO 的核心动机：同名嵌套（start read → start read → end → end）
    // 各自命中自己的 seq，不串 id
    let mut t = WireTranslator::new();
    let start = |n: u64| serde_json::json!({"type": "tool_execution_start", "name": "read", "input": {"n": n}});
    let end =
        serde_json::json!({"type": "tool_execution_end", "name": "read", "result": {"ok": true}});

    let Some(AgentEvent::ToolCall { id: id_a, .. }) = t.translate(&start(1)) else {
        panic!("first start 应产出 ToolCall");
    };
    let Some(AgentEvent::ToolCall { id: id_b, .. }) = t.translate(&start(2)) else {
        panic!("second start 应产出 ToolCall");
    };
    assert_ne!(id_a, id_b, "嵌套同名工具 id 不应相同");

    // 后 start 的先 end（LIFO）
    let Some(AgentEvent::ToolResult { id: rid_b, .. }) = t.translate(&end) else {
        panic!("first end 应产出 ToolResult");
    };
    assert_eq!(rid_b, id_b, "后 start 的应先 end");

    let Some(AgentEvent::ToolResult { id: rid_a, .. }) = t.translate(&end) else {
        panic!("second end 应产出 ToolResult");
    };
    assert_eq!(rid_a, id_a, "先 start 的应后 end");

    // 配对完毕：再来的 end 无在飞记录 → 跳过
    assert!(t.translate(&end).is_none(), "未配对 end 应被跳过");
}
