#![allow(dead_code)] // consumed by runner in M3
//! Worker lifecycle: spawn / steer / abort / event stream.
//!
//! A Worker wraps an `crate::client::Client` connected to `omp --mode rpc` and provides
//! a task-oriented interface for agent interaction: send a prompt, read events,
//! steer the agent, and abort when done.
//!
//! ## Lifecycle state map (#46)
//!
//! ```text
//! spawn ──► ready handshake ──► negotiate v2 ──► idle
//!           (crate::client::Client::new)  (set_auto_retry/   │
//!                                compaction off)   ├─► prompt(msg) ──┐
//!                                                  │                 ▼
//!                                                  │            event stream
//!                                                  │        (read_event / events())
//!                                                  │                 │
//!                                                  ├─► steer(msg) ◄──┘ (between events)
//!                                                  ├─► abort() ──► killed
//!                                                  └─► Drop     ──► kill process group
//! ```
//!
//! Error paths: spawn/handshake/negotiation failure → `Worker::new` errs;
//! transport death mid-call → `RpcError::ProcessExited` from prompt/steer/
//! read_event; `Drop` always kills the child even after errors.
//!
//! ## Example
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

use serde::Serialize;
use serde_json::Value;

/// Events emitted by the worker during agent execution.
#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
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
    client: crate::client::Client,
}

impl Worker {
    /// Spawn a new worker process (`omp --mode rpc`).
    ///
    /// Blocks until the initial `ready` handshake completes.
    pub fn new(omp_path: &str) -> Result<Self, crate::client::RpcError> {
        let client = crate::client::Client::new(omp_path)?;
        Ok(Worker { client })
    }

    /// Spawn with a connect timeout and auto-reconnect retry count.
    pub fn new_with_opts(
        omp_path: &str,
        connect_timeout: Option<std::time::Duration>,
        max_retries: u32,
    ) -> Result<Self, crate::client::RpcError> {
        let client = crate::client::Client::new_with_opts(omp_path, connect_timeout, max_retries)?;
        Ok(Worker { client })
    }

    /// Reconnect the underlying client (kill + respawn + renegotiate).
    pub fn reconnect(&mut self) -> Result<(), crate::client::RpcError> {
        self.client.reconnect()
    }

    /// Send a ping to check liveness. Returns Ok if the process responds.
    pub fn ping(&mut self) -> Result<(), crate::client::RpcError> {
        let id = self.client.next_id_str();
        let req = crate::client::Request::new("ping").with_id(&id).done();
        self.client.send(&req)?;
        Ok(())
    }

    /// Register tool definitions that omp may invoke through this client.
    pub fn register_external_tools(
        &mut self,
        defs: Vec<adaptor::ToolDef>,
    ) -> Result<(), crate::client::RpcError> {
        let id = self.client.next_id_str();
        let req = crate::client::Request::new("register_external_tools")
            .with_id(&id)
            .with_field("tools", defs)
            .done();
        let response = self.client.send(&req)?;
        if response.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(crate::client::RpcError::Protocol(
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("register_external_tools failed")
                    .to_string(),
            ))
        }
    }

    /// Send a prompt to the agent (initial task brief or follow-up).
    ///
    /// Returns the response data.  The agent will subsequently emit events
    /// that can be read via `read_event()`.
    pub fn prompt(&mut self, message: &str) -> Result<Value, crate::client::RpcError> {
        let id = self.client.next_id_str();
        let req = crate::client::Request::new("prompt")
            .with_id(&id)
            .with_field("message", message)
            .done();
        self.client.send(&req)
    }

    /// Steer the running agent with an instruction.
    pub fn steer(&mut self, message: &str) -> Result<Value, crate::client::RpcError> {
        let id = self.client.next_id_str();
        let req = crate::client::Request::new("steer")
            .with_id(&id)
            .with_field("message", message)
            .done();
        self.client.send(&req)
    }

    /// Abort the current agent session.
    pub fn abort(&mut self) -> Result<Value, crate::client::RpcError> {
        let id = self.client.next_id_str();
        let req = crate::client::Request::new("abort").with_id(&id).done();
        self.client.send(&req)
    }

    /// PID of the underlying omp worker process (its process group leader).
    pub fn child_pid(&self) -> u32 {
        self.client.child_pid()
    }

    /// Read the next event from the agent, blocking until one arrives.
    ///
    /// Returns `None` when the agent has no more events and the session is
    /// idle (i.e. a `prompt` or `steer` response was received without
    /// subsequent agent events).  Callers should loop until `None` and then
    /// decide whether to prompt again or abort.
    pub fn read_event(&mut self) -> Result<Option<WorkerEvent>, crate::client::RpcError> {
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
