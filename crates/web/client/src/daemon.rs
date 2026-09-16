//! Web 侧 daemon 客户端封装（C5.2a 读侧真数据）。
//!
//! 全部方法阻塞式（一次一连接的 JSONL over UDS），调用方
//! （page-workspace）负责放进 `std::thread` 执行，避免卡渲染。
//! `WebDaemon` 只是 `daemon::DaemonClient` 的薄壳：构造时按
//! `config::Config::daemon_socket_path` 的解析规则找 socket，
//! 方法返回值直接映射成 `omenic-web-state` 的 UI DTO。

use std::collections::HashSet;
use std::path::PathBuf;

use daemon::{ClientError, Command, DaemonClient, Subscription};
use omenic_web_state::convert::{message_to_chat, summary_to_session};
use omenic_web_state::types::{ChatMessage, Session};
use serde_json::Value;
use session::SessionRole;

/// daemon 侧的统计 DTO 原样转出，供 page-stats 直接消费——统计页没有
/// 需要额外映射的展示形状（KPI 文案在页面里现算），再抄一层 UI DTO 只会
/// 制造两处需要同步的定义。
pub use daemon::state::{STATS_UNAVAILABLE, StatsBucket, StatsRecentRun, StatsSummary};

/// 阻塞式 daemon 客户端。Clone 便宜（内部只有 socket 路径）。
#[derive(Debug, Clone)]
pub struct WebDaemon {
    client: DaemonClient,
}

impl WebDaemon {
    /// 按 config 的解析规则拿 socket 并构造客户端；socket 路径不存在
    /// 返回 `None`。
    ///
    /// 解析规则与 `config::Config::daemon_socket_path` 完全一致：
    /// 1. `OMENIC_DAEMON_SOCKET` 环境变量（非空时原样使用）；
    /// 2. 平台配置目录 + `omenic/daemon.sock`（Unix：`$XDG_CONFIG_HOME`
    ///    非空否则 `$HOME/.config`）。
    ///
    /// socket 不从 data_dir 推导（config 侧也不这么推导），所以这里没有
    /// 入参——旧名 `from_data_dir` 收一个被忽略的 `&str`，调用方会误以为
    /// 传了 data_dir 就能选 daemon。
    pub fn from_env_or_default() -> Option<WebDaemon> {
        let socket = resolve_socket_path()?;
        if !socket.exists() {
            return None;
        }
        Some(WebDaemon {
            client: DaemonClient::connect_to(socket),
        })
    }

    /// 显式指定 socket 路径（集成测试用），不做存在性检查。
    pub fn connect_to(socket: impl Into<PathBuf>) -> WebDaemon {
        WebDaemon {
            client: DaemonClient::connect_to(socket),
        }
    }

    /// daemon 是否可达（连不上 / ping 失败都算 false）。
    pub fn ping(&self) -> bool {
        self.client.ping().unwrap_or(false)
    }

    /// 会话列表（存储侧按最近活跃排序）。DB 的 list 是 LIKE 查询、
    /// 空 query 会被拒，`"%"` 通配即全量。
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<Session>, ClientError> {
        let rows = self.client.session_list("%", limit)?;
        Ok(rows.iter().map(summary_to_session).collect())
    }

    /// 拉取一个会话的消息（按 seq 升序）。
    pub fn load_messages(&self, sid: &str, limit: u32) -> Result<Vec<ChatMessage>, ClientError> {
        let rows = self.client.session_load_messages(sid, limit)?;
        Ok(rows.iter().map(message_to_chat).collect())
    }

    /// 按消息文本搜会话：`session.search` 的命中按 session 去重后映射回
    /// `Session` 列表。空 query 退化为全量列表（与 UI ⌘K 行为一致，
    /// 也避开存储侧对空 query 的拒绝）。
    pub fn search_sessions(&self, query: &str, limit: u32) -> Result<Vec<Session>, ClientError> {
        if query.trim().is_empty() {
            return self.list_sessions(limit);
        }
        let hits = self.client.session_search(query, None, limit)?;
        let mut out: Vec<Session> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for hit in &hits {
            if !seen.insert(hit.session_id.clone()) {
                continue;
            }
            if let Some(summary) = self.client.session_get(&hit.session_id)? {
                out.push(summary_to_session(&summary));
            }
        }
        Ok(out)
    }

    /// 新建会话（存储侧幂等：已存在的 id 原样返回原行）。
    pub fn create_session(&self, sid: &str, title: &str) -> Result<(), ClientError> {
        self.create_session_with_parent(sid, title, None)
    }

    /// [`Self::create_session`] 的带谱系版本：`parent_id` 指明本会话从哪个
    /// 会话 fork 而来（5.3/5.5 分组的父子边），`None` 建根会话。返回值
    /// 仍是 `()`——调用方（page-workspace）拿内存里的 id 直接插列表，
    /// 不需要 daemon 回执的 summary。
    pub fn create_session_with_parent(
        &self,
        sid: &str,
        title: &str,
        parent_id: Option<&str>,
    ) -> Result<(), ClientError> {
        self.client
            .session_create_with_parent(sid, title, parent_id)?;
        Ok(())
    }

    /// 删除会话（连带消息）；会话不存在同样返回 Ok。
    pub fn delete_session(&self, sid: &str) -> Result<(), ClientError> {
        self.client.session_delete(sid)?;
        Ok(())
    }

    /// 中止当前运行（orbit 模式置 abort 标志；omp 模式转发 abort）。
    pub fn abort_worker(&self) -> Result<(), ClientError> {
        self.client.call_raw(
            daemon::protocol::Command::WorkerAbort,
            serde_json::json!({}),
        )?;
        Ok(())
    }

    /// 追加一条消息：`role_user` 为 true 是用户，否则 assistant。
    pub fn append_message(
        &self,
        sid: &str,
        role_user: bool,
        text: &str,
    ) -> Result<(), ClientError> {
        let role = if role_user {
            SessionRole::User
        } else {
            SessionRole::Assistant
        };
        self.client.session_append(sid, role, text)?;
        Ok(())
    }

    /// `worker.prompt`：把一条用户消息转交 daemon 的 omp worker（首次调用
    /// 自动拉起 worker 进程）。返回 worker 的原始 rpc 响应值；实际回复内容
    /// 全部走 [`Self::subscribe_worker`] 的事件推送，调用方（page-workspace）
    /// 通常直接忽略返回值。阻塞到 worker 应答——调用方须放进 `std::thread`。
    pub fn worker_prompt(&self, message: &str) -> Result<Value, ClientError> {
        self.client.call(
            Command::WorkerPrompt,
            serde_json::json!({ "message": message }),
        )
    }

    /// `worker.prompt` 的带归属版本（WP-C）：把 `session_id` / `run_id` 透传
    /// 给 daemon，daemon 据此把 run 记进 run ledger（`runs.start`，prompt
    /// 返回时写结束状态）。run_id 由调用方生成（`r-<epoch_ms>`，与 CLI 的
    /// `session resume` 同一格式），刷新后 [`Self::runs_for_session`] 拿它
    /// 组装出 Active/Aborted。仅加调用入参，daemon 侧这两个字段本来就是
    /// 可选的，协议零改动。
    pub fn worker_prompt_run(
        &self,
        session_id: &str,
        run_id: &str,
        message: &str,
    ) -> Result<Value, ClientError> {
        self.client.call(
            Command::WorkerPrompt,
            serde_json::json!({
                "message": message,
                "session_id": session_id,
                "run_id": run_id,
            }),
        )
    }

    /// `event.subscribe("worker")`：worker 事件推送的专用长连接（C5.2b）。
    /// 连接存活期间 daemon 持续推送 `EventFrame`；Drop 关连接，daemon 自动
    /// 分离该连接的全部订阅（断线清理）。广播语义：多订阅者各收全流。
    pub fn subscribe_worker(&self) -> Result<Subscription, ClientError> {
        self.client.subscribe("worker")
    }

    /// `event.subscribe("worker")` 的 per-run 视图（G7-B）：返回的订阅只
    /// 投递被 daemon 打上 `run_id` 归属的事件帧；属于其他 run 的事件（用户
    /// 中途切了会话、或另一个会话的 turn 正在跑）在读循环里直接丢弃，不混入
    /// 当前会话视图。daemon 无法归属的帧（旧 daemon / 归属 prompt 之外的事件）
    /// 仍然放行——见 [`daemon::EventFrame::belongs_to_run`]。
    ///
    /// `run_id` 须非空（调用方生成的 `r-<epoch_ms>`）；没有 run 归属的调用方
    /// 用 [`Self::subscribe_worker`] 拿全量流。连接语义与后者完全相同。
    pub fn subscribe_worker_run(
        &self,
        run_id: &str,
    ) -> Result<RunFilteredSubscription, ClientError> {
        let inner = self.client.subscribe("worker")?;
        Ok(RunFilteredSubscription {
            inner,
            run_id: run_id.to_string(),
        })
    }

    /// `run.list`（按 session 过滤）：返回该会话的 run 记录，供列表侧组装
    /// 运行状态（WP-C：半开 run → aborted，见
    /// `omenic_web_state::convert::infer_session_status`）。daemon 的
    /// `run.list` 语义只有全量 `{ limit }` → `[RunRecord]`，不可改，故在
    /// 客户端按 `session_id` 过滤。空列表合法（会话从未跑过 run）。
    pub fn runs_for_session(
        &self,
        sid: &str,
        limit: u32,
    ) -> Result<Vec<daemon::state::RunRecord>, ClientError> {
        let runs = self.client.run_list(limit)?;
        Ok(runs.into_iter().filter(|r| r.session_id == sid).collect())
    }

    /// `stats.summary`（C5.7）：统计页的唯一数据源。`range` 取
    /// `"1h"`/`"24h"`/`"7d"`/`"30d"`/`"90d"`/`"All"`，未知值 daemon 侧退化
    /// 成 24h（不会报错）。聚合全部来自 run ledger；run 记录里没有 token /
    /// 费用列，那些指标由 [`StatsSummary::unavailable`] 列出，页面据此隐藏
    /// 对应卡片而不是显示编造的零。
    ///
    /// 阻塞式，调用方必须放进 `std::thread`（LiveView 渲染路径里同步 RPC
    /// 会撞 "runtime within a runtime"）。
    pub fn stats_summary(&self, range: &str) -> Result<StatsSummary, ClientError> {
        self.client.stats_summary(range)
    }
}

/// Run-scoped view of a worker subscription (G7-B).  Wraps the raw
/// [`Subscription`] from [`WebDaemon::subscribe_worker`] and yields only the
/// frames the daemon attributed to `run_id`, so events belonging to another
/// run never reach the current session view.  Read API mirrors
/// [`Subscription::next_event`] (same keepalive / disconnect contract), so a
/// caller can swap one for the other without changing its loop.
pub struct RunFilteredSubscription {
    inner: Subscription,
    run_id: String,
}

impl RunFilteredSubscription {
    /// Id to pass to `event.unsubscribe`（透传内层订阅）。
    pub fn id(&self) -> u64 {
        self.inner.id()
    }

    /// 读下一个属于本 run 的事件帧。`Ok(None)` = `dur` 内没有匹配帧到达
    /// （keepalive tick 语义不变），或 daemon 在帧中途关了流；`Err` = 断线。
    /// 其他 run 的帧被丢弃后继续等待，不计入超时预算：用的是截止时间而非
    /// 每帧超时，跳过一阵外来事件不会拖长调用方的 tick 间隔。
    pub fn next_event(
        &mut self,
        dur: std::time::Duration,
    ) -> Result<Option<daemon::EventFrame>, ClientError> {
        let deadline = std::time::Instant::now()
            .checked_add(dur)
            .unwrap_or_else(std::time::Instant::now);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match self.inner.next_event(remaining)? {
                None => return Ok(None),
                Some(frame) if frame.belongs_to_run(&self.run_id) => return Ok(Some(frame)),
                Some(_) => {
                    // 另一个 run 的事件：丢弃，不混入当前视图。
                    continue;
                }
            }
        }
    }
}

/// 与 `config::Config::daemon_socket_path` 相同的 socket 解析。
fn resolve_socket_path() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("OMENIC_DAEMON_SOCKET")
        && !v.is_empty()
    {
        return Some(PathBuf::from(v));
    }
    let base = platform_config_dir()?;
    Some(base.join("omenic").join("daemon.sock"))
}

/// 与 `config::platform_config_dir` 相同的平台配置目录解析。
fn platform_config_dir() -> Option<PathBuf> {
    #[cfg(target_family = "unix")]
    {
        if let Some(v) = std::env::var_os("XDG_CONFIG_HOME")
            && !v.is_empty()
        {
            return Some(PathBuf::from(v));
        }
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .map(|p| p.join(".config"))
    }
    #[cfg(not(target_family = "unix"))]
    {
        // daemon 的 UDS 监听本身只有 Unix 实现，其余平台暂不解析
        None
    }
}
