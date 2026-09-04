//! Mock data for web UI pages.
//!
//! These types mirror the shapes the real backend will eventually serve.
//! All data here is synthetic; replaced by real `#[server]` endpoints later.

use serde::{Deserialize, Serialize};

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub title: String,
    pub kind: String, // "bash", "edit", "read", "status"
    pub summary: String,
    pub detail: String,
    pub status: String, // "success", "running", "error"
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
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
    pub description: String,
    pub acceptance: String,
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
    mock_messages_for_session("s1")
}

pub fn mock_messages_for_session(session_id: &str) -> Vec<ChatMessage> {
    match session_id {
        "s2" => vec![
            ChatMessage {
                id: "s2-m1".into(),
                role: "user".into(),
                content: "我们需要将外部 MCP 服务的工具注册进 worker 的 runner 中。".into(),
                tool_calls: vec![],
                timestamp: "12:10".into(),
            },
            ChatMessage {
                id: "s2-m2".into(),
                role: "assistant".into(),
                content: "已检查 MCP Client 与 runner 的接口契约，计划在 `register()` 调用阶段做动态注入。".into(),
                tool_calls: vec![
                    ToolCall {
                        id: "tc-s2-1".into(),
                        title: "已运行 grep -n 'builtin_tools' crates/task/src/runner.rs".into(),
                        kind: "bash".into(),
                        summary: "定位到 builtin_tools 注册点 runner.rs:203".into(),
                        detail: "crates/task/src/runner.rs:203:        let mut tools = tools::builtin_tools();\ncrates/task/src/runner.rs:204:        tools.extend(mcp_tools);".into(),
                        status: "success".into(),
                    },
                ],
                timestamp: "12:12".into(),
            },
        ],
        "s3" => vec![
            ChatMessage {
                id: "s3-m1".into(),
                role: "user".into(),
                content: "排查 memory store 在意外断电时可能发生的数据截断问题。".into(),
                tool_calls: vec![],
                timestamp: "10:05".into(),
            },
            ChatMessage {
                id: "s3-m2".into(),
                role: "assistant".into(),
                content: "定位到 `sync_all` 阶段缺少原子重命名机制。改用临时文件写入 + fsync + rename 保证事务完整。".into(),
                tool_calls: vec![
                    ToolCall {
                        id: "tc-s3-1".into(),
                        title: "已写入 crates/memory/src/store.rs +18 -4".into(),
                        kind: "edit".into(),
                        summary: "引入 tempfile + fs::rename 原子落盘".into(),
                        detail: "@@ -45,6 +45,18 @@\n+    let tmp_path = format!(\"{}.tmp\", self.path.display());\n+    std::fs::write(&tmp_path, &serialized)?;\n+    let file = std::fs::File::open(&tmp_path)?;\n+    file.sync_all()?;\n+    std::fs::rename(&tmp_path, &self.path)?;".into(),
                        status: "success".into(),
                    },
                ],
                timestamp: "10:08".into(),
            },
        ],
        _ => vec![
            ChatMessage {
                id: "m1".into(),
                role: "user".into(),
                content: "帮我重构 orbit 的 compaction 策略，把固定 50 条改成字符预算模式".into(),
                tool_calls: vec![],
                timestamp: "14:30".into(),
            },
            ChatMessage {
                id: "m2".into(),
                role: "assistant".into(),
                content: "好的，我来看一下当前的 compaction 实现。\n改动集中在 `crates/orbit/src/lib.rs`，关键变化：\n- COMPACT_CHAR_BUDGET = 120_000\n- 保留最近 KEEP_RECENT_MIN = 4 条消息\n- 跳过 orphan tool_results".into(),
                tool_calls: vec![
                    ToolCall {
                        id: "tc-1".into(),
                        title: "已运行 git diff main..HEAD -- crates/orbit/".into(),
                        kind: "bash".into(),
                        summary: "1 个文件变动，45 行新增，20 行删除".into(),
                        detail: "diff --git a/crates/orbit/src/lib.rs b/crates/orbit/src/lib.rs\nindex 8666f2a..bd33257 100644\n--- a/crates/orbit/src/lib.rs\n+++ b/crates/orbit/src/lib.rs\n@@ -310,12 +310,24 @@ pub const COMPACT_CHAR_BUDGET: usize = 120_000;\n+    let mut char_count = 0;\n+    let mut kept_msgs = Vec::new();\n+    for msg in messages.iter().rev() {\n+        char_count += msg.content.len();\n+        if char_count > COMPACT_CHAR_BUDGET {\n+            break;\n+        }\n+        kept_msgs.push(msg.clone());\n+    }".into(),
                        status: "success".into(),
                    },
                    ToolCall {
                        id: "tc-2".into(),
                        title: "已写入 crates/orbit/src/lib.rs +45 -20".into(),
                        kind: "edit".into(),
                        summary: "替换固定计数为按字符预算的动态滑动窗口".into(),
                        detail: "@@ -319,10 +319,35 @@\n-    let slice = &messages[messages.len().saturating_sub(50)..];\n+    let kept_recent = messages.iter().rev().take(KEEP_RECENT_MIN).cloned().collect();\n+    // 字符预算窗口截断逻辑\n".into(),
                        status: "success".into(),
                    },
                ],
                timestamp: "14:31".into(),
            },
            ChatMessage {
                id: "m3".into(),
                role: "user".into(),
                content: "kept_chars 的守卫逻辑有 bug，system_prompt 没算进去".into(),
                tool_calls: vec![],
                timestamp: "14:35".into(),
            },
            ChatMessage {
                id: "m4".into(),
                role: "assistant".into(),
                content: "确认了。`compact()` 里 `kept_chars` 只统计了 `messages` 的字符，漏掉了 `system_prompt` 的长度。\n修复：在 `compact()` 开头把 `system_prompt.len()` 加入 `kept_chars` 初始值。\n测试通过。".into(),
                tool_calls: vec![
                    ToolCall {
                        id: "tc-3".into(),
                        title: "已写入 crates/orbit/src/lib.rs +2 -1".into(),
                        kind: "edit".into(),
                        summary: "在 kept_chars 累加初始值计入 system_prompt.len()".into(),
                        detail: "@@ -330,2 +330,3 @@\n-    let mut kept_chars = 0;\n+    let mut kept_chars = system_prompt.as_ref().map(|s| s.len()).unwrap_or(0);".into(),
                        status: "success".into(),
                    },
                ],
                timestamp: "14:36".into(),
            },
        ],
    }
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
            title: "EC-1 LLM 读超时闸门".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 0,
            description: "ureq::AgentBuilder 设置 timeout_read 避免半开连接挂死".into(),
            acceptance: "半开连接测试在 deadline 内返回 StreamEvent::Error".into(),
        },
        TaskItem {
            id: "t2".into(),
            title: "EC-2 run_bash 危险命令闸门".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 0,
            description: "引入 AST/规则级别命令安全评估，拦截 rm -rf / 等高危命令".into(),
            acceptance: "危险命令不触发 spawn，结构化拒绝并提醒模型".into(),
        },
        TaskItem {
            id: "t3".into(),
            title: "EC-3 Guarded name() 类型统一".into(),
            status: "in_progress".into(),
            kind: "bug".into(),
            priority: 1,
            description: "Guarded 统一为 &str 返回，允许 MCP 动态工具接入".into(),
            acceptance: "Guarded<McpTool> 编译通过且与内置工具行为一致".into(),
        },
        TaskItem {
            id: "t4".into(),
            title: "EC-4 runner 采纳 rpc 超时/重连".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 1,
            description: "read_event 出错时优先触发 reconnect，避免单次网络抖动失败".into(),
            acceptance: "fake-omp crash 场景下先重试一次再判定失败".into(),
        },
        TaskItem {
            id: "t5".into(),
            title: "EC-5 memory 接线".into(),
            status: "blocked".into(),
            kind: "feature".into(),
            priority: 2,
            description: "将 crates/memory 注册为 builtin 工具暴露给 agent".into(),
            acceptance: "memory_append / list / search 正常检索".into(),
        },
        TaskItem {
            id: "t6".into(),
            title: "EC-6 LLM 重试分类".into(),
            status: "open".into(),
            kind: "feature".into(),
            priority: 1,
            description: "429 与 5xx 区分于客户端错误，执行指数退避重试".into(),
            acceptance: "首个 delta 发出前的 429 自动触发重试".into(),
        },
        TaskItem {
            id: "t7".into(),
            title: "Dioxus Web UI 交互".into(),
            status: "done".into(),
            kind: "feature".into(),
            priority: 0,
            description: "全功能 LiveView 交互：会话切换、任务看板、工具展开面板".into(),
            acceptance: "浏览器可实时交互并响应点击、输入与筛选".into(),
        },
    ]
}

pub fn mock_stats() -> StatsData {
    mock_stats_for_range("24h")
}

pub fn mock_stats_for_range(range: &str) -> StatsData {
    match range {
        "1h" => StatsData {
            kpis: vec![
                KpiCard {
                    label: "费用估算".into(),
                    value: "$14.20".into(),
                    delta: "+2.1%".into(),
                    delta_positive: false,
                },
                KpiCard {
                    label: "请求数".into(),
                    value: "342".into(),
                    delta: "+8.4%".into(),
                    delta_positive: true,
                },
                KpiCard {
                    label: "缓存节省".into(),
                    value: "$5.10".into(),
                    delta: "+1.2%".into(),
                    delta_positive: true,
                },
                KpiCard {
                    label: "缓存率".into(),
                    value: "92.1%".into(),
                    delta: "+2.5%".into(),
                    delta_positive: true,
                },
                KpiCard {
                    label: "错误率".into(),
                    value: "1.2%".into(),
                    delta: "-0.5%".into(),
                    delta_positive: true,
                },
            ],
            sub_metrics: vec![
                SubMetric {
                    label: "UNCACHED INPUT".into(),
                    value: "4.8M".into(),
                },
                SubMetric {
                    label: "CACHE READ".into(),
                    value: "41M".into(),
                },
                SubMetric {
                    label: "OUTPUT TOKENS".into(),
                    value: "110K".into(),
                },
                SubMetric {
                    label: "CONVERSATION TOTAL".into(),
                    value: "46M".into(),
                },
                SubMetric {
                    label: "TOKENS/S".into(),
                    value: "28.4".into(),
                },
                SubMetric {
                    label: "AVG LATENCY".into(),
                    value: "11.2s".into(),
                },
                SubMetric {
                    label: "AVG TTFT".into(),
                    value: "8.5s".into(),
                },
            ],
            agent_bars: vec![
                AgentTokenBar {
                    agent: "Main agent".into(),
                    tokens: "38M".into(),
                    pct: 82.6,
                    color: "#ec4899".into(),
                },
                AgentTokenBar {
                    agent: "Subagents".into(),
                    tokens: "8M".into(),
                    pct: 17.4,
                    color: "#38bdf8".into(),
                },
            ],
            throughput: vec![
                ThroughputPoint {
                    time: "15:00".into(),
                    requests: 28.0,
                    tokens: 95.0,
                },
                ThroughputPoint {
                    time: "15:10".into(),
                    requests: 45.0,
                    tokens: 160.0,
                },
                ThroughputPoint {
                    time: "15:20".into(),
                    requests: 62.0,
                    tokens: 230.0,
                },
                ThroughputPoint {
                    time: "15:30".into(),
                    requests: 58.0,
                    tokens: 210.0,
                },
                ThroughputPoint {
                    time: "15:40".into(),
                    requests: 75.0,
                    tokens: 280.0,
                },
                ThroughputPoint {
                    time: "15:50".into(),
                    requests: 74.0,
                    tokens: 275.0,
                },
            ],
            feed: vec![
                FeedItem {
                    model: "qwen3-32b".into(),
                    provider: "AGNES".into(),
                    time_ago: "1 分钟前".into(),
                    duration: "4.2s".into(),
                    cost: "$0.002".into(),
                },
                FeedItem {
                    model: "agnes-2.5-flash".into(),
                    provider: "AGNES".into(),
                    time_ago: "3 分钟前".into(),
                    duration: "1.8s".into(),
                    cost: "$0.001".into(),
                },
                FeedItem {
                    model: "qwen3-32b".into(),
                    provider: "AGNES".into(),
                    time_ago: "7 分钟前".into(),
                    duration: "6.5s".into(),
                    cost: "$0.003".into(),
                },
            ],
        },
        "7d" => StatsData {
            kpis: vec![
                KpiCard {
                    label: "费用估算".into(),
                    value: "$2,410.80".into(),
                    delta: "+9.8%".into(),
                    delta_positive: false,
                },
                KpiCard {
                    label: "请求数".into(),
                    value: "59,200".into(),
                    delta: "+14.3%".into(),
                    delta_positive: true,
                },
                KpiCard {
                    label: "缓存节省".into(),
                    value: "$890.00".into(),
                    delta: "+4.2%".into(),
                    delta_positive: true,
                },
                KpiCard {
                    label: "缓存率".into(),
                    value: "88.2%".into(),
                    delta: "-0.4%".into(),
                    delta_positive: true,
                },
                KpiCard {
                    label: "错误率".into(),
                    value: "3.5%".into(),
                    delta: "-0.3%".into(),
                    delta_positive: true,
                },
            ],
            sub_metrics: vec![
                SubMetric {
                    label: "UNCACHED INPUT".into(),
                    value: "840M".into(),
                },
                SubMetric {
                    label: "CACHE READ".into(),
                    value: "6.8B".into(),
                },
                SubMetric {
                    label: "OUTPUT TOKENS".into(),
                    value: "18.2M".into(),
                },
                SubMetric {
                    label: "CONVERSATION TOTAL".into(),
                    value: "7.9B".into(),
                },
                SubMetric {
                    label: "TOKENS/S".into(),
                    value: "16.8".into(),
                },
                SubMetric {
                    label: "AVG LATENCY".into(),
                    value: "21.4s".into(),
                },
                SubMetric {
                    label: "AVG TTFT".into(),
                    value: "17.9s".into(),
                },
            ],
            agent_bars: vec![
                AgentTokenBar {
                    agent: "Main agent".into(),
                    tokens: "6.5B".into(),
                    pct: 82.3,
                    color: "#ec4899".into(),
                },
                AgentTokenBar {
                    agent: "Subagents".into(),
                    tokens: "1.4B".into(),
                    pct: 17.7,
                    color: "#38bdf8".into(),
                },
            ],
            throughput: vec![
                ThroughputPoint {
                    time: "周一".into(),
                    requests: 7200.0,
                    tokens: 950.0,
                },
                ThroughputPoint {
                    time: "周二".into(),
                    requests: 8900.0,
                    tokens: 1200.0,
                },
                ThroughputPoint {
                    time: "周三".into(),
                    requests: 8400.0,
                    tokens: 1100.0,
                },
                ThroughputPoint {
                    time: "周四".into(),
                    requests: 9300.0,
                    tokens: 1320.0,
                },
                ThroughputPoint {
                    time: "周五".into(),
                    requests: 10200.0,
                    tokens: 1450.0,
                },
                ThroughputPoint {
                    time: "周六".into(),
                    requests: 7800.0,
                    tokens: 1020.0,
                },
                ThroughputPoint {
                    time: "周日".into(),
                    requests: 7400.0,
                    tokens: 960.0,
                },
            ],
            feed: vec![
                FeedItem {
                    model: "qwen3-32b".into(),
                    provider: "AGNES".into(),
                    time_ago: "10 分钟前".into(),
                    duration: "12.3s".into(),
                    cost: "$0.004".into(),
                },
                FeedItem {
                    model: "kimi-k3".into(),
                    provider: "new-api".into(),
                    time_ago: "35 分钟前".into(),
                    duration: "8.1s".into(),
                    cost: "$0.003".into(),
                },
                FeedItem {
                    model: "agnes-2.5-flash".into(),
                    provider: "AGNES".into(),
                    time_ago: "2 小时前".into(),
                    duration: "3.2s".into(),
                    cost: "$0.001".into(),
                },
            ],
        },
        _ => StatsData {
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
                    color: "#ec4899".into(),
                },
                AgentTokenBar {
                    agent: "Subagents".into(),
                    tokens: "196M".into(),
                    pct: 16.8,
                    color: "#38bdf8".into(),
                },
            ],
            throughput: vec![
                ThroughputPoint {
                    time: "00:00".into(),
                    requests: 12.0,
                    tokens: 45.0,
                },
                ThroughputPoint {
                    time: "04:00".into(),
                    requests: 5.0,
                    tokens: 18.0,
                },
                ThroughputPoint {
                    time: "08:00".into(),
                    requests: 42.0,
                    tokens: 180.0,
                },
                ThroughputPoint {
                    time: "12:00".into(),
                    requests: 55.0,
                    tokens: 230.0,
                },
                ThroughputPoint {
                    time: "16:00".into(),
                    requests: 48.0,
                    tokens: 195.0,
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
            ],
        },
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
