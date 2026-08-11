#![allow(dead_code)] // consumed by runner in M3
//! Worker lifecycle: spawn / steer / abort / event stream.
//!
//! A Worker wraps an `rpc::Client` connected to `omp --mode rpc` and provides
//! a task-oriented interface for agent interaction: send a prompt, read events,
//! steer the agent, and abort when done.
//!
//! ## Lifecycle
//! ```ignore
//! let mut worker = worker::Worker::new("/usr/bin/omp")?;
//! worker.prompt("Write a design doc for the auth module")?;
//! for event in worker.events() {
//!     match event {
//!         WorkerEvent::Message { text } => println!("{text}"),
//!         WorkerEvent::AgentEnd => break,
//!         _ => {}
//!     }
//! }
//! worker.abort()?;
//! ```

use serde_json::Value;

use crate::rpc;

/// Events emitted by the worker during agent execution.
#[derive(Debug)]
pub enum WorkerEvent {
    /// Agent started processing.
    AgentStart,
    /// A text message from the agent.
    Message { text: String },
    /// A tool execution (tool name, args, result).
    ToolExecution {
        name: String,
        input: Value,
        result: Option<Value>,
    },
    /// Agent finished (session idle).
    AgentEnd,
    /// An unhandled event type (raw value for forward-compat).
    Unknown(Value),
}

/// A worker session connected to an omp agent.
pub struct Worker {
    client: rpc::Client,
}

impl Worker {
    /// Spawn a new worker process (`omp --mode rpc`).
    ///
    /// Blocks until the initial `ready` handshake completes.
    pub fn new(omp_path: &str) -> Result<Self, rpc::RpcError> {
        let client = rpc::Client::new(omp_path)?;
        Ok(Worker { client })
    }

    /// Send a prompt to the agent (initial task brief or follow-up).
    ///
    /// Returns the response data.  The agent will subsequently emit events
    /// that can be read via `read_event()`.
    pub fn prompt(&mut self, message: &str) -> Result<Value, rpc::RpcError> {
        let id = self.client.next_id_str();
        let req = rpc::Request::new("prompt")
            .with_id(&id)
            .with_field("message", message)
            .done();
        self.client.send(&req)
    }

    /// Steer the running agent with an instruction.
    pub fn steer(&mut self, message: &str) -> Result<Value, rpc::RpcError> {
        let id = self.client.next_id_str();
        let req = rpc::Request::new("steer")
            .with_id(&id)
            .with_field("message", message)
            .done();
        self.client.send(&req)
    }

    /// Abort the current agent session.
    pub fn abort(&mut self) -> Result<Value, rpc::RpcError> {
        let id = self.client.next_id_str();
        let req = rpc::Request::new("abort").with_id(&id).done();
        self.client.send(&req)
    }

    /// Read the next event from the agent, blocking until one arrives.
    ///
    /// Returns `None` when the agent has no more events and the session is
    /// idle (i.e. a `prompt` or `steer` response was received without
    /// subsequent agent events).  Callers should loop until `None` and then
    /// decide whether to prompt again or abort.
    pub fn read_event(&mut self) -> Result<Option<WorkerEvent>, rpc::RpcError> {
        let raw = self.client.next_frame_raw()?;
        let ty = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match ty {
            "response" => {
                // A response frame means the previous command completed.
                // The agent may still have events queued after this, but
                // for MVP we treat it as a yield point.
                Ok(None)
            }
            "agent_start" => Ok(Some(WorkerEvent::AgentStart)),
            "agent_end" => Ok(Some(WorkerEvent::AgentEnd)),
            "message_start" | "message_update" => {
                let text = raw
                    .pointer("/message/content")
                    .or_else(|| raw.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(Some(WorkerEvent::Message { text }))
            }
            "tool_execution" | "tool_execution_start" => {
                let name = raw
                    .get("toolName")
                    .or_else(|| raw.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let input = raw.get("input").cloned().unwrap_or(Value::Null);
                Ok(Some(WorkerEvent::ToolExecution {
                    name,
                    input,
                    result: None,
                }))
            }
            "tool_execution_end" => {
                let name = raw
                    .get("toolName")
                    .or_else(|| raw.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let result = raw.get("result").cloned();
                Ok(Some(WorkerEvent::ToolExecution {
                    name,
                    input: Value::Null,
                    result,
                }))
            }
            _ => Ok(Some(WorkerEvent::Unknown(raw))),
        }
    }

    /// Convenience iterator: yields events until `None` (response received).
    ///
    /// Consume with `for event in worker.events() { ... }`.
    pub fn events(&mut self) -> WorkerEvents<'_> {
        WorkerEvents { worker: self }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Client's Drop sends abort + kills the process.
    }
}

/// Iterator over worker events.
pub struct WorkerEvents<'a> {
    worker: &'a mut Worker,
}

impl<'a> Iterator for WorkerEvents<'a> {
    type Item = WorkerEvent;

    fn next(&mut self) -> Option<Self::Item> {
        match self.worker.read_event() {
            Ok(Some(event)) => Some(event),
            Ok(None) => None,
            Err(e) => {
                // Return a synthetic event so the caller can handle the error.
                Some(WorkerEvent::Unknown(serde_json::json!({
                    "type": "error",
                    "error": e.to_string(),
                })))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_event_agent_start() {
        let raw = serde_json::json!({"type": "agent_start"});
        let event = parse_event_test(&raw);
        assert!(matches!(event, WorkerEvent::AgentStart));
    }

    #[test]
    fn worker_event_agent_end() {
        let raw = serde_json::json!({"type": "agent_end"});
        let event = parse_event_test(&raw);
        assert!(matches!(event, WorkerEvent::AgentEnd));
    }

    #[test]
    fn worker_event_message() {
        let raw = serde_json::json!({
            "type": "message_update",
            "message": {"content": "Hello from agent"}
        });
        let event = parse_event_test(&raw);
        match event {
            WorkerEvent::Message { text } => {
                assert_eq!(text, "Hello from agent");
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn worker_event_message_plain_text() {
        let raw = serde_json::json!({
            "type": "message_update",
            "text": "plain text reply"
        });
        let event = parse_event_test(&raw);
        match event {
            WorkerEvent::Message { text } => {
                assert_eq!(text, "plain text reply");
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn worker_event_tool_execution() {
        let raw = serde_json::json!({
            "type": "tool_execution",
            "toolName": "bash",
            "input": {"command": "ls"}
        });
        let event = parse_event_test(&raw);
        match event {
            WorkerEvent::ToolExecution { name, input, .. } => {
                assert_eq!(name, "bash");
                assert_eq!(input["command"], "ls");
            }
            _ => panic!("expected ToolExecution"),
        }
    }

    #[test]
    fn worker_event_unknown() {
        let raw = serde_json::json!({"type": "some_future_event", "data": 42});
        let event = parse_event_test(&raw);
        assert!(matches!(event, WorkerEvent::Unknown(_)));
    }

    #[test]
    fn worker_abort_command_format() {
        // Simulate serialization of the abort command
        let req = rpc::Request::new("abort").with_id("test-1").done();
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["type"], "abort");
        assert_eq!(json["id"], "test-1");
    }

    /// Live smoke against a real omp binary. Run with:
    /// `cargo test -- --ignored worker::tests::live_worker`.
    /// Requires `omp` on PATH.
    #[test]
    #[ignore]
    fn live_worker() {
        let mut worker = Worker::new("omp").expect("spawn + ready failed");
        // Simple local-only prompt — no agent turn required, but exercises
        // the full prompt -> response path.
        let resp = worker
            .prompt("Reply with exactly: pong")
            .unwrap_or_else(|e| {
                // If prompt fails due to unsupported command, we can't proceed.
                panic!("prompt failed: {e}")
            });
        println!("prompt response: {resp:?}");

        // Read a few events; should terminate or yield None.
        let mut saw_any = false;
        for event in worker.events().take(10) {
            saw_any = true;
            println!("event: {event:?}");
            if matches!(event, WorkerEvent::AgentEnd) {
                break;
            }
        }
        // A local-only prompt may produce zero agent events — that's fine.
        let _ = &saw_any;

        // Abort is always available.
        let _ = worker.abort();
        println!("worker smoke ok");
    }

    /// Parse a raw JSON value as if it came from next_frame_raw.
    /// This mirrors the logic in Worker::read_event.
    fn parse_event_test(raw: &Value) -> WorkerEvent {
        let ty = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "response" => panic!("response should return None, not an event"),
            "agent_start" => WorkerEvent::AgentStart,
            "agent_end" => WorkerEvent::AgentEnd,
            "message_start" | "message_update" => {
                let text = raw
                    .pointer("/message/content")
                    .or_else(|| raw.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                WorkerEvent::Message { text }
            }
            "tool_execution" | "tool_execution_start" => {
                let name = raw
                    .get("toolName")
                    .or_else(|| raw.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let input = raw.get("input").cloned().unwrap_or(Value::Null);
                WorkerEvent::ToolExecution {
                    name,
                    input,
                    result: None,
                }
            }
            "tool_execution_end" => {
                let name = raw
                    .get("toolName")
                    .or_else(|| raw.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let result = raw.get("result").cloned();
                WorkerEvent::ToolExecution {
                    name,
                    input: Value::Null,
                    result,
                }
            }
            _ => WorkerEvent::Unknown(raw.clone()),
        }
    }
}
