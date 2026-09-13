//! omenic-web-state：web UI 的状态词汇表。
//!
//! - [`convert`]：daemon `session` 存储行 → UI DTO 的纯函数转换（C5.2a）
//! - [`types`]：页面/组件消费的 DTO（Session/ChatMessage/TaskItem/Stats…）
//! - [`ui_state`]：C5.1 `AgentEvent` → UI 状态转译层（纯函数）
//! - [`memory_link`]：jcode 记忆注入/抽取接缝（独有功能，C1 域）

pub mod convert;
pub mod memory_link;
pub mod types;
pub mod ui_state;
