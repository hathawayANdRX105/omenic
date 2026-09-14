//! convert 转换层的单元测试：字段映射 + 相对时间格式。

use omenic_web_state::convert::{message_to_chat, summary_to_session};
use omenic_web_state::types::SessionStatus;
use session::{SessionMessage, SessionRole, SessionSummary};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[test]
fn summary_maps_fields_and_relative_time() {
    let now = now_ms();
    let summary = SessionSummary {
        id: "s-1".into(),
        title: "标题甲".into(),
        created_at_ms: (now - 60_000) as i64,
        updated_at_ms: (now - 120_000) as i64,
        message_count: 3,
    };
    let s = summary_to_session(&summary);
    // 字段映射：id/title 原样；存储无状态概念 → Idle；model 占位 default
    assert_eq!(s.id, "s-1");
    assert_eq!(s.title, "标题甲");
    assert_eq!(s.status, SessionStatus::Idle);
    assert_eq!(s.model, "default");
    // 相对时间：updated_at 约 2 分钟前 → epoch 原样透传 + "N 分钟前"
    assert_eq!(s.last_active_epoch, now - 120_000);
    assert_eq!(s.last_active, "2 分钟前");
}

#[test]
fn summary_recent_activity_is_just_now() {
    let now = now_ms();
    let summary = SessionSummary {
        id: "s-2".into(),
        title: "标题乙".into(),
        created_at_ms: now as i64,
        updated_at_ms: now as i64,
        message_count: 0,
    };
    let s = summary_to_session(&summary);
    assert_eq!(s.last_active_epoch, now);
    assert_eq!(s.last_active, "刚刚");
}

#[test]
fn message_user_maps_role_and_timestamp() {
    let now = now_ms();
    let m = SessionMessage {
        session_id: "s-1".into(),
        seq: 1,
        role: SessionRole::User,
        text: "你好".into(),
        created_at_ms: now as i64,
    };
    let c = message_to_chat(&m);
    // 字段映射：role/text/ts；id = session_id-seq；tool_calls/parts 留空
    assert_eq!(c.role, "user");
    assert_eq!(c.content, "你好");
    assert_eq!(c.id, "s-1-1");
    assert_eq!(c.ts_epoch_ms, now);
    assert_eq!(c.timestamp, "刚刚");
    assert!(c.tool_calls.is_empty());
    assert!(c.parts.is_empty());
}

#[test]
fn message_non_user_maps_to_assistant() {
    let now = now_ms();
    let m = SessionMessage {
        session_id: "s-1".into(),
        seq: 2,
        role: SessionRole::Tool,
        text: "工具输出".into(),
        created_at_ms: (now - 3_600_000) as i64,
    };
    let c = message_to_chat(&m);
    // 非 User 角色（system/tool/assistant）统一映射为 assistant
    assert_eq!(c.role, "assistant");
    assert_eq!(c.content, "工具输出");
    assert_eq!(c.ts_epoch_ms, now - 3_600_000);
    assert_eq!(c.timestamp, "1 小时前");
}
