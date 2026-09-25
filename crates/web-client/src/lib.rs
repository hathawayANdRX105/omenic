//! omenic-web-client：LLM 运行时配置（读写 `.oi/config.toml` + 连接探测）
//! 与 daemon 客户端封装（C5.2a 读侧真数据）。

pub mod daemon;
pub mod llm;

/// 用户问题（plan-mode review 等）的线上类型：卡片渲染与回答都直接
/// 用 daemon 侧定义，web 不再抄一层 DTO。经 `daemon` 模块转出——crate
/// 根的同名模块会遮蔽外部 `daemon` crate（uniform path），不能直接
/// `pub use daemon::`。
pub use crate::daemon::{QuestionAnswer, QuestionItem, QuestionOption};

/// daemon 客户端错误：回答路径要区分终局错误（问题已消失）与传输错误。
pub use crate::daemon::ClientError;
