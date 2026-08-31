//! The `task` tool: parallel read-only subagent exploration.
//!
//! One prompt → run inline. Multiple prompts → spawn one `std::thread` per
//! prompt and join via a channel. Wall-clock is capped at
//! `SUBAGENT_TIMEOUT_SECS`; on timeout the signal is flipped so all live
//! subagents abort together and the partial outputs are returned.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use adaptor::Model;
use orbit::HttpLlm;
use serde_json::{Value, json};
use tools::{Tool, ToolError};

use crate::config::{MAX_TURNS_DEFAULT, SUBAGENT_TIMEOUT_SECS};
use crate::runner::{SubagentError, run_subagent};

/// The `task` tool, registered opt-in by callers that want parallel
/// read-only exploration. NOT part of `tools::builtin_tools()`.
pub struct TaskTool;

impl Tool for TaskTool {
    fn name(&self) -> &'static str {
        "task"
    }

    fn description(&self) -> String {
        "并行运行多个只读子 agent 探索 prompt。每个子 agent 只能调用 read/grep/glob。\
         适用于「X 文件在哪」「Y 函数清单」类只读查询，替代主 agent 直接 read 大文件。\
         多 prompt 走 std::thread 并行，受 5min wall-clock 限制。"
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "One or more read-only exploration prompts."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Per-subagent loop turn cap (default 10)."
                },
                "backend": {
                    "description": "LLM model spec. Provide {api_key, model, base_url?} \
                                    or omit to fall back to AGNES_API_KEY / CHAT_MODEL env."
                }
            },
            "required": ["prompts"]
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        let prompts: Vec<String> = args
            .get("prompts")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::Message("missing array argument: prompts".into()))?
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| ToolError::Message("prompts entries must be strings".into()))
                    .map(str::to_string)
            })
            .collect::<Result<_, _>>()?;

        if prompts.is_empty() {
            return Err(ToolError::Message("prompts must not be empty".into()));
        }

        let max_turns: usize = args
            .get("max_turns")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(MAX_TURNS_DEFAULT);

        let model = resolve_model(args.get("backend"))?;
        let tools = read_only_tools();
        let backend = HttpLlm;

        // Single prompt: inline run, no thread.
        if prompts.len() == 1 {
            let out = run_subagent(
                &backend,
                &model,
                &prompts[0],
                max_turns,
                &tools,
                signal,
                0,
                None,
            )
            .map_err(|e| ToolError::Message(format!("subagent: {e}")))?;
            return Ok(format!("## Task 1:\n{out}"));
        }

        // N>1: parallel via std::thread::scope; join at a wall-clock deadline.
        let wall = Duration::from_secs(SUBAGENT_TIMEOUT_SECS);
        let started = Instant::now();
        let deadline = started + wall;
        let (tx, rx) = mpsc::channel::<(u32, Result<String, SubagentError>)>();
        let cancel = AtomicBool::new(false);

        let results: Vec<(u32, Result<String, SubagentError>)> = thread::scope(|s| {
            let backend_ref = &backend;
            let tools_ref: &[Box<dyn Tool>] = &tools;
            for (idx, prompt) in prompts.iter().enumerate() {
                let tx = tx.clone();
                let model = model.clone();
                let prompt = prompt.as_str();
                s.spawn(move || {
                    let local = AtomicBool::new(false);
                    let outcome = run_subagent(
                        backend_ref,
                        &model,
                        prompt,
                        max_turns,
                        tools_ref,
                        &local,
                        idx as u32,
                        None,
                    );
                    let _ = tx.send((idx as u32, outcome));
                });
            }
            drop(tx);

            let mut acc: Vec<(u32, Result<String, SubagentError>)> =
                Vec::with_capacity(prompts.len());
            while acc.len() < prompts.len() {
                let now = Instant::now();
                if now >= deadline {
                    cancel.store(true, Ordering::Relaxed);
                    signal.store(true, Ordering::Relaxed);
                    break;
                }
                let remaining = deadline - now;
                match rx.recv_timeout(remaining) {
                    Ok(item) => acc.push(item),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        cancel.store(true, Ordering::Relaxed);
                        signal.store(true, Ordering::Relaxed);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            acc
        });
        let _ = cancel; // suppress unused warning if all prompts return early

        // Index-stable ordering: fill missing slots with a synthetic timeout
        // error so the caller can still render "Task N: <error>".
        let mut by_idx: Vec<Option<Result<String, SubagentError>>> =
            (0..prompts.len()).map(|_| None).collect();
        for (idx, res) in results {
            if let Some(slot) = by_idx.get_mut(idx as usize) {
                *slot = Some(res);
            }
        }
        let mut out = String::new();
        for (i, slot) in by_idx.into_iter().enumerate() {
            let body = match slot {
                Some(Ok(s)) => s,
                Some(Err(e)) => format!("<error: {e}>"),
                None => "<error: timeout — subagent still running when wall clock expired>".into(),
            };
            out.push_str(&format!("## Task {}:\n{}\n\n", i + 1, body));
        }
        Ok(out)
    }
}

/// Build the read-only tool set: read / grep / glob only. Bounded to keep
/// the subagent from doing anything but exploration.
fn read_only_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(tools::read::ReadFile),
        Box::new(tools::grep::Grep),
        Box::new(tools::glob::Glob),
    ]
}

/// Resolve the model spec: caller-provided > AGNES_API_KEY env fallback.
/// Matches `tui::model_from_config` field semantics so behaviour is the same
/// in TUI chat and the subagent tool.
fn resolve_model(spec: Option<&Value>) -> Result<Model, ToolError> {
    if let Some(obj) = spec {
        let api_key = obj
            .get("api_key")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Message("backend.api_key required".into()))?
            .to_string();
        let model = obj
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Message("backend.model required".into()))?
            .to_string();
        let base_url = obj
            .get("base_url")
            .and_then(Value::as_str)
            .map(|s| format!("{}/v1", s.trim_end_matches('/')));
        return Ok(Model {
            api_key,
            model,
            base_url,
            max_tokens: Some(4096),
        });
    }
    let api_key = std::env::var("AGNES_API_KEY")
        .map_err(|_| ToolError::Message("AGNES_API_KEY not set and no backend provided".into()))?;
    let model = std::env::var("CHAT_MODEL").unwrap_or_else(|_| "agnes-2.5-flash".into());
    let base_url = std::env::var("AGNES_BASE_URL")
        .ok()
        .map(|s| format!("{}/v1", s.trim_end_matches('/')));
    Ok(Model {
        api_key,
        model,
        base_url,
        max_tokens: Some(4096),
    })
}
