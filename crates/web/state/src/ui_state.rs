//! C5.1：`AgentEvent` DTO + 事件 → UI 状态转译层（纯函数，fixture 先行）。
//!
//! 转译器不关心事件来自哪里：G4 之前是 `omenic-web-mock` 的模拟流，
//! 之后换成 daemon `event.subscribe` 的实时流（C3.3），页面代码零改动。
//! serde 形状对齐 `orbit::AgentEvent`（3.1 定稿后以冻结契约为准）。

use crate::types::{ChatMessage, MessagePart, ToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 跨 crate 事件契约（snake_case tag）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TurnStart,
    AssistantText {
        delta: String,
    },
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
    ToolStart {
        id: String,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
    },
    TurnEnd {
        stop_reason: String,
    },
}

/// 单个会话的可见 UI 状态：消息列表（按发生顺序渲染）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiState {
    pub messages: Vec<ChatMessage>,
}

impl UiState {
    /// 追加一条消息（用户输入 / 预留的 assistant 占位）。
    pub fn push_message(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
    }

    /// 把事件应用到当前状态。约定：`AssistantText`/`ToolCall` 等
    /// assistant 侧事件作用于最后一条 assistant 消息（不存在则先补占位）。
    pub fn apply(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::TurnStart => {}
            AgentEvent::AssistantText { delta } => {
                let msg = self.last_assistant_or_placeholder();
                msg.content.push_str(delta);
                match msg.parts.last_mut() {
                    Some(MessagePart::Text(existing)) => existing.push_str(delta),
                    _ => msg.parts.push(MessagePart::Text(delta.clone())),
                }
            }
            AgentEvent::ToolCall { id, name, args } => {
                let tc = tool_call_from_rpc(id, name, args);
                let msg = self.last_assistant_or_placeholder();
                msg.tool_calls.push(tc.clone());
                msg.parts.push(MessagePart::Tool(tc));
            }
            // 工具真正开始执行；ToolCall 臂已在流解析时置 running，无需变更
            AgentEvent::ToolStart { .. } => {}
            AgentEvent::ToolResult { id, result, .. } => {
                let is_err = result.starts_with("error") || result.contains("[exit ");
                let status = if is_err { "error" } else { "success" };
                let summary = if is_err {
                    "执行失败"
                } else {
                    "执行完成"
                };
                let msg = self.last_assistant_or_placeholder();
                for t in msg.tool_calls.iter_mut().filter(|t| t.id == *id) {
                    t.status = status.to_string();
                    t.summary = summary.to_string();
                    t.detail = result.clone();
                }
                for p in msg.parts.iter_mut() {
                    if let MessagePart::Tool(t) = p
                        && t.id == *id
                    {
                        t.status = status.to_string();
                        t.summary = summary.to_string();
                        t.detail = result.clone();
                    }
                }
            }
            AgentEvent::TurnEnd { stop_reason } => {
                let msg = self.last_assistant_or_placeholder();
                if msg.content.is_empty() && msg.tool_calls.is_empty() {
                    let text = format!(
                        "Agent 执行结束（原因: {stop_reason}）。未能获取有效回复，请在「设置」页检查 API 凭证与端点地址。"
                    );
                    msg.content = text.clone();
                    msg.parts.push(MessagePart::Text(text));
                }
            }
        }
    }

    fn last_assistant_or_placeholder(&mut self) -> &mut ChatMessage {
        if !self.messages.last().is_some_and(|m| m.role == "assistant") {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            self.messages.push(ChatMessage {
                id: format!("asst-{now_ms}"),
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![],
                parts: vec![],
                timestamp: "刚刚".into(),
                ts_epoch_ms: now_ms,
            });
        }
        self.messages.last_mut().unwrap()
    }
}

/// 便捷入口：独立应用一个事件。
pub fn apply_event(state: &mut UiState, ev: &AgentEvent) {
    state.apply(ev);
}

/// RPC 工具调用 → 展示用 `ToolCall`（标题取 command/path，kind 归一化）。
fn tool_call_from_rpc(id: &str, name: &str, args: &Value) -> ToolCall {
    let kind = match name {
        "run_bash" => "bash",
        "edit" => "edit",
        "read_file" => "read",
        "write_file" => "write",
        "delete_file" => "delete",
        "grep" => "grep",
        "glob" => "glob",
        other => {
            return ToolCall {
                id: id.to_string(),
                title: other.to_string(),
                kind: "tool".to_string(),
                summary: "正在执行...".to_string(),
                detail: serde_json::to_string_pretty(args).unwrap_or_default(),
                status: "running".to_string(),
            };
        }
    };
    let title = match kind {
        "bash" => args.get("command").and_then(Value::as_str).unwrap_or(name),
        _ => args.get("path").and_then(Value::as_str).unwrap_or(name),
    };
    ToolCall {
        id: id.to_string(),
        title: title.to_string(),
        kind: kind.to_string(),
        summary: "正在执行...".to_string(),
        detail: serde_json::to_string_pretty(args).unwrap_or_default(),
        status: "running".to_string(),
    }
}
