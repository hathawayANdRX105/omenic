//! R2 3.1 — `AgentEvent` is the frozen cross-crate event contract.
//! Every variant must survive a JSON round-trip unchanged, and the wire
//! shape (internally tagged `"type"`, snake_case) must stay pinned: daemon
//! subscribers and the web UI parse these strings.

use llm::ToolCallSpec;
use orbit::{AgentEvent, TurnStop};
use serde_json::{Value, json};

fn round_trip(event: &AgentEvent) -> Value {
    let wire = serde_json::to_value(event).expect("serialize AgentEvent");
    let back: AgentEvent = serde_json::from_value(wire.clone()).expect("deserialize AgentEvent");
    assert_eq!(&back, event, "round-trip changed the event: {wire}");
    wire
}

#[test]
fn every_variant_round_trips() {
    let events = [
        AgentEvent::TurnStart,
        AgentEvent::AssistantText {
            delta: "你好 🌍".into(),
        },
        AgentEvent::ToolCall(ToolCallSpec {
            id: "call-1".into(),
            name: "read".into(),
            args: json!({"path": "src/lib.rs"}),
        }),
        AgentEvent::ToolStart {
            id: "call-1".into(),
            name: "read".into(),
        },
        AgentEvent::ToolResult {
            id: "call-1".into(),
            name: "read".into(),
            result: "ok".into(),
        },
        AgentEvent::TurnEnd {
            stop_reason: TurnStop::EndTurn,
        },
    ];
    for event in &events {
        round_trip(event);
    }
}

#[test]
fn wire_shape_is_pinned() {
    assert_eq!(
        round_trip(&AgentEvent::TurnStart),
        json!({ "type": "turn_start" })
    );
    assert_eq!(
        round_trip(&AgentEvent::AssistantText { delta: "hi".into() }),
        json!({ "type": "assistant_text", "delta": "hi" })
    );
    assert_eq!(
        round_trip(&AgentEvent::ToolCall(ToolCallSpec {
            id: "c".into(),
            name: "t".into(),
            args: json!({"k": 1}),
        })),
        json!({ "type": "tool_call", "id": "c", "name": "t", "args": {"k": 1} })
    );
    assert_eq!(
        round_trip(&AgentEvent::ToolStart {
            id: "c".into(),
            name: "t".into(),
        }),
        json!({ "type": "tool_start", "id": "c", "name": "t" })
    );
    assert_eq!(
        round_trip(&AgentEvent::ToolResult {
            id: "c".into(),
            name: "t".into(),
            result: "r".into(),
        }),
        json!({ "type": "tool_result", "id": "c", "name": "t", "result": "r" })
    );
    assert_eq!(
        round_trip(&AgentEvent::TurnEnd {
            stop_reason: TurnStop::MaxTurns,
        }),
        json!({ "type": "turn_end", "stop_reason": "max_turns" })
    );
}

#[test]
fn all_turn_stop_reasons_round_trip() {
    let cases = [
        (TurnStop::EndTurn, "end_turn"),
        (TurnStop::MaxTokens, "max_tokens"),
        (TurnStop::Aborted, "aborted"),
        (TurnStop::Error, "error"),
        (TurnStop::MaxTurns, "max_turns"),
    ];
    for (reason, name) in cases {
        let wire = serde_json::to_value(reason).unwrap();
        assert_eq!(wire.as_str().unwrap(), name);
        let back: TurnStop = serde_json::from_value(wire).unwrap();
        assert_eq!(back, reason);
    }
}
