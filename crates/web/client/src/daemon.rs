//! Web 侧 daemon 客户端封装（C5.2a 读侧真数据）。
//!
//! 全部方法阻塞式（一次一连接的 JSONL over UDS），调用方
//! （page-workspace）负责放进 `std::thread` 执行，避免卡渲染。
//! `WebDaemon` 只是 `daemon::DaemonClient` 的薄壳：构造时按
//! `config::Config::daemon_socket_path` 的解析规则找 socket，
//! 方法返回值直接映射成 `omenic-web-state` 的 UI DTO。

use std::collections::HashSet;
use std::path::PathBuf;

use daemon::{ClientError, DaemonClient};
use omenic_web_state::convert::{message_to_chat, summary_to_session};
use omenic_web_state::types::{ChatMessage, Session};
use session::SessionRole;

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
    /// `data_dir` 目前不参与 socket 解析（config 侧也不从 data_dir 推导
    /// socket），保留入参以对齐 `LlmRuntimeConfig` 的调用形状。
    pub fn from_data_dir(data_dir: &str) -> Option<WebDaemon> {
        let _ = data_dir;
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
        self.client.session_create(sid, title)?;
        Ok(())
    }

    /// 删除会话（连带消息）；会话不存在同样返回 Ok。
    pub fn delete_session(&self, sid: &str) -> Result<(), ClientError> {
        self.client.session_delete(sid)?;
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
