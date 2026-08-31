//! Subagent limits and system prompt.
//!
//! Constants are kept here so the runner, the task tool, and the tests
//! share one source of truth (no magic numbers drifting across files).

/// Hard wall-clock cap for a single subagent run.
pub const SUBAGENT_TIMEOUT_SECS: u64 = 300;

/// Truncation cap on a subagent's collected assistant text.
pub const MAX_SUBAGENT_RESPONSE_BYTES: usize = 128 * 1024;

/// Default cap on agent loop turns when the caller doesn't specify one.
pub const MAX_TURNS_DEFAULT: usize = 10;

/// Spill location for truncated output, matching tools::SPILL_DIR style.
pub const SPILL_DIR: &str = "/tmp";

/// System prompt injected into the subagent's context. Read-only by design.
pub const SUBAGENT_SYSTEM_PROMPT: &str = "你是 subagent — 一个只读探索助手。只能调用 read/grep/glob 三个工具。\
 不写文件、不执行命令、不修改状态。最终只输出对主 agent 有用的精炼结论。";
