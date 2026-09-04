//! Mock data for web UI pages.
//!
//! These types mirror the shapes the real backend will eventually serve.
//! All data here is synthetic; replaced by real `#[server]` endpoints later.

use serde::{Deserialize, Serialize};

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub last_active: String,
    pub model: String,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Idle,
    Archived,
}

// ── Status line ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusLine {
    pub model: String,
    pub thinking: String,
    pub cwd: String,
    pub git_branch: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub context_pct: f64,
    pub context_max: u64,
}

// ── Tasks (mirrors crates/task) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub kind: String,
    pub priority: u8,
}

// ── Stats ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KpiCard {
    pub label: String,
    pub value: String,
    pub delta: String,
    pub delta_positive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubMetric {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTokenBar {
    pub agent: String,
    pub tokens: String,
    pub pct: f64,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThroughputPoint {
    pub time: String,
    pub requests: f64,
    pub tokens: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedItem {
    pub model: String,
    pub provider: String,
    pub time_ago: String,
    pub duration: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsData {
    pub kpis: Vec<KpiCard>,
    pub sub_metrics: Vec<SubMetric>,
    pub agent_bars: Vec<AgentTokenBar>,
    pub throughput: Vec<ThroughputPoint>,
    pub feed: Vec<FeedItem>,
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub name: String,
    pub base_url: String,
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub display_name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigData {
    pub providers: Vec<ProviderEntry>,
    pub default_model: String,
    pub data_dir: String,
}

// ── Mock data generators ────────────────────────────────────────────────────

pub fn mock_sessions() -> Vec<Session> {
    vec![
        Session {
            id: "s1".into(),
            title: "重构 orbit compaction".into(),
            last_active: "2 分钟前".into(),
            model: "qwen3-32b".into(),
            status: SessionStatus::Active,
        },
        Session {
            id: "s2".into(),
            title: "MCP 工具接线".into(),
            last_active: "15 分钟前".into(),
            model: "qwen3-32b".into(),
            status: SessionStatus::Idle,
        },
        Session {
            id: "s3".into(),
            title: "memory store 修复".into(),
            last_active: "1 小时前".into(),
            model: "agnes-2.5-flash".into(),
            status: SessionStatus::Idle,
        },
        Session {
            id: "s4".into(),
            title: "TUI 流式渲染".into(),
            last_active: "3 小时前".into(),
            model: "qwen3-32b".into(),
            status: SessionStatus::Archived,
        },
        Session {
            id: "s5".into(),
            title: "subagent 并行探索".into(),
            last_active: "昨天".into(),
            model: "agnes-2.5-flash".into(),
            status: SessionStatus::Archived,
        },
    ]
}

pub fn mock_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage { id: "m1".into(), role: "user".into(), content: "帮我重构 orbit 的 compaction 策略，把固定 50 条改成字符预算模式".into(), timestamp: "14:30".into() },
        ChatMessage { id: "m2".into(), role: "assistant".into(), content: "好的，我来看一下当前的 compaction 实现。\n\n已运行 `git diff main..HEAD -- crates/orbit/`\n\n改动集中在 `crates/orbit/src/lib.rs`，涉及 `compact()` 和 `run_agent_streaming()`。\n\n关键变化：\n- COMPACT_CHAR_BUDGET = 120_000\n- 保留最近 KEEP_RECENT_MIN = 4 条消息\n- 跳过 orphan tool_results\n\n已写入 `crates/orbit/src/lib.rs` +45 -20".into(), timestamp: "14:31".into() },
        ChatMessage { id: "m3".into(), role: "user".into(), content: "kept_chars 的守卫逻辑有 bug，system_prompt 没算进去".into(), timestamp: "14:35".into() },
        ChatMessage { id: "m4".into(), role: "assistant".into(), content: "确认了。`compact()` 里 `kept_chars` 只统计了 `messages` 的字符，漏掉了 `system_prompt` 的长度。\n\n修复：在 `compact()` 开头把 `system_prompt.len()` 加入 `kept_chars` 初始值。\n\n已写入 `crates/orbit/src/lib.rs` +2 -1\n\n测试通过。".into(), timestamp: "14:36".into() },
    ]
}

pub fn mock_statusline() -> StatusLine {
    StatusLine {
        model: "qwen3-32b".into(),
        thinking: "off".into(),
        cwd: "~/projects/omenic".into(),
        git_branch: "feat/web-dioxus".into(),
        tokens_in: 45_230,
        tokens_out: 12_847,
        cost_usd: 0.023,
        context_pct: 34.2,
        context_max: 128_000,
    }
}

pub fn mock_tasks() -> Vec<TaskItem> {
    vec![
        TaskItem {
            id: "t1".into(),
            title: "EC-1 LLM 读超时".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 0,
        },
        TaskItem {
            id: "t2".into(),
            title: "EC-2 run_bash 危险命令闸门".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 0,
        },
        TaskItem {
            id: "t3".into(),
            title: "EC-3 Guarded name() 类型统一".into(),
            status: "in_progress".into(),
            kind: "bug".into(),
            priority: 1,
        },
        TaskItem {
            id: "t4".into(),
            title: "EC-4 runner 采纳 rpc 超时/重连".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 1,
        },
        TaskItem {
            id: "t5".into(),
            title: "EC-5 memory 接线".into(),
            status: "blocked".into(),
            kind: "feature".into(),
            priority: 2,
        },
        TaskItem {
            id: "t6".into(),
            title: "EC-6 LLM 重试分类".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 1,
        },
        TaskItem {
            id: "t7".into(),
            title: "Dioxus Web UI".into(),
            status: "in_progress".into(),
            kind: "feature".into(),
            priority: 0,
        },
    ]
}

pub fn mock_stats() -> StatsData {
    StatsData {
        kpis: vec![
            KpiCard {
                label: "费用估算".into(),
                value: "$361.37".into(),
                delta: "+12.3%".into(),
                delta_positive: false,
            },
            KpiCard {
                label: "请求数".into(),
                value: "8,541".into(),
                delta: "+5.2%".into(),
                delta_positive: true,
            },
            KpiCard {
                label: "缓存节省".into(),
                value: "$128.40".into(),
                delta: "0.0%".into(),
                delta_positive: true,
            },
            KpiCard {
                label: "缓存率".into(),
                value: "89.6%".into(),
                delta: "+2.1%".into(),
                delta_positive: true,
            },
            KpiCard {
                label: "错误率".into(),
                value: "4.0%".into(),
                delta: "-0.8%".into(),
                delta_positive: true,
            },
        ],
        sub_metrics: vec![
            SubMetric {
                label: "UNCACHED INPUT".into(),
                value: "121M".into(),
            },
            SubMetric {
                label: "CACHE READ".into(),
                value: "1B".into(),
            },
            SubMetric {
                label: "OUTPUT TOKENS".into(),
                value: "2.6M".into(),
            },
            SubMetric {
                label: "CONVERSATION TOTAL".into(),
                value: "1.2B".into(),
            },
            SubMetric {
                label: "TOKENS/S".into(),
                value: "15.5".into(),
            },
            SubMetric {
                label: "AVG LATENCY".into(),
                value: "23.6s".into(),
            },
            SubMetric {
                label: "AVG TTFT".into(),
                value: "19.3s".into(),
            },
        ],
        agent_bars: vec![
            AgentTokenBar {
                agent: "Main agent".into(),
                tokens: "970M".into(),
                pct: 83.2,
                color: "#e91e63".into(),
            },
            AgentTokenBar {
                agent: "Subagents".into(),
                tokens: "196M".into(),
                pct: 16.8,
                color: "#2196f3".into(),
            },
        ],
        throughput: vec![
            ThroughputPoint {
                time: "00:00".into(),
                requests: 12.0,
                tokens: 45.0,
            },
            ThroughputPoint {
                time: "02:00".into(),
                requests: 8.0,
                tokens: 32.0,
            },
            ThroughputPoint {
                time: "04:00".into(),
                requests: 5.0,
                tokens: 18.0,
            },
            ThroughputPoint {
                time: "06:00".into(),
                requests: 15.0,
                tokens: 62.0,
            },
            ThroughputPoint {
                time: "08:00".into(),
                requests: 42.0,
                tokens: 180.0,
            },
            ThroughputPoint {
                time: "10:00".into(),
                requests: 68.0,
                tokens: 290.0,
            },
            ThroughputPoint {
                time: "12:00".into(),
                requests: 55.0,
                tokens: 230.0,
            },
            ThroughputPoint {
                time: "14:00".into(),
                requests: 72.0,
                tokens: 310.0,
            },
            ThroughputPoint {
                time: "16:00".into(),
                requests: 48.0,
                tokens: 195.0,
            },
            ThroughputPoint {
                time: "18:00".into(),
                requests: 35.0,
                tokens: 140.0,
            },
            ThroughputPoint {
                time: "20:00".into(),
                requests: 28.0,
                tokens: 95.0,
            },
            ThroughputPoint {
                time: "22:00".into(),
                requests: 18.0,
                tokens: 58.0,
            },
        ],
        feed: vec![
            FeedItem {
                model: "qwen3-32b".into(),
                provider: "AGNES".into(),
                time_ago: "2 分钟前".into(),
                duration: "12.3s".into(),
                cost: "$0.004".into(),
            },
            FeedItem {
                model: "agnes-2.5-flash".into(),
                provider: "AGNES".into(),
                time_ago: "5 分钟前".into(),
                duration: "3.1s".into(),
                cost: "$0.001".into(),
            },
            FeedItem {
                model: "qwen3-32b".into(),
                provider: "AGNES".into(),
                time_ago: "8 分钟前".into(),
                duration: "18.7s".into(),
                cost: "$0.007".into(),
            },
            FeedItem {
                model: "kimi-k3".into(),
                provider: "new-api".into(),
                time_ago: "12 分钟前".into(),
                duration: "8.2s".into(),
                cost: "$0.003".into(),
            },
            FeedItem {
                model: "qwen3-32b".into(),
                provider: "AGNES".into(),
                time_ago: "15 分钟前".into(),
                duration: "22.1s".into(),
                cost: "$0.009".into(),
            },
            FeedItem {
                model: "agnes-2.5-flash".into(),
                provider: "AGNES".into(),
                time_ago: "20 分钟前".into(),
                duration: "4.5s".into(),
                cost: "$0.001".into(),
            },
        ],
    }
}

pub fn mock_config() -> ConfigData {
    ConfigData {
        providers: vec![
            ProviderEntry {
                name: "AGNES".into(),
                base_url: "https://agnes.internal/v1".into(),
                models: vec![
                    ModelEntry {
                        name: "agnes-2.5-flash".into(),
                        display_name: "Agnes 2.5 Flash".into(),
                        active: true,
                    },
                    ModelEntry {
                        name: "agnes-2.5-pro".into(),
                        display_name: "Agnes 2.5 Pro".into(),
                        active: false,
                    },
                ],
            },
            ProviderEntry {
                name: "new-api".into(),
                base_url: "http://localhost:3000/v1".into(),
                models: vec![
                    ModelEntry {
                        name: "kimi-k3".into(),
                        display_name: "Kimi K3".into(),
                        active: true,
                    },
                    ModelEntry {
                        name: "qwen3-32b".into(),
                        display_name: "Qwen3 32B".into(),
                        active: true,
                    },
                    ModelEntry {
                        name: "deepseek-v3".into(),
                        display_name: "DeepSeek V3".into(),
                        active: false,
                    },
                ],
            },
        ],
        default_model: "qwen3-32b".into(),
        data_dir: "~/.oi".into(),
    }
}
