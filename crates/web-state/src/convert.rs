//! daemon 存储行 → UI DTO 的纯函数转换层（C5.2a）。
//!
//! 输入是 `session` crate 的 serde 形状（daemon 协议原样透传），输出是
//! [`crate::types`] 里页面/组件消费的 DTO。不碰网络、不碰信号，方便
//! 单测与在 `omenic-web-client` 的 daemon 封装里复用。

use std::collections::HashMap;

use serde_json::Value;
use session::{RunRecord, SessionMessage, SessionRole, SessionSummary};

use crate::types::{ChatMessage, PendingAttachment, Session, SessionStatus, format_relative_time};
use crate::ui_state::AgentEvent;

/// `SessionSummary` → 侧栏/快速切换用的 `Session`。
///
/// 存储侧没有运行状态概念，统一映射 [`SessionStatus::Idle`]；列表侧的
/// 运行态（含半开 run 的 [`SessionStatus::Aborted`]）由调用方拿
/// [`infer_session_status`] 的结果在渲染时覆盖（见 page-workspace 的
/// `run_status_cache`）。存储侧也没有 model 字段，占位 `default`。
/// `last_active` 取 `updated_at_ms` 的相对时间。`parent_id` 原样透传
/// （`None` 即根会话），谱系分组（5.3/5.5）由侧栏按它自己组装树。
pub fn summary_to_session(s: &SessionSummary) -> Session {
    let updated = s.updated_at_ms.max(0) as u64;
    Session {
        id: s.id.clone(),
        title: s.title.clone(),
        last_active: format_relative_time(updated),
        model: "default".into(),
        status: SessionStatus::Idle,
        last_active_epoch: updated,
        parent_id: s.parent_id.clone(),
    }
}

/// 一个会话的 run 记录 → 列表侧的运行状态（WP-C：半开 run 显示 aborted）。
///
/// daemon 的 `SessionSummary` 没有状态字段，`session.*` / `run.list` 的
/// 命令语义又已冻结，会话列表的运行态改由 `run.list` 的 run 记录组装
/// （ROADMAP-active 缺口表「crash-repair 未接线」自述的兜底方案）。判定对齐 dsh
/// `interruptedTurnClosers` 的重载语义：有 `TurnStart` 无 `TurnEnd`
/// closer 的 run，在重载视图里即崩溃孤儿。
///
/// 三态判定（输入须是**该会话**的 run 记录，由调用方按 `session_id`
/// 过滤，见 `WebDaemon::runs_for_session`）：
///
/// 1. 存在未关闭的 run（`finished_at_ms` 为 `None`），且它不是当前页面
///    在飞的那个 run → [`SessionStatus::Aborted`]（半开 / 崩溃孤儿）；
/// 2. 未关闭的 run 恰好只有 `live_run_id` 指向的在飞 run →
///    [`SessionStatus::Active`]；
/// 3. 无 run 或全部正常关闭 → [`SessionStatus::Idle`]。
///
/// 纯函数、不碰网络：`live_run_id` 为 `None`（页面刚加载 / 刷新后无在飞
/// 上下文）时，未关闭的 run 一律视为孤儿 → `Aborted`，这正是「手动中断
/// run 后刷新页面，列表标 aborted 而非消失」的入口。
pub fn infer_session_status(runs: &[RunRecord], live_run_id: Option<&str>) -> SessionStatus {
    let half_open: Vec<&RunRecord> = runs.iter().filter(|r| r.finished_at_ms.is_none()).collect();
    if half_open.is_empty() {
        return SessionStatus::Idle;
    }
    // 唯一未关闭者恰为当前在飞 run → Active；除此之外（未关闭者不是在飞
    // run、或存在多个孤儿）都视作有崩溃孤儿 → Aborted
    let only_live_inflight =
        half_open.len() == 1 && Some(half_open[0].run_id.as_str()) == live_run_id;
    if only_live_inflight {
        SessionStatus::Active
    } else {
        SessionStatus::Aborted
    }
}

/// `SessionMessage` → 聊天流用的 `ChatMessage`。
///
/// UI 只有 user/assistant 两种渲染形态：`User` → "user"，其余
/// （assistant/system/tool）→ "assistant"。tool_calls/parts 在存储侧
/// 暂无对应列，留空（渲染回退 content）。`id` 用 `session_id-seq`
/// 保证同会话内唯一。
pub fn message_to_chat(m: &SessionMessage) -> ChatMessage {
    let role = match m.role {
        SessionRole::User => "user",
        SessionRole::Assistant | SessionRole::System | SessionRole::Tool => "assistant",
    };
    let ts = m.created_at_ms.max(0) as u64;
    ChatMessage {
        id: format!("{}-{}", m.session_id, m.seq),
        role: role.into(),
        content: m.text.clone(),
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![],
        attachments: m
            .attachments
            .iter()
            .map(|a| PendingAttachment {
                name: a.name.clone(),
                media_type: a.media_type.clone(),
                data: a.data.clone(),
            })
            .collect(),
        timestamp: format_relative_time(ts),
        ts_epoch_ms: ts,
    }
}

// ---------------- worker wire 事件翻译（C5.2b）----------------

/// worker 事件流 → [`AgentEvent`]。omp 工具事件无调用 id，start/end 按
/// 「同名 LIFO 配对」合成 id（`name-seq`），保证 tool_result 能按 id 命中。
///
/// 状态归调用方所有：订阅读线程为每条连接持有一个实例；断线重连时丢弃
/// 重建（新连接的事件流从零开始，在飞配对状态随之作废）。
///
/// 帧形状：omp 原始 wire 用 `"type"` 标签 + `toolName` 字段；daemon R2 3.3
/// 转发层（`dispatch.rs` 的 forwarder）实际广播 `WorkerEvent` 的 serde
/// 形状——`"event"` 标签 + `name` 字段。两种形状按同一映射翻译，字段
/// 读取顺序先 wire 后转发层。
#[derive(Debug, Default)]
pub struct WireTranslator {
    /// 全局自增序号：合成 id 尾数，跨工具单调不重复。
    seq: u64,
    /// 在飞工具：name → 未配对 start 的 seq 栈（LIFO 配对）。
    inflight: HashMap<String, Vec<u64>>,
}

impl WireTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 翻译一帧 worker 事件。返回 `None` 表示与本页无关：`error` 帧
    /// （worker 传输故障，重连由订阅层负责）与未知/缺字段类型直接丢弃。
    pub fn translate(&mut self, event: &Value) -> Option<AgentEvent> {
        let ty = event
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| event.get("event").and_then(Value::as_str))
            .unwrap_or("");
        match ty {
            "agent_start" => Some(AgentEvent::TurnStart),
            "message_start" | "message_update" | "message" => {
                let text = event
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| event.pointer("/message/content").and_then(Value::as_str))
                    .unwrap_or("");
                // 空增量不产生事件，避免 apply 侧凭空补出空 assistant 气泡
                (!text.is_empty()).then(|| AgentEvent::AssistantText {
                    delta: text.to_string(),
                })
            }
            "reasoning" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                (!delta.is_empty()).then(|| AgentEvent::Reasoning { delta })
            }
            "tool_execution" | "tool_execution_start" => {
                let name = tool_name(event)?;
                let args = event.get("input").cloned().unwrap_or(Value::Null);
                // 假设同名 start/end 严格配对或嵌套（LIFO 正确的前提）；
                // 同层交错完成会误配对——wire 无调用 id 的协议限制，
                // 与 per-run 路由同批解决
                self.seq += 1;
                self.inflight
                    .entry(name.clone())
                    .or_default()
                    .push(self.seq);
                Some(AgentEvent::ToolCall {
                    id: format!("{}-{}", name, self.seq),
                    name,
                    args,
                })
            }
            "tool_execution_end" => {
                let name = tool_name(event)?;
                // 同名 LIFO：后 start 的先 end；没有在飞记录（未配对 end）→ 跳过
                let seq = {
                    let stack = self.inflight.get_mut(&name)?;
                    let seq = stack.pop()?;
                    if stack.is_empty() {
                        self.inflight.remove(&name);
                    }
                    seq
                };
                // result：Some → pretty JSON；None/缺失 → 空串
                let result = match event.get("result") {
                    Some(v) if !v.is_null() => serde_json::to_string_pretty(v).unwrap_or_default(),
                    _ => String::new(),
                };
                Some(AgentEvent::ToolResult {
                    id: format!("{}-{}", name, seq),
                    name,
                    result,
                })
            }
            "agent_end" => Some(AgentEvent::TurnEnd {
                stop_reason: "end_turn".into(),
            }),
            // 事件流故障（rpc 层合成帧）：转成 error 停止原因的 TurnEnd，
            // 复用消费端全部收尾（占位文案/结算/复位），否则 is_streaming
            // 会永久卡 true 锁死输入；UDS 仍健在所以不会触发重连
            "error" => Some(AgentEvent::TurnEnd {
                stop_reason: "error".into(),
            }),
            // 未知类型：不转译
            _ => None,
        }
    }
}

/// 工具名：omp wire 是 `toolName`，转发层归一成 `name`；两者都没有视为
/// 坏帧，丢弃。
fn tool_name(event: &Value) -> Option<String> {
    event
        .get("toolName")
        .or_else(|| event.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
}
