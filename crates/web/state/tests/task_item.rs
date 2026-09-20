//! `TaskItem::from_task`：CLI 任务存储（`tasks.jsonl`）→ 任务看板卡片的
//! 映射单测。覆盖三件事：
//! - 状态词汇表（`TaskStatus` → TaskPanel 仅有的四个 chip）；
//! - 字段直通（priority / description / acceptance 不被篡改或丢空）；
//! - `kind` 的 snake_case 渲染（与 `TaskKind` 的 serde 序列化一致）。
//!
//! 对照 `statusline_timing.rs` 里 `from_run` 的覆盖：run 记录没有真实优先级
//! 与验收标准（那里用中性值 + 留空），任务存储两者都有，所以这里断言
//! 「原样透传」而不是「落成默认值」。

use omenic_web_state::types::{TaskItem, task_status_to_panel};
use task::{Task, TaskKind, TaskStatus};

/// 造一条全字段填真的任务：构造默认值会让「字段直传」测试恒真，失去意义。
/// status / kind 由参数传入，其余字段固定且非空非默认。
fn task(status: TaskStatus, kind: TaskKind) -> Task {
    Task {
        id: "t-1".to_string(),
        title: "看板探针".to_string(),
        kind,
        status,
        attempts: 0,
        priority: 0,
        parent: None,
        deps: vec![],
        description: "描述正文".to_string(),
        acceptance: "验收标准".to_string(),
        created_at: "2026-09-20T00:00:00Z".to_string(),
        updated_at: "2026-09-20T00:00:00Z".to_string(),
    }
}

#[test]
fn from_task_maps_status_vocabulary() {
    // TaskPanel 的过滤/chip 词汇表只有四个词；TaskStatus 四态必须一一落进
    // 这四个词。会红：Failed 被映成 "open"（失败任务混进待办 chip，还丢掉
    // danger 色的视觉告警），或新增 TaskStatus 变体后 match 漏掉。
    assert_eq!(task_status_to_panel(&TaskStatus::Open), "open");
    assert_eq!(task_status_to_panel(&TaskStatus::InProgress), "in_progress");
    assert_eq!(task_status_to_panel(&TaskStatus::Done), "done");
    assert_eq!(task_status_to_panel(&TaskStatus::Failed), "blocked");

    // 端到端：卡片 status 走同一个映射函数，不能有第二条翻译路径
    assert_eq!(
        TaskItem::from_task(&task(TaskStatus::Failed, TaskKind::Bug)).status,
        "blocked"
    );
}

#[test]
fn from_task_passes_real_fields_through() {
    // 任务存储本身有这些列，卡片必须原样透传。会红：priority 被硬编码成
    // RUN_TASK_PRIORITY 的 2（P0 任务被降级成弱化 chip）、description /
    // acceptance 被丢成空串（面板里看不到任务说明与验收标准）。
    let card = TaskItem::from_task(&task(TaskStatus::InProgress, TaskKind::Feature));
    assert_eq!(card.id, "t-1");
    assert_eq!(card.title, "看板探针");
    assert_eq!(card.priority, 0);
    assert_eq!(card.description, "描述正文");
    assert_eq!(card.acceptance, "验收标准");

    // 空描述 / 空验收也要原样透传：TaskPanel 空串不渲染，但映射层不能因此
    // 替换成占位文案（那样面板会显示假信息）。
    let mut blank = task(TaskStatus::Open, TaskKind::Chore);
    blank.description = String::new();
    blank.acceptance = String::new();
    let blank_card = TaskItem::from_task(&blank);
    assert_eq!(blank_card.description, "");
    assert_eq!(blank_card.acceptance, "");
}

#[test]
fn from_task_kind_snake_case() {
    // TaskKind 的 serde 是 snake_case，卡片必须映同样的串。会红：用 Debug
    // 格式渲染成 "Feature" 之类的大写串（与序列化形式不一致，且不符合
    // TaskPanel 已有的 kind 文案约定）。
    assert_eq!(
        TaskItem::from_task(&task(TaskStatus::Open, TaskKind::Feature)).kind,
        "feature"
    );
    assert_eq!(
        TaskItem::from_task(&task(TaskStatus::Open, TaskKind::Bug)).kind,
        "bug"
    );
    assert_eq!(
        TaskItem::from_task(&task(TaskStatus::Open, TaskKind::Milestone)).kind,
        "milestone"
    );
    assert_eq!(
        TaskItem::from_task(&task(TaskStatus::Open, TaskKind::Unknown)).kind,
        "unknown"
    );
}
