//! Incremental SSE chunk parser. Accumulates partial tool_calls by index;
//! pure logic so chunk-boundary behavior is unit-testable offline.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::{StopReason, ToolCallSpec};

#[derive(Debug, Default)]
struct ToolCallBuf {
    id: String,
    name: String,
    args_buf: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    tool_calls: BTreeMap<u64, ToolCallBuf>,
}

/// What one SSE `data:` line contributed.
#[derive(Debug, Default, PartialEq)]
pub struct SseLineOut {
    pub text_delta: Option<String>,
    pub stop_reason: Option<StopReason>,
    pub reasoning_delta: Option<String>,
}

impl SseParser {
    pub fn new() -> Self {
        SseParser::default()
    }

    /// Parse one `data:` payload (without the `data: ` prefix).
    /// Malformed JSON lines are ignored, matching llm.ts.
    pub fn handle_data(&mut self, data: &str) -> SseLineOut {
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return SseLineOut::default();
        };
        let Some(choice) = chunk["choices"].get(0) else {
            return SseLineOut::default();
        };

        let mut out = SseLineOut::default();

        if let Some(text) = choice["delta"]["content"].as_str()
            && !text.is_empty()
        {
            out.text_delta = Some(text.to_string());
        }

        // DeepSeek reasoning_content deltas
        if let Some(reasoning) = choice["delta"]["reasoning_content"].as_str()
            && !reasoning.is_empty()
        {
            out.reasoning_delta = Some(reasoning.to_string());
        }

        // tool_call deltas: accumulate name + partial-JSON arguments by index.
        if let Some(tcs) = choice["delta"]["tool_calls"].as_array() {
            for tc in tcs {
                let idx = tc["index"].as_u64().unwrap_or(0);
                let entry = self.tool_calls.entry(idx).or_insert_with(|| ToolCallBuf {
                    id: tc["id"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{idx}")),
                    ..ToolCallBuf::default()
                });
                // 续帧守卫：newapi 类网关把 tool_call 续帧的 id/name 重复为
                // 空串（只带 arguments 分片）。无条件覆盖会把首帧的真实值冲
                // 成 ""——模型所有工具调用以 `tool "" not found` 告终。空串
                // 视为「本帧无此字段」，仅非空才覆盖。
                if let Some(id) = tc["id"].as_str()
                    && !id.is_empty()
                {
                    entry.id = id.to_string();
                }
                if let Some(name) = tc["function"]["name"].as_str()
                    && !name.is_empty()
                {
                    entry.name = name.to_string();
                }
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    entry.args_buf.push_str(args);
                }
            }
        }

        // finish_reason map: tool_calls→tool_use, length→max_tokens, stop→implicit end_turn.
        out.stop_reason = match choice["finish_reason"].as_str() {
            Some("tool_calls") => Some(StopReason::ToolUse),
            Some("length") => Some(StopReason::MaxTokens),
            _ => None,
        };

        out
    }

    /// Flush accumulated tool_calls in index order; args parsed or `{}` on bad JSON.
    pub fn flush(&self) -> Vec<ToolCallSpec> {
        self.tool_calls
            .values()
            .map(|tc| {
                let args = if tc.args_buf.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&tc.args_buf).unwrap_or_else(|_| json!({}))
                };
                ToolCallSpec {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    args,
                }
            })
            .collect()
    }
}
