//! The `task` tool: parallel read-only subagent exploration.
//!
//! One prompt → run inline. Multiple prompts → bounded fan-out: at most
//! `MAX_CONCURRENT_SUBAGENTS` run at once, the rest queue on the semaphore.
//! Wall-clock is capped at `SUBAGENT_TIMEOUT_SECS`; on timeout (or parent
//! abort) every live subagent's signal is flipped so loops stop and partial
//! outputs are collected during a short grace window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use adaptor::Model;
use orbit::HttpLlm;
use serde_json::{Value, json};
use tools::{Tool, ToolError};

use crate::config::{MAX_CONCURRENT_SUBAGENTS, MAX_TURNS_DEFAULT, SUBAGENT_TIMEOUT_SECS};
use crate::parallel::Semaphore;
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
         多 prompt 有界并行（同时最多 4 个，其余排队），受 5min wall-clock 限制。"
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
        let prompts = preflight(args)?;
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

        // N>1: bounded fan-out via scoped threads, joined at a wall-clock
        // deadline. Each child owns its signal; the collector flips them
        // all when the deadline hits or the parent aborts, so live loops
        // stop instead of leaking past the tool call.
        let wall = Duration::from_secs(SUBAGENT_TIMEOUT_SECS);
        let deadline = Instant::now() + wall;
        let (tx, rx) = mpsc::channel::<(u32, Result<String, SubagentError>)>();
        let sem = Semaphore::new(MAX_CONCURRENT_SUBAGENTS);
        let locals: Vec<Arc<AtomicBool>> = (0..prompts.len())
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect();

        let results: Vec<(u32, Result<String, SubagentError>)> = thread::scope(|s| {
            let backend = &backend;
            for (idx, prompt) in prompts.iter().enumerate() {
                let tx = tx.clone();
                let model = model.clone();
                let prompt = prompt.as_str();
                let local = Arc::clone(&locals[idx]);
                let sem = &sem;
                let tools = &tools;
                s.spawn(move || {
                    // Admission is bounded; a queued child bails out as soon
                    // as its signal flips (deadline or parent abort).
                    let outcome = match sem.acquire(&local) {
                        Ok(_guard) => run_subagent(
                            backend, &model, prompt, max_turns, tools, &local, idx as u32, None,
                        ),
                        Err(_) => Err(SubagentError::Aborted {
                            partial: String::new(),
                        }),
                    };
                    let _ = tx.send((idx as u32, outcome));
                });
            }
            drop(tx);

            // Wait for a result from every slot. Poll the parent signal on
            // short recv timeouts; on deadline/abort flip all child signals
            // and drain their partial results during a 2s grace window.
            let mut acc: Vec<(u32, Result<String, SubagentError>)> =
                Vec::with_capacity(prompts.len());
            let mut grace_end: Option<Instant> = None;
            while acc.len() < prompts.len() {
                let now = Instant::now();
                if grace_end.is_none() && (now >= deadline || signal.load(Ordering::Relaxed)) {
                    grace_end = Some(now + Duration::from_secs(2));
                    for l in &locals {
                        l.store(true, Ordering::Relaxed);
                    }
                }
                let end = grace_end.unwrap_or(deadline);
                if now >= end {
                    break;
                }
                match rx.recv_timeout((end - now).min(Duration::from_millis(50))) {
                    Ok(item) => acc.push(item),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            acc
        });

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

/// Validate the request before any subagent gets a thread or a permit
/// (omp preflight pattern: everything checked up front, no partial spawns).
fn preflight(args: &Value) -> Result<Vec<String>, ToolError> {
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
    let max_turns = args
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(MAX_TURNS_DEFAULT);
    if max_turns == 0 {
        return Err(ToolError::Message("max_turns must be at least 1".into()));
    }
    Ok(prompts)
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
