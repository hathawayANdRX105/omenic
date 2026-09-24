use agent_loop::{LoopEngine, Provider, run_agent_loop};
use protocol::{
    AbortSignal, LlmError, Message, RunError, RunState, RunStatus, ToolError, ToolResult, ToolSpec,
};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use tools_harness::ToolCatalog;

#[test]
fn loop_engine_default_values() {
    let engine = LoopEngine::default();
    assert_eq!(engine.max_turns, 0, "default max_turns is 0");
    assert_eq!(engine.model, "", "default model is empty");
}

fn assistant(content: &str) -> Message {
    Message {
        role: "assistant".to_string(),
        content: content.to_string(),
    }
}

fn tool_call(name: &str, args: Value) -> Message {
    Message {
        role: "tool_call".to_string(),
        content: json!({"name": name, "arguments": args}).to_string(),
    }
}

/// Scripted provider: pops replies in order, falling back to `repeat`.
/// Records the `messages` it saw on each call.
struct Scripted {
    replies: RefCell<VecDeque<Message>>,
    repeat: Message,
    seen: RefCell<Vec<Vec<Message>>>,
    fail: bool,
}

impl Scripted {
    fn new(replies: Vec<Message>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            repeat: assistant("done"),
            seen: RefCell::new(Vec::new()),
            fail: false,
        }
    }
    fn repeating(msg: Message) -> Self {
        Self {
            replies: RefCell::new(VecDeque::new()),
            repeat: msg,
            seen: RefCell::new(Vec::new()),
            fail: false,
        }
    }
    fn failing() -> Self {
        Self {
            replies: RefCell::new(VecDeque::new()),
            repeat: assistant("never"),
            seen: RefCell::new(Vec::new()),
            fail: true,
        }
    }
}

impl Provider for Scripted {
    fn call(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Message, LlmError>> + Send>> {
        let next = self
            .replies
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| self.repeat.clone());
        self.seen.borrow_mut().push(messages.to_vec());
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                Err(LlmError::Transport("stub down".to_string()))
            } else {
                Ok(next)
            }
        })
    }
}

struct EchoTool;

impl tools_harness::Tool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "echo".to_string(),
            params_schema: json!({"type": "object"}),
        }
    }
    fn execute(&self, args: &Value, _abort: &AbortSignal) -> Result<ToolResult, ToolError> {
        Ok(ToolResult {
            output: args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            is_error: false,
        })
    }
}

fn catalog_with(tool: Arc<dyn tools_harness::Tool>) -> ToolCatalog {
    let cat = ToolCatalog::new();
    cat.register(tool);
    cat
}

#[test]
fn loop_tool_call_then_end_turn() {
    let engine = LoopEngine {
        max_turns: 8,
        model: "test-model".to_string(),
    };
    let provider = Scripted::new(vec![
        tool_call("echo", json!({"text": "hi"})),
        assistant("all done"),
    ]);
    let executor = catalog_with(Arc::new(EchoTool));
    let run = run_agent_loop(&engine, &provider, &executor, &AbortSignal::new()).unwrap();

    assert_eq!(run.steps().len(), 2);
    assert_eq!(run.steps()[0].summary, "tool:echo");
    assert_eq!(run.steps()[1].status, RunStatus::EndTurn);
    assert_eq!(run.steps()[1].summary, "all done");
    assert_eq!(run.status(), RunStatus::EndTurn);

    // Invariant 1: the second provider call saw the backfilled tool result.
    let seen = provider.seen.borrow();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].is_empty(), "loop seeds no messages");
    assert_eq!(seen[1].len(), 2);
    assert_eq!(seen[1][1].role, "tool");
    assert_eq!(seen[1][1].content, "hi");
}

#[test]
fn loop_unknown_tool_backfills_error_and_continues() {
    let engine = LoopEngine {
        max_turns: 8,
        model: String::new(),
    };
    let provider = Scripted::new(vec![tool_call("nope", json!({})), assistant("ok")]);
    let executor = catalog_with(Arc::new(EchoTool));
    let run = run_agent_loop(&engine, &provider, &executor, &AbortSignal::new()).unwrap();
    assert_eq!(run.steps().len(), 2);
    let seen = provider.seen.borrow();
    assert!(
        seen[1][1].content.contains("unknown tool"),
        "got: {}",
        seen[1][1].content
    );
}

#[test]
fn loop_propagates_llm_failure() {
    let engine = LoopEngine {
        max_turns: 4,
        model: String::new(),
    };
    let provider = Scripted::failing();
    let executor = catalog_with(Arc::new(EchoTool));
    let err = run_agent_loop(&engine, &provider, &executor, &AbortSignal::new()).unwrap_err();
    match err {
        RunError::Llm(s) => assert_eq!(s, "stub down"),
        other => panic!("expected RunError::Llm, got {other:?}"),
    }
}

#[test]
fn loop_respects_abort_before_calling_provider() {
    let engine = LoopEngine {
        max_turns: 4,
        model: String::new(),
    };
    let provider = Scripted::new(vec![assistant("x")]);
    let executor = catalog_with(Arc::new(EchoTool));
    let abort = AbortSignal::new();
    abort.abort();
    let err = run_agent_loop(&engine, &provider, &executor, &abort).unwrap_err();
    assert!(matches!(err, RunError::Aborted));
    assert!(
        provider.seen.borrow().is_empty(),
        "provider must not be called"
    );
}

#[test]
fn loop_stops_at_max_turns() {
    let engine = LoopEngine {
        max_turns: 3,
        model: String::new(),
    };
    let provider = Scripted::repeating(tool_call("echo", json!({"text": "yap"})));
    let executor = catalog_with(Arc::new(EchoTool));
    let run = run_agent_loop(&engine, &provider, &executor, &AbortSignal::new()).unwrap();
    assert_eq!(run.steps().len(), 4, "3 tool steps + 1 MaxTurns step");
    assert_eq!(run.steps().last().unwrap().status, RunStatus::MaxTurns);
    assert_eq!(run.steps()[0].status, RunStatus::EndTurn);
    let seen = provider.seen.borrow();
    assert_eq!(seen.len(), 3, "exactly max_turns provider calls");
}
