//! `TaskItem::from_todo` / `from_goal`：模型工具写入的 todo/goal 存储
//! （`todos.jsonl` / `goals.jsonl`）→ 任务看板卡片的映射单测。覆盖四件事：
//! - todo 状态词汇表（`TodoStatus` → TaskPanel 仅有的四个 chip）；
//! - goal 状态词汇表（`GoalStatus` → 同样的四个 chip）；
//! - todo 字段直通（note → description，kind/priority/acceptance 不编造）；
//! - goal 的关联 todo 计数（`todo_ids.len()` → description）。
//!
//! 对照 `task_item.rs` 里 `from_task` 的覆盖：任务存储有真实 priority 与
//! acceptance，todo/goal 两个模型都没有，所以这里断言「落成中性值 2 +
//! 空串」而不是「原样透传」——投影层不许给模型加字段。

use omenic_web_state::types::{
    RUN_TASK_PRIORITY, TaskItem, goal_status_to_panel, todo_status_to_panel,
};
use task::goal::{Goal, GoalStatus};
use task::todo::{Todo, TodoStatus};

/// 造一条 todo：status / note 由参数传入，其余字段固定且非空非默认。
fn todo(status: TodoStatus, note: Option<&str>) -> Todo {
    let mut t = Todo::new("看板探针".to_string()).expect("non-empty title");
    t.status = status;
    t.note = note.map(str::to_string);
    t
}

/// 造一条 goal：status / 关联 id 数由参数传入，其余字段固定。
fn goal(status: GoalStatus, linked: usize) -> Goal {
    let mut g = Goal::new("看板目标".to_string()).expect("non-empty title");
    g.status = status;
    g.todo_ids = (0..linked).map(|i| format!("todo-{i}")).collect();
    g
}

#[test]
fn from_todo_maps_status_vocabulary() {
    // TaskPanel 的过滤/chip 词汇表只有四个词；TodoStatus 四态必须一一落进
    // 这四个词。会红：Cancelled 被映成 "open"（取消项混进待办 chip，用户
    // 以为还要做），或新增 TodoStatus 变体后 match 漏掉。
    assert_eq!(todo_status_to_panel(&TodoStatus::Open), "open");
    assert_eq!(todo_status_to_panel(&TodoStatus::InProgress), "in_progress");
    assert_eq!(todo_status_to_panel(&TodoStatus::Done), "done");
    assert_eq!(todo_status_to_panel(&TodoStatus::Cancelled), "blocked");

    // 端到端：卡片 status 走同一个映射函数，不能有第二条翻译路径
    assert_eq!(
        TaskItem::from_todo(&todo(TodoStatus::Cancelled, None)).status,
        "blocked"
    );
}

#[test]
fn from_goal_maps_status_vocabulary() {
    // GoalStatus 只有三态，同样锁进四词表。会红：Active 被映成 "open"
    // （推进中的目标显示成待办），或 Abandoned 漏映射成未知串（chip 渲染
    // 掉出词汇表）。
    assert_eq!(goal_status_to_panel(&GoalStatus::Active), "in_progress");
    assert_eq!(goal_status_to_panel(&GoalStatus::Achieved), "done");
    assert_eq!(goal_status_to_panel(&GoalStatus::Abandoned), "blocked");

    // 端到端：卡片 status 走同一个映射函数，不能有第二条翻译路径
    assert_eq!(
        TaskItem::from_goal(&goal(GoalStatus::Abandoned, 0)).status,
        "blocked"
    );
}

#[test]
fn from_todo_passes_real_fields_through() {
    // note 是 todo 唯一的自由文本，卡片必须透传成 description。会红：note
    // 丢失（None 与 Some 渲染无差别）、kind 写死成 "task"（看板 kind 列
    // 把 todo 显示成编排任务）、priority 编造成非 2 值（中性 chip 被渲染
    // 成高优）。
    let card = TaskItem::from_todo(&todo(TodoStatus::InProgress, Some("备注正文")));
    assert_eq!(card.id, "看板探针");
    assert_eq!(card.title, "看板探针");
    assert_eq!(card.description, "备注正文");
    assert_eq!(card.kind, "todo");
    assert_eq!(card.priority, 2);
    assert_eq!(card.priority, RUN_TASK_PRIORITY);
    // Todo 模型没有验收标准概念：投影层留空而不是占位文案
    assert_eq!(card.acceptance, "");

    // note=None → 空串（TaskPanel 空串不渲染，但映射层不塞占位假信息）
    let blank = TaskItem::from_todo(&todo(TodoStatus::Open, None));
    assert_eq!(blank.description, "");
}

#[test]
fn from_goal_counts_linked_todos() {
    // description 只承载真实计数。会红：计数丢字段（三个关联显示成 0 或
    // 空）、或把 todo_ids 原样 JSON 塞进 description（面板出现看不懂的
    // 数组串）。
    let card = TaskItem::from_goal(&goal(GoalStatus::Active, 3));
    assert_eq!(card.kind, "goal");
    assert_eq!(card.priority, RUN_TASK_PRIORITY);
    assert_eq!(card.description, "3 个关联 todo");

    // 空关联 → "0"（真实计数，不是空串占位）
    assert_eq!(
        TaskItem::from_goal(&goal(GoalStatus::Active, 0)).description,
        "0 个关联 todo"
    );
}
