//! slash — T9 封闭斜杠命令注册表 + 模糊过滤 + `/help` 自动输出
//! （route §3 T9，行为正本）。
//!
//! 形态取 dh-rs `tui/command_palette.rs`（任务书 §2 实录）：**名称 + 一行
//! 描述 + 执行动作同表**——[`COMMANDS`] 是封闭 const 集合，没有动态命令
//! 发现、没有插件注册（route §8 禁止项）。面板候选与 `/help` 输出读同一
//! 张表，故 jcode 口径成立：*新注册的命令不可能缺席帮助*（
//! [`help_text`] 遍历生成，测试 `tests/help_autogen.rs` 逐条钉）。
//!
//! 本模块无 IO：[`Intent`] 是需要事件循环做 IO 的命令意图（如开 picker），
//! 由 `App` 入队、`app.rs` 事件循环消费执行；命令执行不进 prompt 提交
//! 路径（route §3 T9：不产生模型回合）。
//!
//! 模糊过滤是自实现的 subsequence 匹配（omp `fuzzy.ts` 只作参考，不引
//! 依赖、不移植，任务书 §2 取舍）。

/// 命令执行动作（注册表字段之一）：match 必须列全——给注册表加新动作先
/// 在这里过编译器，`App::run_command` 随之被迫逐条答复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `/help`：输出帮助（本地 transcript）。
    Help,
    /// `/clear`：清本地视图。
    Clear,
    /// `/model`：回显当前 model。
    Model,
    /// `/sessions`：开 T4 会话 picker（落点即 Ctrl+K 同一处）。
    Sessions,
    /// `/theme`：回显当前主题。
    Theme,
    /// `/search`：开 T11 转录搜索 overlay（与 Ctrl+R 同一落点）。
    Search,
}

/// 需要事件循环做 IO 的命令意图（[`App`](crate::app::App) 是无 IO 纯状态：
/// 只入队不出手，RPC/picker 归 `app.rs` 事件循环）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// `/sessions`：开会话 picker（与 Ctrl+K 同一落点、同一运行中不开口径）。
    OpenSessions,
}

/// 一条注册项：名称（含前导 `/`）+ 一行描述 + 执行动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// 命令名（含前导 `/`，整行 trim 后须与它精确相等才算命中）。
    pub name: &'static str,
    /// 一行描述（面板与 `/help` 共用）。
    pub description: &'static str,
    /// 执行动作。
    pub action: Action,
}

/// T9 首批封闭注册表（route §3 T9 定的五条）：**顺序即面板默认顺序**。
/// 只接已有能力——落点见各 `action` 的 `App::run_command` 臂。
pub const COMMANDS: &[Command] = &[
    Command {
        name: "/help",
        description: "list all slash commands",
        action: Action::Help,
    },
    Command {
        name: "/clear",
        description: "clear the transcript view",
        action: Action::Clear,
    },
    Command {
        name: "/model",
        description: "show the current model",
        action: Action::Model,
    },
    Command {
        name: "/sessions",
        description: "open the session picker",
        action: Action::Sessions,
    },
    Command {
        name: "/theme",
        description: "show the current theme",
        action: Action::Theme,
    },
    // T11：追加在**表尾**（首批五条之外的第 6 条）——顺序即面板默认顺序，
    // `tests/help_autogen.rs` / `tests/slash_palette.rs` 的登记行同步 +1。
    Command {
        name: "/search",
        description: "search the transcript (Ctrl+R)",
        action: Action::Search,
    },
];

/// 精确命中：trim 后的整行 == 注册名（`/help extra` 不算）。
pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|cmd| cmd.name == name)
}

/// 模糊过滤：`query` 是 composer 整行（行首 `/`），按 subsequence 顺序
/// 匹配命令名（大小写不敏感、空白不参与匹配）；空查询 = 全量。
/// 结果保持注册表顺序（面板高亮游标按这个顺序走）。
pub fn filter(query: &str) -> Vec<&'static Command> {
    let query = query.trim();
    COMMANDS
        .iter()
        .filter(|cmd| fuzzy_match(query, cmd.name))
        .collect()
}

/// subsequence 匹配（大小写不敏感）：`query` 的字符按序出现在 `target`
/// 即命中——`"/se"` → `/sessions`、`"/hp"` → `/help`（非子串也命中）、
/// `"/zz"` 无命中。自实现：不引 fuzzy 依赖（任务书 §2 取舍）。
pub fn fuzzy_match(query: &str, target: &str) -> bool {
    let query = query.to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    let mut chars = target.chars();
    query.chars().all(|q| chars.any(|t| t == q))
}

/// `/help` 正文：**遍历 [`COMMANDS`] 生成**（jcode 口径：新注册命令必现，
/// 不手写命令清单）。表头 1 行 + 每条命令 1 行。
pub fn help_text() -> String {
    let mut out = String::from("slash commands:");
    for cmd in COMMANDS {
        out.push('\n');
        out.push_str(cmd.name);
        out.push_str(" — ");
        out.push_str(cmd.description);
    }
    out
}
