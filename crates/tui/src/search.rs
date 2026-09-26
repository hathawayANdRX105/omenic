//! search — T11 转录搜索 overlay 的纯状态（route §3 T11，行为正本）。
//!
//! 三层切分与 T4 picker 同构（任务书 §3⑤ 照 picker 范式）：[`SearchState`]
//! 是无 IO 纯状态（按键进去、状态与视口快照出来）；渲染在
//! [`crate::ui::search_overlay`]；真 RPC 由 `app.rs` 事件循环按
//! [`SearchState::dirty`] 消费（App 无 IO，同 T9 intent 的分工）。
//!
//! 查询语义走**已有** `session.search`（scope = 当前会话，存储侧
//! `LIKE … COLLATE NOCASE` 子串匹配）：命中回执是整条消息，excerpt 与
//! 高亮区间在客户端按 query 现算——协议不需要 excerpt 字段，daemon/protocol
//! 零增补（任务书 §2 取舍：能不改就不改）。子串匹配、无正则、无搜索历史
//! （每次开 overlay 复位查询，route §8 非目标）。

use std::collections::HashMap;

use web_state::types::ChatMessage;

use crate::scroll::ScrollModel;

/// 一次拉取的命中上限（存储侧 `session::MAX_LIMIT` = 1000 之内）。
/// 打满即按 `N+ matches` 显示——结果可能被截断，不假装是全量。
pub const FETCH_LIMIT: u32 = 200;

/// overlay 占行数：查询行 + 当前命中 excerpt 行 + 状态/按键提示行。
/// 恒定行数让视口几何在开合期间稳定（只在开合两刻变化一次）。
pub const OVERLAY_ROWS: u16 = 3;

/// 一条命中：RPC 回执的一条消息 + 映射到当前 transcript 的消息下标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// ledger 行 id（`{session_id}-{seq}`，`message_to_chat` 的 id 口径）。
    pub id: String,
    /// 角色（`user` / `assistant`——内容撞车时的映射判据之一）。
    pub role: String,
    /// 消息全文（excerpt / 高亮区间在渲染侧按 query 现算）。
    pub text: String,
    /// 在当前 transcript 投影中的消息下标；`None` = 不在视图里（历史回填
    /// 截断、或本地新推消息 ledger 还没有）——仍进命中列表（真值：daemon
    /// 侧确实有），只是跳转定位跳不过去。
    pub msg: Option<usize>,
}

/// 把 `session.search` 回执映射到当前 transcript 投影的消息下标
/// （跳转定位的前提：命中要能在视图里找到那一行）。两遍：
///
/// 1. **id 精确命中**：历史回填的消息 id 就是 `{session_id}-{seq}`，与回执
///    同源——同内容多条也不会撞；
/// 2. **角色 + 全文单调配对**：id 对不上的（本地新推消息用 `user-<ts>` 那套
///    id）按出现顺序向前找第一个未占用的同角色同文消息——RPC 与 transcript
///    同为 seq 升序，单调配对保证同文多条一一对应、不跳着抢。
///
/// 对不上的命中 `msg = None`（见 [`Hit::msg`]），不编造行号。
pub fn map_hits(messages: &[ChatMessage], rows: &[ChatMessage]) -> Vec<Hit> {
    let mut hits: Vec<Hit> = rows
        .iter()
        .map(|row| Hit {
            id: row.id.clone(),
            role: row.role.clone(),
            text: row.content.clone(),
            msg: None,
        })
        .collect();
    let mut taken = vec![false; messages.len()];
    {
        let mut by_id: HashMap<&str, usize> = HashMap::new();
        for (i, msg) in messages.iter().enumerate() {
            by_id.entry(msg.id.as_str()).or_insert(i);
        }
        for hit in &mut hits {
            if let Some(&i) = by_id.get(hit.id.as_str()) {
                hit.msg = Some(i);
                taken[i] = true;
            }
        }
    }
    let mut cursor = 0;
    for hit in &mut hits {
        if hit.msg.is_some() {
            continue;
        }
        let found = (cursor..messages.len()).find(|&i| {
            !taken[i] && messages[i].role == hit.role && messages[i].content == hit.text
        });
        if let Some(i) = found {
            hit.msg = Some(i);
            taken[i] = true;
            cursor = i + 1;
        }
    }
    hits
}

/// 子串高亮区间（字节半开区间，落在净化后的展示文本上）：大小写不敏感，
/// 对齐存储侧 `LIKE … COLLATE NOCASE`——`to_ascii_lowercase` 逐字节 1:1
/// 替换（非 ASCII 字节原样），偏移可直接切原文。空查询 = 空区间
/// （不把整行都涂成命中）。
pub fn match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_ascii_lowercase();
    text.to_ascii_lowercase()
        .match_indices(&needle)
        .map(|(at, _)| (at, at + needle.len()))
        .collect()
}

/// T11 搜索 overlay 纯状态（无 IO，可被测试直接驱动；`Default` = 关闭态）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchState {
    /// overlay 开合（开 = 键路由短路 + `ui::areas` 让行，两处同源）。
    open: bool,
    /// 当前查询词（每次开 overlay 复位——不做搜索历史，route §8 非目标）。
    query: String,
    /// 命中列表（查询变更即清空，绝不显示旧查询的结果）。
    hits: Vec<Hit>,
    /// 当前命中下标（[`Self::current_hit`] 渲染 excerpt、跳转定位同一项）。
    current: usize,
    /// 是否已跳过（`false` = 尚未 Enter/Shift+Enter：视口还没被搜索动过，
    /// 下一次 Enter 跳当前项、Shift+Enter 循环到末项）。
    jumped: bool,
    /// 查询变更待取数（事件循环按此消费 RPC，App 无 IO）。
    dirty: bool,
    /// 回执条数触到 [`FETCH_LIMIT`]：结果可能被截断（`N+ matches` 不装全量）。
    capped: bool,
    /// 最近一次 RPC 失败文案（真值安全：报错，不假装无命中）。
    error: Option<String>,
    /// 打开前的视口快照（ESC 还原的正本——bug 本体：搜索后滚动位置丢失）。
    saved: Option<ScrollModel>,
}

impl SearchState {
    /// overlay 是否打开。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 当前查询词。
    pub fn query(&self) -> &str {
        &self.query
    }

    /// 当前查询的命中列表。
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// 当前命中下标（无命中时无意义，取 [`Self::current_hit`] 为 `None`）。
    pub fn current(&self) -> usize {
        self.current
    }

    /// 当前命中（渲染 excerpt 与跳转定位读同一项；无命中 = `None`）。
    pub fn current_hit(&self) -> Option<&Hit> {
        self.hits.get(self.current)
    }

    /// 是否已发生过跳转导航（渲染 `k/n` 计数的判据）。
    pub fn jumped(&self) -> bool {
        self.jumped
    }

    /// 查询变更待取数（事件循环的取数判据之一）。
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// 最近一次 RPC 失败文案（`None` = 无错误）。
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 打开前的视口快照（测试观察缝：开合还原的存取两点）。
    pub fn saved_viewport(&self) -> Option<ScrollModel> {
        self.saved
    }

    /// 状态行文案（渲染与测试同一份真值，按优先级）：RPC 失败 → 空查询
    /// （type to search，不冒充无命中）→ 待取数 → 无命中（带查询词）→
    /// 命中计数（触顶 `N+`，不假装全量）。
    pub fn status(&self) -> String {
        if let Some(err) = &self.error {
            return format!("search error: {err}");
        }
        if self.query.trim().is_empty() {
            return "type to search".to_string();
        }
        if self.dirty {
            return "searching...".to_string();
        }
        if self.hits.is_empty() {
            return format!("no matches for \"{}\"", self.query);
        }
        if self.capped {
            format!("{}+ matches", self.hits.len())
        } else {
            format!("{} matches", self.hits.len())
        }
    }

    /// 打开 overlay：快照当前视口（ESC 还原用）并复位查询/命中/错误——
    /// 每次打开都是干净起点（无搜索历史）。
    pub fn open(&mut self, viewport: ScrollModel) {
        *self = Self::default();
        self.open = true;
        self.saved = Some(viewport);
    }

    /// 关闭 overlay：交还打开前的视口快照并复位全部状态。返回快照由
    /// 调用方写回视口（`None` = 本来就没开过，不碰视口）。
    pub fn close(&mut self) -> Option<ScrollModel> {
        self.open = false;
        let saved = self.saved.take();
        *self = Self::default();
        saved
    }

    /// 视图整体更换（切会话 / 清屏）：命中行号全部失效，连同视口快照一并
    /// 丢弃——旧台的滚动位置没有意义，不走 [`Self::close`] 的还原语义
    /// （调用方随后 reset 视口）。
    pub fn forget(&mut self) {
        *self = Self::default();
    }

    /// 查询编辑（键入 / 退格）：清命中与错误、游标回顶、按新词置
    /// `dirty`；清空到空词直接撤掉取数（存储侧空查询是错误，不发）。
    pub fn edit(&mut self, apply: impl FnOnce(&mut String)) {
        apply(&mut self.query);
        self.hits.clear();
        self.current = 0;
        self.jumped = false;
        self.capped = false;
        self.error = None;
        self.dirty = !self.query.trim().is_empty();
    }

    /// 注入取数结果（事件循环的 RPC 回执）：游标回顶、撤 dirty。
    pub fn set_hits(&mut self, hits: Vec<Hit>, capped: bool) {
        self.hits = hits;
        self.capped = capped;
        self.current = 0;
        self.jumped = false;
        self.dirty = false;
        self.error = None;
    }

    /// 取数失败（真值安全：状态行报错；旧查询的命中在 [`Self::edit`] 已清，
    /// 不会拿旧结果冒充新查询的结果）。
    pub fn set_error(&mut self, message: String) {
        self.hits.clear();
        self.current = 0;
        self.jumped = false;
        self.capped = false;
        self.dirty = false;
        self.error = Some(message);
    }

    /// 导航一步：Enter 下一条 / Shift+Enter 上一条，**循环边界**（到尾回
    /// 头、到头回尾）。返回应跳转的命中下标；无命中 = `None`（视口不动，
    /// 状态行的 no matches 就是全部真相）。
    ///
    /// 尚未跳过（[`Self::jumped`] = false）时语义是「光标停在起点之前」：
    /// Enter 落到首条、Shift+Enter 循环到末条——与双向查找（vim `n` / `N`）
    /// 同口径。
    pub fn step(&mut self, back: bool) -> Option<usize> {
        let len = self.hits.len();
        if len == 0 {
            return None;
        }
        self.current = if !self.jumped {
            if back { len - 1 } else { 0 }
        } else if back {
            if self.current == 0 {
                len - 1
            } else {
                self.current - 1
            }
        } else {
            (self.current + 1) % len
        };
        self.jumped = true;
        Some(self.current)
    }
}
