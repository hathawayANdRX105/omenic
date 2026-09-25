//! scroll — transcript 视口滚动模型（route §3 T6：翻页 / 脱钩跟尾 / 追尾滑动）。
//!
//! 滚动的是**渲染行**（行数随列宽断行变化），不是消息条数——与
//! `App::scroll`（T4 的回填历史滚动，route §4 `history_scroll_bounds_clamp`）
//! 是两套状态，互不干扰。模型抄 dsh-rs 的最小两字段（refs
//! `dsh-rs/src/cli/tui.rs:184-185` 的 `scroll` + `auto_follow`）：
//! [`ScrollModel::offset`]（视口首行，自顶）+ [`ScrollModel::follow`]（跟尾），
//! 几何缓存（`height` / `total`）每帧由 [`ScrollModel::sync`] 喂入，**clamp
//! 单点**（refs `tui.rs:412-416`：跟尾取 max、脱钩取 min）也收敛在 `sync`
//! 与 [`ScrollModel::view_top`] 两处——渲染只读模型，不自己发明边界。
//!
//! 追尾滑动抄 jcode（refs `jcode-tui/src/tui/ui_viewport.rs:65-114` 的
//! `resolve_tail_follow_scroll`）：大块追加不整屏蹦——追加量（含既有滞后）
//! ≤4 行直接 snap，否则每帧最多前进 3 行滑入，滞后封顶 1 个视口。翻页语义
//! 抄 grok（refs `xai-grok-pager/.../nav.rs:429` 的 `page_up`：视口顶上移
//! 一页；步长 = 视口高 − 1 行，route §3 T6 定死）。

/// 追尾滑动的 snap 阈值：追加量（含既有滞后）不超过该值直接贴底
/// （jcode `TAIL_CATCHUP_MIN_JUMP`，refs `ui_viewport.rs:68`）。
const SNAP_JUMP: usize = 4;

/// 追尾滑动每帧最多前进的行数（jcode `TAIL_CATCHUP_MAX_STEP`，refs
/// `ui_viewport.rs:73`）。
const SLIDE_STEP: usize = 3;

/// transcript 视口滚动状态（无 IO、可直接单测；route §3 T6 契约种子）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollModel {
    /// 视口首行（自顶 0-based；`follow` 时由 [`Self::sync`] 的追尾滑动推进）。
    offset: usize,
    /// 跟尾：`true` = 视口追内容底（流式追加把视口拽回底）；`false` = 用户
    /// 已脱钩（上滚即脱钩，流式只被 clamp、不拽回）。
    follow: bool,
    /// 视口行数（每帧 [`Self::sync`] 按几何同步；resize 随之更新）。
    height: usize,
    /// 内容总行数（每帧 [`Self::sync`] 重数；随列宽断行与流式追加变化）。
    total: usize,
    /// 是否完成过 [`Self::sync`]：未同步前 `follow` 视图钉底——让「没喂过
    /// 几何的首帧」与 tail-only 渲染等价（不显示内容顶部）。
    synced: bool,
}

impl ScrollModel {
    /// 初始态：跟尾、未同步（首帧即钉底，等价 T6 前的 tail-only）。
    pub fn new() -> Self {
        Self {
            offset: 0,
            follow: true,
            height: 0,
            total: 0,
            synced: false,
        }
    }

    /// 每帧几何同步（`ui::sync_viewport` 调用）：更新 `total`/`height` 并
    /// 解析本帧显示位——跟尾走追尾滑动，脱钩只做单点 clamp（内容收缩 /
    /// resize 后不越界；内容增长时 offset 原地不动 = 流式不拽回）。
    pub fn sync(&mut self, total: usize, height: usize) {
        self.total = total;
        self.height = height;
        self.synced = true;
        let max = self.max_offset();
        if self.follow {
            self.offset = resolve_tail_follow(max, height, self.offset);
        } else {
            self.offset = self.offset.min(max);
        }
    }

    /// 上滚一页（PgUp）：脱钩 + 视口顶上移「视口高 − 1 行」（grok `page_up`
    /// 的顶对齐语义；脱钩即刻生效，动画中的追尾位也从此停住 = 重新脱钩）。
    pub fn page_up(&mut self) {
        self.follow = false;
        self.offset = self.offset.saturating_sub(self.page_rows());
    }

    /// 下翻一页（PgDn）：视口顶下移一页、clamp 到底；**落到底即恢复跟尾**
    /// （route §3「PgDn 到底 = 恢复跟随」；dsh-rs `PageDown` 同口径，refs
    /// `tui.rs:664-670`——saturating 到 0 即 `auto_follow = true`）。
    pub fn page_down(&mut self) {
        let max = self.max_offset();
        self.offset = self.offset.saturating_add(self.page_rows()).min(max);
        if self.offset == max {
            self.follow = true;
        }
    }

    /// 上滚半页（Ctrl+U 半页键，route §3 T6 二选一定 Ctrl+U）：脱钩 + 视口
    /// 顶上移半视口（步长 = 视口高 / 2，最少 1 行；route 只钉了整页步长，
    /// 半页公式由本任务拍板，见交付报告）。
    pub fn half_up(&mut self) {
        self.follow = false;
        self.offset = self.offset.saturating_sub(self.half_rows());
    }

    /// 跳到底（End）：单点 clamp 到 max + 恢复跟尾（route §3「End = 恢复
    /// 跟随」）。
    pub fn to_end(&mut self) {
        self.offset = self.max_offset();
        self.follow = true;
    }

    /// 复位到初始态（切会话回填：视口回底、重新钉尾）。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// 视口首行（自顶）。
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// 是否跟尾（`false` = 用户已脱钩，`↑N 行` 指示随之出现）。
    pub fn is_following(&self) -> bool {
        self.follow
    }

    /// 视口行数（最近一次 [`Self::sync`] 的几何）。
    pub fn height(&self) -> usize {
        self.height
    }

    /// 内容总行数（最近一次 [`Self::sync`] 按当时列宽数出的行数）。
    pub fn total(&self) -> usize {
        self.total
    }

    /// 视口首行的最大值（单点 clamp 的 max，dsh-rs `max_scroll` 同式：
    /// `len - visible`，refs `tui.rs:412`）。
    pub fn max_offset(&self) -> usize {
        self.total.saturating_sub(self.height)
    }

    /// `↑N 行` 指示（route §3 T6）：跟尾 = `None`（指示不出现）；脱钩 =
    /// `Some(max - offset)`——距内容底的**精确**行数，回底（End / PgDn 落底
    /// 重挂）后随之消失。
    pub fn lift(&self) -> Option<usize> {
        if self.follow {
            None
        } else {
            Some(self.max_offset().saturating_sub(self.offset))
        }
    }

    /// 本帧应渲染的视口首行（渲染侧的防御性单点 clamp）：未同步的跟尾态
    /// 直接钉底（首帧 = tail-only）；其余取 `offset` 夹进给定几何的 max
    /// ——resize 后即使 `sync` 未及执行也不越界（route §3「resize 后 clamp」）。
    pub fn view_top(&self, total: usize, height: usize) -> usize {
        let max = total.saturating_sub(height);
        if self.follow && !self.synced {
            return max;
        }
        self.offset.min(max)
    }

    /// 整页步长：视口高 − 1 行（route §3 T6 定值；视口只有 1 行时保底 1，
    /// 翻页不许原地卡死）。
    fn page_rows(&self) -> usize {
        self.height.saturating_sub(1).max(1)
    }

    /// 半页步长：视口高 / 2，最少 1 行。
    fn half_rows(&self) -> usize {
        (self.height / 2).max(1)
    }
}

impl Default for ScrollModel {
    fn default() -> Self {
        Self::new()
    }
}

/// 追尾滑动（jcode `resolve_tail_follow_scroll` 逐条对齐，refs
/// `ui_viewport.rs:65-114`；三语义的测试名也照抄进 `tests/scroll_follow.rs`）：
///
/// - **首帧 / 内容收缩**（`prev == 0` 或 `max <= prev`）：直接 snap；
/// - **小追加**（`jump <= 4`）：直接 snap（流式小步不整屏蹦）；
/// - **大块追加**：每帧最多前进 [`SLIDE_STEP`] 行滑入，且滞后
///   （`max - 位`）封顶 1 个视口（巨大块不重播整屏）。
fn resolve_tail_follow(max_scroll: usize, height: usize, prev: usize) -> usize {
    if prev == 0 || max_scroll <= prev {
        return max_scroll;
    }
    let jump = max_scroll - prev;
    if jump <= SNAP_JUMP {
        return max_scroll;
    }
    // 滞后上限 = max(视口, snap 阈值)（jcode `max_lag` 同式）。
    let floor = max_scroll.saturating_sub(height.max(SNAP_JUMP));
    prev.max(floor).saturating_add(SLIDE_STEP).min(max_scroll)
}
