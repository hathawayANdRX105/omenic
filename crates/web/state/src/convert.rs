//! daemon 存储行 → UI DTO 的纯函数转换层（C5.2a）。
//!
//! 输入是 `session` crate 的 serde 形状（daemon 协议原样透传），输出是
//! [`crate::types`] 里页面/组件消费的 DTO。不碰网络、不碰信号，方便
//! 单测与在 `omenic-web-client` 的 daemon 封装里复用。

use session::{SessionMessage, SessionRole, SessionSummary};

use crate::types::{ChatMessage, Session, SessionStatus, format_relative_time};

/// `SessionSummary` → 侧栏/快速切换用的 `Session`。
///
/// 存储侧没有运行状态概念，统一映射 [`SessionStatus::Idle`]（发送中由
/// 页面在内存里自行置 Active）；存储侧也没有 model 字段，占位 `default`。
/// `last_active` 取 `updated_at_ms` 的相对时间。
pub fn summary_to_session(s: &SessionSummary) -> Session {
    let updated = s.updated_at_ms.max(0) as u64;
    Session {
        id: s.id.clone(),
        title: s.title.clone(),
        last_active: format_relative_time(updated),
        model: "default".into(),
        status: SessionStatus::Idle,
        last_active_epoch: updated,
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
        tool_calls: vec![],
        parts: vec![],
        timestamp: format_relative_time(ts),
        ts_epoch_ms: ts,
    }
}
