//! omenic-web-mock：G4 接线前的假数据源。
//!
//! - [`store`]：会话/消息/任务/统计 fixture（形状即真实数据形状）
//! - [`stream`]：`AgentEvent` 模拟流——on_send 的假 transport，
//!   G4 时换成 daemon `event.subscribe`，页面零改动

pub mod store;
pub mod stream;
