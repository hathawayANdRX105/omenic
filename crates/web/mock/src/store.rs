//! Fixture 数据：形状与真实数据一致，G4 后由 `omenic-web-client` 取代。

use omenic_web_state::types::{
    AgentTokenBar, ChatMessage, FeedItem, KpiCard, MessagePart, Session, SessionStatus, StatsData,
    StatusLine, SubMetric, TaskItem, ThroughputPoint, WorkspaceSpace, format_relative_time,
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn session(id: &str, title: &str, age_min: u64, status: SessionStatus, model: &str) -> Session {
    let epoch = now_ms().saturating_sub(age_min * 60_000);
    Session {
        id: id.into(),
        title: title.into(),
        last_active: format_relative_time(epoch),
        model: model.into(),
        status,
        last_active_epoch: epoch,
    }
}

// ── Sidebar：项目（worktree）+ 各自会话 ─────────────────────────────────────

pub fn spaces() -> Vec<WorkspaceSpace> {
    vec![
        WorkspaceSpace {
            id: "~/projects/omenic".into(),
            name: "omenic (main)".into(),
            path: "~/projects/omenic".into(),
            branch: "main".into(),
            is_active: true,
        },
        WorkspaceSpace {
            id: "~/projects/omenic/.wt/web-dsh".into(),
            name: "web-dsh".into(),
            path: "~/projects/omenic/.wt/web-dsh".into(),
            branch: "feat/web-dsh-replica".into(),
            is_active: false,
        },
        WorkspaceSpace {
            id: "~/projects/omenic/.wt/harness-plugin".into(),
            name: "harness-plugin".into(),
            path: "~/projects/omenic/.wt/harness-plugin".into(),
            branch: "feat/plugin-face".into(),
            is_active: false,
        },
    ]
}

pub fn sessions_for_space(space_path: &str) -> Vec<Session> {
    if space_path.ends_with("web-dsh") {
        vec![
            session(
                "s-dsh-1",
                "dsh 视觉规格提取",
                18,
                SessionStatus::Idle,
                "deepseek-v4-flash",
            ),
            session(
                "s-dsh-2",
                "composer 折叠卡对齐",
                95,
                SessionStatus::Archived,
                "qwen3-32b",
            ),
        ]
    } else if space_path.ends_with("harness-plugin") {
        vec![session(
            "s-plug-1",
            "6.1 服务注册容器",
            240,
            SessionStatus::Active,
            "claude-opus-4-7",
        )]
    } else {
        vec![
            session(
                "s-compaction",
                "重构 orbit compaction 预算窗口",
                3,
                SessionStatus::Idle,
                "qwen3-32b",
            ),
            session(
                "s-mcp",
                "MCP 工具接线",
                22,
                SessionStatus::Idle,
                "qwen3-32b",
            ),
            session(
                "s-memory",
                "memory store 断电修复",
                65,
                SessionStatus::Idle,
                "agnes-2.5-flash",
            ),
            session(
                "s-sidebar",
                "sidebar 三级树复刻",
                190,
                SessionStatus::Archived,
                "agnes-2.5-flash",
            ),
            session(
                "s-minimap",
                "minimap 提示词导航",
                1_500,
                SessionStatus::Archived,
                "agnes-2.5-flash",
            ),
        ]
    }
}

// ── 会话消息 ────────────────────────────────────────────────────────────────

fn tc(id: &str, kind: &str, title: &str, summary: &str, detail: &str) -> ToolCallLike {
    ToolCallLike {
        id: id.into(),
        kind: kind.into(),
        title: title.into(),
        summary: summary.into(),
        detail: detail.into(),
    }
}

/// 本地构造辅助（避免在 fixture 里拼 state::ToolCall 的 status 样板）。
struct ToolCallLike {
    id: String,
    kind: String,
    title: String,
    summary: String,
    detail: String,
}

impl ToolCallLike {
    fn into_tool(self) -> omenic_web_state::types::ToolCall {
        omenic_web_state::types::ToolCall {
            id: self.id,
            title: self.title,
            kind: self.kind,
            summary: self.summary,
            detail: self.detail,
            status: "success".into(),
        }
    }
}

fn user_msg(id: &str, content: &str, age_min: u64) -> ChatMessage {
    let epoch = now_ms().saturating_sub(age_min * 60_000);
    ChatMessage {
        id: id.into(),
        role: "user".into(),
        content: content.into(),
        tool_calls: vec![],
        parts: vec![],
        timestamp: format_relative_time(epoch),
        ts_epoch_ms: epoch,
    }
}

fn assistant_msg(id: &str, parts: Vec<MessagePart>, age_min: u64) -> ChatMessage {
    let epoch = now_ms().saturating_sub(age_min * 60_000);
    let content = parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    ChatMessage {
        id: id.into(),
        role: "assistant".into(),
        content,
        tool_calls: parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => Some(tc.clone()),
                _ => None,
            })
            .collect(),
        parts,
        timestamp: format_relative_time(epoch),
        ts_epoch_ms: epoch,
    }
}

pub fn messages_for_session(session_id: &str) -> Vec<ChatMessage> {
    match session_id {
        "s-mcp" => vec![
            user_msg("s2-m1", "我们需要将外部 MCP 服务的工具注册进 worker 的 runner 中。", 25),
            assistant_msg(
                "s2-m2",
                vec![
                    MessagePart::Text("已检查 MCP Client 与 runner 的接口契约，计划在 `register()` 调用阶段做动态注入。".into()),
                    MessagePart::Tool(tc(
                        "tc-s2-1",
                        "bash",
                        "grep -n 'builtin_tools' crates/task/src/runner.rs",
                        "定位到 builtin_tools 注册点 runner.rs:203",
                        "crates/task/src/runner.rs:203:        let mut tools = tools::builtin_tools();\ncrates/task/src/runner.rs:204:        tools.extend(mcp_tools);",
                    ).into_tool()),
                    MessagePart::Text("契约兼容：`Guarded` 包装的 name() 返回 `&str`，MCP 动态工具可以直接适配。接下来把 `mcp_tools` 的生命周期挂到 runner 上。".into()),
                ],
                24,
            ),
        ],
        "s-memory" => vec![
            user_msg("s3-m1", "排查 memory store 在意外断电时可能发生的数据截断问题。", 70),
            assistant_msg(
                "s3-m2",
                vec![
                    MessagePart::Text("定位到 `sync_all` 阶段缺少原子重命名机制。改用临时文件写入 + fsync + rename 保证事务完整。".into()),
                    MessagePart::Tool(tc(
                        "tc-s3-1",
                        "edit",
                        "crates/memory/src/store.rs +18 -4",
                        "引入 tempfile + fs::rename 原子落盘",
                        "@@ -45,6 +45,18 @@\n+    let tmp_path = format!(\"{}.tmp\", self.path.display());\n+    std::fs::write(&tmp_path, &serialized)?;\n+    let file = std::fs::File::open(&tmp_path)?;\n+    file.sync_all()?;\n+    std::fs::rename(&tmp_path, &self.path)?;",
                    ).into_tool()),
                    MessagePart::Text("修复完成。断电场景下最坏丢失最后一笔 append，不再出现半写记录。回归测试 `memory::tests::torn_write` 通过。".into()),
                ],
                68,
            ),
        ],
        _ => vec![
            user_msg("s-c-1", "帮我重构 orbit 的 compaction 策略，把固定 50 条改成字符预算模式", 6),
            assistant_msg(
                "s-c-2",
                vec![
                    MessagePart::Text("好的，我先看一下当前的 compaction 实现，并确认改动范围。".into()),
                    MessagePart::Tool(tc(
                        "tc-1",
                        "bash",
                        "git diff main..HEAD -- crates/orbit/",
                        "1 个文件变动，45 行新增，20 行删除",
                        "diff --git a/crates/orbit/src/lib.rs b/crates/orbit/src/lib.rs\nindex 8666f2a..bd33257 100644\n--- a/crates/orbit/src/lib.rs\n+++ b/crates/orbit/src/lib.rs\n@@ -310,12 +310,24 @@ pub const COMPACT_CHAR_BUDGET: usize = 120_000;\n+    let mut char_count = 0;\n+    let mut kept_msgs = Vec::new();\n+    for msg in messages.iter().rev() {\n+        char_count += msg.content.len();\n+        if char_count > COMPACT_CHAR_BUDGET { break; }\n+        kept_msgs.push(msg.clone());\n+    }",
                    ).into_tool()),
                    MessagePart::Tool(tc(
                        "tc-2",
                        "grep",
                        "grep -n 'COMPACT_CHAR_BUDGET' crates/orbit/src/lib.rs",
                        "命中 1 处",
                        "crates/orbit/src/lib.rs:310:pub const COMPACT_CHAR_BUDGET: usize = 120_000;",
                    ).into_tool()),
                    MessagePart::Text("改动集中在 `crates/orbit/src/lib.rs`：引入字符预算窗口，保留最近 `KEEP_RECENT_MIN` 条消息，跳过 orphan tool_results。".into()),
                    MessagePart::Tool(tc(
                        "tc-3",
                        "edit",
                        "crates/orbit/src/lib.rs +45 -20",
                        "替换固定计数为按字符预算的动态滑动窗口",
                        "@@ -319,10 +319,35 @@\n-    let slice = &messages[messages.len().saturating_sub(50)..];\n+    let kept_recent = messages.iter().rev().take(KEEP_RECENT_MIN).cloned().collect();\n+    // 字符预算窗口截断逻辑",
                    ).into_tool()),
                    MessagePart::Text("完成。compaction 现在按字符预算（`COMPACT_CHAR_BUDGET = 120_000`）动态滑动窗口截断，并保留最近 4 条消息。22 个集成测试全部通过：\n\n```text\ntest result: ok. 22 passed; 0 failed\n```".into()),
                ],
                5,
            ),
            user_msg("s-c-3", "kept_chars 的守卫逻辑有 bug，system_prompt 没算进去", 2),
            assistant_msg(
                "s-c-4",
                vec![
                    MessagePart::Text("确认了。`compact()` 里 `kept_chars` 只统计了 `messages` 的字符，漏掉了 `system_prompt` 的长度。\n\n修复：在 `compact()` 开头把 `system_prompt.len()` 加入 `kept_chars` 初始值。测试通过。".into()),
                ],
                1,
            ),
        ],
    }
}

// ── 任务看板 ────────────────────────────────────────────────────────────────

pub fn tasks() -> Vec<TaskItem> {
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

// ── 统计 ────────────────────────────────────────────────────────────────────

pub fn stats_for_range(range: &str) -> StatsData {
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
                    color: "#679efe".into(),
                },
                AgentTokenBar {
                    agent: "Subagents".into(),
                    tokens: "8M".into(),
                    pct: 17.4,
                    color: "#4ed17e".into(),
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
                    color: "#679efe".into(),
                },
                AgentTokenBar {
                    agent: "Subagents".into(),
                    tokens: "1.4B".into(),
                    pct: 17.7,
                    color: "#4ed17e".into(),
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
                    color: "#679efe".into(),
                },
                AgentTokenBar {
                    agent: "Subagents".into(),
                    tokens: "196M".into(),
                    pct: 16.8,
                    color: "#4ed17e".into(),
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

// ── 状态条 / 模型清单 ───────────────────────────────────────────────────────

pub fn statusline() -> StatusLine {
    StatusLine {
        model: "qwen3-32b".into(),
        thinking: "off".into(),
        cwd: "~/projects/omenic".into(),
        git_branch: "feat/web-dsh-replica".into(),
        tokens_in: 45_230,
        tokens_out: 12_847,
        cost_usd: 0.023,
        context_pct: 34.2,
        context_max: 128_000,
        // WP-C 的计时字段。mock 无真实 run：无在飞 run，总耗时归零。
        // 本 crate 由 WP-D 整体移除，这里只补齐字段让 workspace 可编译。
        run_started_at_ms: None,
        elapsed_ms: 0,
    }
}

pub const MODELS: &[&str] = &[
    "deepseek-v4-flash",
    "qwen3-32b",
    "agnes-2.5-flash",
    "claude-opus-4-7",
    "kimi-k3",
];

pub const THINKING_OPTIONS: &[(&str, &str)] = &[
    ("关闭", "off"),
    ("轻量", "2k"),
    ("标准", "8k"),
    ("深度", "16k"),
];
