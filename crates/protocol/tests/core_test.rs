use protocol::{AbortSignal, RunId, RunState, RunStatus, StepId, ToolResult, ToolSpec, new_run};

#[test]
fn new_run_is_empty_with_given_id() {
    let run = new_run(RunId::new("run-7"));
    assert_eq!(run.id, RunId::new("run-7"));
    assert!(run.steps().is_empty(), "new_run must start with zero steps");
    assert_eq!(
        run.status(),
        RunStatus::EndTurn,
        "a stepless run reports EndTurn"
    );
}

#[test]
fn abort_signal_starts_unset() {
    let sig = AbortSignal::new();
    assert!(!sig.is_aborted(), "fresh AbortSignal must not be aborted");
    sig.abort();
    assert!(sig.is_aborted(), "aborted signal must report aborted");
}

#[test]
fn run_status_variants() {
    let statuses = [
        RunStatus::EndTurn,
        RunStatus::MaxTokens,
        RunStatus::Aborted,
        RunStatus::Error,
        RunStatus::MaxTurns,
    ];
    assert_eq!(statuses.len(), 5, "RunStatus must have exactly 5 variants");
}

#[test]
fn tool_spec_roundtrip() {
    let spec = ToolSpec {
        name: "bash".to_string(),
        description: "Run a shell command".to_string(),
        params_schema: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
    };
    let s = serde_json::to_string(&spec).unwrap();
    let back: ToolSpec = serde_json::from_str(&s).unwrap();
    assert_eq!(back.name, "bash");
    assert_eq!(back.description, "Run a shell command");
}

#[test]
fn tool_result_carries_error_flag() {
    let r = ToolResult {
        output: "ok".to_string(),
        is_error: false,
    };
    assert!(!r.is_error);
    let e = ToolResult {
        output: "boom".to_string(),
        is_error: true,
    };
    assert!(e.is_error);
}

#[test]
fn run_id_display() {
    let id = RunId::new("run-123");
    assert_eq!(format!("{}", id), "run-123");
}

#[test]
fn step_id_display() {
    let id = StepId::new("step-456");
    assert_eq!(id.as_str(), "step-456");
}
