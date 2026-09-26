//! inline.rs — T8：inline dock 原生 scrollback 轨（route §3 T8 设计注记 +
//! decisions D17 四裁；对齐 dh-rs `inline_screen.rs` 与 grok minimal 帧序）。
//!
//! 三层拆分（本模块内，全部可单测）：
//! - **账本**（[`Screen`] 内嵌行列/世代记账）：cols/rows + transcript 尾行
//!   （`row`/`col`）+ `generation`；dock 恒 [`DOCK_ROWS`] 行钉在屏底，
//!   一切写入先按当前几何落点。
//! - **序列构造器**（[`Screen::attach`] / [`Screen::take_frame`] /
//!   [`Screen::resize`] / [`Screen::detach`] / [`apply_frame`]）：手写 ANSI
//!   字节帧，测试直接断言输出字节；stdout 落盘只发生在事件循环薄接线层。
//! - **接线层**（[`run_inline`]）：raw mode + 事件循环（按键/resize/事件流），
//!   渲染投影复用 T1 的 [`crate::render_linear_line`]（零 ESC 净化由它保证）。
//!
//! 渲染模型（D17-1，写即定稿）：主屏 + CSI `r` 全屏滚动区 + 底部 dock；
//! transcript 区（dock 之上整片屏）内容越界时**推 `\n` 原生上滚**——
//! 已上滚进 scrollback 的行永不重写（本模块的写入点只有 transcript 尾行
//! 与 dock 两行，从不 CUP 回历史行），对症 #455「追加输出打花 scrollback」。
//! **禁** alternate screen（`?1049` 零命中）、**禁** ratatui `Viewport::Inline`、
//! **禁** 鼠标捕获（滚轮归终端原生，T7 不进本路径）。
//!
//! 帧序（grok minimal `draw`，`:47-108`）：每个 stdout 帧包一对 DEC 同步
//! 更新（`?2026` begin/end），帧内先落 transcript 增量（含回合落定的
//! commit 记账推进）再重绘 dock 两行；一切 resize 先 adopt 新几何再落笔
//! （D17-3）。半截写 → [`POISON_TEARDOWN_BYTES`]（ED2 清视口 + 复位）+
//! 非零退出，不做半屏苟活（D17-1 poison 口径，对齐 dh-rs teardown）。
//!
//! 交互归属（route §3 T8）：T6 翻页键（PgUp/PgDn/End/Ctrl+U）本档 no-op
//! （回看/搜索/复制归终端原生 scrollback，[`scrollback_key`] 过滤）；
//! T2–T5 编辑键语义经 [`crate::app::App`] 原样复用；enhanced/linear 路径
//! 与 T1–T7 现状一字不动。

use std::io::{self, Write};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use web_client::ClientError;
use web_client::daemon::WebDaemon;
use web_state::ui_state::{AgentEvent, UiState};

use crate::app::{App, KeyAction};
use crate::ui::footer;
use crate::{
    TuiError, TuiOptions, client_error, persist_assistant, push_user_message, render_linear_line,
    resolve_session,
};

/// dock 行数（route §3 T8 设计注记 ③ 定稿：单行 composer + 状态行，恒 2）。
pub const DOCK_ROWS: u16 = 2;

/// ESC 字节（十进制形态）。theme_lint（D11 字面量禁令）禁止 src 里出现
/// 十六进制数值与反斜杠转义形态的 ESC 序列——控制序列一律从数字/字节
/// 码位拼装，序列的可读文本只活在 tests/ 侧。
const ESC: u8 = 27;

/// CSI 引导（ESC + `[`）。
const CSI: [u8; 2] = [ESC, b'['];

/// DEC 同步更新 begin（grok minimal 帧序：commit + dock 重绘包一对）。
pub const SYNC_BEGIN: &[u8] = &[ESC, b'[', b'?', b'2', b'0', b'2', b'6', b'h'];

/// DEC 同步更新 end（退出序列的第一项也用它收尾）。
pub const SYNC_END: &[u8] = &[ESC, b'[', b'?', b'2', b'0', b'2', b'6', b'l'];

/// 进屏序列（dh-rs `stage_attach` `inline_screen.rs:161-199` 同序）：
/// 全屏滚动区 + 关 origin + bracketed paste + 藏光标。
#[rustfmt::skip]
pub const ATTACH_PRELUDE: &[u8] = &[
    ESC, b'[', b'r', // 复位滚动区
    ESC, b'[', b'?', b'6', b'l', // 关 origin
    ESC, b'[', b'?', b'2', b'0', b'0', b'4', b'h', // 开 bracketed paste
    ESC, b'[', b'?', b'2', b'5', b'l', // 藏光标
];

/// 半截写 / 无可锚定坐标的兜底还原（对齐 dh-rs `POISON_TEARDOWN_BYTES`，
/// `terminal.rs:17` 引用同款）：先收同步更新，再复位滚动区与 origin，
/// **ED2 清视口**（半截草稿不得进原生历史），关 paste、还光标、复位 SGR。
#[rustfmt::skip]
pub const POISON_TEARDOWN_BYTES: &[u8] = &[
    ESC, b'[', b'?', b'2', b'0', b'2', b'6', b'l', // 同步收尾
    ESC, b'[', b'r', // 复位滚动区
    ESC, b'[', b'?', b'6', b'l', // 关 origin
    ESC, b'[', b'2', b'J', // ED2 清视口（teardown 专用）
    ESC, b'[', b'H', // 回屏原点
    ESC, b'[', b'?', b'2', b'0', b'0', b'4', b'l', // 关 paste
    ESC, b'[', b'?', b'2', b'5', b'h', // 还光标
    ESC, b'[', b'0', b'm', // SGR 复位
];

/// EL0：清光标到行尾（只清未写字节区，绝不碰已落定的行内前缀）。
const EL_END: &[u8] = &[ESC, b'[', b'K'];

/// EL2：整行清空（新行落笔前的残迹清扫，等价 EL0——列 1 起）。
const EL_ROW: &[u8] = &[ESC, b'[', b'2', b'K'];

/// resize 阶段帧头：复位滚动区 + 关 origin + 藏光标（进屏序列去掉
/// bracketed paste——paste 总开关只随 attach/detach 走）。
const RESIZE_PRELUDE: &[u8] = &[
    ESC, b'[', b'r', ESC, b'[', b'?', b'6', b'l', ESC, b'[', b'?', b'2', b'5', b'l',
];

/// 还光标（退出序列 ②）。
const SHOW_CURSOR: &[u8] = &[ESC, b'[', b'?', b'2', b'5', b'h'];

/// 关 bracketed paste（退出序列 ③）。
const PASTE_OFF: &[u8] = &[ESC, b'[', b'?', b'2', b'0', b'0', b'4', b'l'];

/// 关 origin + 复位滚动区（退出序列 ④）。
const ORIGIN_OFF_RESET: &[u8] = &[ESC, b'[', b'?', b'6', b'l', ESC, b'[', b'r'];

/// SGR 复位（退出序列收尾）。
const SGR_RESET: &[u8] = &[ESC, b'[', b'0', b'm'];

/// 事件轮询间隔（同 enhanced `POLL` 档）。
const POLL: Duration = Duration::from_millis(50);

/// `stats.summary` 刷新间隔（状态行空闲段 run 计数的数据源节流）。
const STATS_SYNC: Duration = Duration::from_secs(2);

/// `stats.summary` 的统计窗口（半开 run 计数口径，同 enhanced）。
const STATS_RANGE: &str = "24h";

/// 退出/panic 时可锚定的坐标（transcript 尾行, 列, 屏高）。
/// panic hook 拿不到 [`Screen`]，靠这个静态锚点做「含 panic 的退出序列」
/// （route §3 T8：同步收尾 → 还光标 → 关 paste → 复位滚动区）。
static ANCHOR: Mutex<Option<(u16, u16, u16)>> = Mutex::new(None);

// --- 宽度启发（不引 unicode-width 依赖：裁决级判据即可，误差行由
// 行首 EL 残迹清扫自愈） ---

/// 单字符占格数（组合符 0、东亚宽 2、其余 1）。启发式：本 crate 不带
/// unicode-width 依赖（白名单外不动 Cargo.toml），按主要码位段裁决；
/// 误判最多造成一次行内自愈（[`EL_END`]/dock EL2 都在下一帧补扫）。
fn char_cells(c: char) -> u16 {
    let cp = c as u32;
    if matches!(
        cp,
        // 十进制码位段（theme_lint 禁十六进制字面量）：
        768..=879 // 0300..036F 组合附加符号
        | 6832..=6911 // 1AB0..1AFF
        | 7616..=7679 // 1DC0..1DFF
        | 8400..=8447 // 20D0..20FF
        | 8203..=8207 // 200B..200F 零宽空格/方向记号
        | 8288..=8292 // 2060..2064
        | 65024..=65039 // FE00..FE0F 变体选择符
        | 65056..=65071 // FE20..FE2F
    ) {
        return 0;
    }
    if matches!(
        cp,
        4352..=4447 // 1100..115F 谚文音节
        | 11904..=12350 // 2E80..303E CJK 部首/符号
        | 12353..=13311 // 3041..33FF
        | 13312..=19903 // 3400..4DBF
        | 19968..=40959 // 4E00..9FFF 统一表意文字
        | 40960..=42191 // A000..A4CF
        | 44032..=55203 // AC00..D7A3 谚文音节
        | 63744..=64255 // F900..FAFF
        | 65072..=65135 // FE30..FE6F
        | 65280..=65376 // FF00..FF60 全角
        | 65504..=65510 // FFE0..FFE6
        | 127744..=128591 // 1F300..1F64F
        | 129280..=129535 // 1F900..1F9FF
        | 131072..=262141 // 20000..3FFFD
    ) {
        return 2;
    }
    1
}

/// 按格裁剪到 `cols`（dock 两行的防溢出闸：屏底行写出即可能触发滚动，
/// 锚定必须由我们自己的字节控制，不许交给终端 autowrap 押注）。
fn clip_cells(s: &str, cols: u16) -> String {
    let mut out = String::new();
    let mut used = 0u16;
    for c in s.chars() {
        let w = char_cells(c);
        if used.saturating_add(w) > cols {
            break;
        }
        used = used.saturating_add(w);
        out.push(c);
    }
    out
}

/// 绝对寻址（CSI CUP，1 基）。
fn push_cup(b: &mut Vec<u8>, row: u16, col: u16) {
    b.extend_from_slice(&CSI);
    b.extend_from_slice(format!("{row};{col}H").as_bytes());
}

/// 清 `[from, to]` 整行（EL2）。
fn push_clear_rows(b: &mut Vec<u8>, from: u16, to: u16) {
    for row in from..=to {
        push_cup(b, row, 1);
        b.extend_from_slice(EL_ROW);
    }
}

/// 帧包一对 DEC 同步更新（防闪/防 resize 写花，grok minimal `draw` 帧序）。
fn sync_wrap(mut payload: Vec<u8>) -> Vec<u8> {
    let mut out = SYNC_BEGIN.to_vec();
    out.append(&mut payload);
    out.extend_from_slice(SYNC_END);
    out
}

// --- 账本 + 序列构造器 ---

/// 世代账本（dh-rs `Ledger` 口径的 inline 子集：几何 + 尾行 + 世代）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    /// 当前列数。
    pub cols: u16,
    /// 当前行数。
    pub rows: u16,
    /// transcript 当前尾行（1 基；可短暂 = height+1，等待下次落笔时上滚）。
    pub row: u16,
    /// 尾行下一落笔列（1 基；`cols+1` = 行满、待换行）。
    pub col: u16,
    /// 世代号：attach = 1，回合落定 commit 与 resize 各 ++（D17-4 记账）。
    pub generation: u64,
}

/// inline 屏面：账本 + 未上屏字节（staged）+ dock 上次内容（增量重绘判定）。
///
/// 实现 [`Write`]：事件投影 [`crate::render_linear_line`] 的净化文本直接
/// 流进本结构，按宽度切显示行、按需推原生上滚——**已上滚行零重绘**。
#[derive(Debug)]
pub struct Screen {
    /// 世代账本。
    ledger: Ledger,
    /// 本帧待上屏字节（`take_frame` 收走并包同步更新对）。
    staged: Vec<u8>,
    /// `staged` 内是否已把光标定位到 (row, col)（跨帧必须重新 CUP）。
    positioned: bool,
    /// dock 上次渲染的 (composer, status)（未变则不重绘，帧可整体跳过）。
    dock: Option<(String, String)>,
    /// 半截写/不可恢复（poison 后退出走 [`POISON_TEARDOWN_BYTES`]，
    /// 不再尝试干净 detach）。
    poisoned: bool,
}

impl Screen {
    /// 拿一个未 attach 的屏面（generation 0；attach 后置 1）。
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            ledger: Ledger {
                cols,
                rows,
                row: rows.saturating_sub(DOCK_ROWS),
                col: 1,
                generation: 0,
            },
            staged: Vec::new(),
            positioned: false,
            dock: None,
            poisoned: false,
        }
    }

    /// 当前账本（测试/诊断观察缝）。
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// (cols, rows)。
    pub fn size(&self) -> (u16, u16) {
        (self.ledger.cols, self.ledger.rows)
    }

    /// 世代号（attach=1；回合落定 commit / resize 各 ++）。
    pub fn generation(&self) -> u64 {
        self.ledger.generation
    }

    /// 是否已 poison。
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }

    /// 标记 poison（写失败/几何不可用时由接线层调用）。
    pub fn mark_poisoned(&mut self) {
        self.poisoned = true;
    }

    /// 是否有未上屏的 transcript 增量。
    pub fn has_pending(&self) -> bool {
        !self.staged.is_empty()
    }

    /// transcript 可用高度（dock 之上的最后一行）。
    fn height(&self) -> u16 {
        self.ledger.rows.saturating_sub(DOCK_ROWS)
    }

    /// 进屏帧（route §3 T8 注记 ②，dh-rs `stage_attach` 同构）：
    /// 预置序列 → CUP 屏底 → `DOCK_ROWS+1` 个 `\r\n` 把旧屏推入原生
    /// scrollback → 渲染 dock；账本钉到 (height, 1)、generation=1。
    pub fn attach(&mut self, composer: &str, status: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(ATTACH_PRELUDE);
        push_cup(&mut b, self.ledger.rows, 1);
        for _ in 0..=DOCK_ROWS {
            b.extend_from_slice(b"\r\n");
        }
        self.ledger.row = self.height();
        self.ledger.col = 1;
        self.ledger.generation = 1;
        self.positioned = false;
        self.push_dock(&mut b, composer, status);
        self.refresh_anchor();
        sync_wrap(b)
    }

    /// 收一帧：transcript 增量（若有）+ dock 两行重绘，包同步更新对。
    /// 两者都没变 → `None`（一帧都不写，光标与屏幕保持不动）。
    pub fn take_frame(&mut self, composer: &str, status: &str) -> Option<Vec<u8>> {
        let dock_unchanged = self
            .dock
            .as_ref()
            .is_some_and(|(c, s)| c == composer && s == status);
        if self.staged.is_empty() && dock_unchanged {
            return None;
        }
        let mut b = std::mem::take(&mut self.staged);
        self.push_dock(&mut b, composer, status);
        // 上屏后光标停在 dock：下一笔 transcript 必须重新 CUP。
        self.positioned = false;
        self.refresh_anchor();
        Some(sync_wrap(b))
    }

    /// resize 帧（D17-3 锚定：按新几何重算账本、dock 重新钉新屏底、
    /// **已入 scrollback 行零触碰**；对齐 dh-rs `stage_resize`）。
    ///
    /// 尚未上屏的增量先按旧几何落盘（时序正确），随后：复位滚动区 →
    /// 清旧 dock 行（新界内）→ CUP 新屏底 + `DOCK_ROWS+1` 个 `\r\n`
    /// 建立全新边界 → dock 重绘。尾行/列重置、generation++。
    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        composer: &str,
        status: &str,
    ) -> Result<Vec<u8>, TuiError> {
        if rows <= DOCK_ROWS || cols == 0 {
            return Err(TuiError::Io(io::Error::other(
                "terminal too small for inline dock",
            )));
        }
        let mut b = std::mem::take(&mut self.staged); // 旧几何增量先按时序落盘
        b.extend_from_slice(RESIZE_PRELUDE);
        let old_dock_start = self.ledger.rows.saturating_sub(DOCK_ROWS).saturating_add(1);
        let old_dock_end = self.ledger.rows.min(rows);
        push_clear_rows(&mut b, old_dock_start, old_dock_end);
        push_cup(&mut b, rows, 1);
        for _ in 0..=DOCK_ROWS {
            b.extend_from_slice(b"\r\n");
        }
        // adopt 新几何（一切后续写入先按此落笔）。
        self.ledger.cols = cols;
        self.ledger.rows = rows;
        self.ledger.row = rows - DOCK_ROWS;
        self.ledger.col = 1;
        self.ledger.generation += 1;
        self.positioned = false;
        self.push_dock(&mut b, composer, status);
        self.refresh_anchor();
        Ok(sync_wrap(b))
    }

    /// 退出帧（route §3 T8 退出序列，dh-rs `stage_detach` 同构）：
    /// 同步更新收尾 → 清 dock 两行 + 光标停到 transcript 尾之后
    /// （退出后 transcript 完整留在 scrollback，shell 提示符接在其下）→
    /// 恢复光标 → 关 bracketed paste → 复位滚动区。
    pub fn detach(&self) -> Vec<u8> {
        detach_bytes(Some(self.anchor()))
    }

    /// 一折事件进 transcript 投影（T1 同一渲染器，净化由它保证）；
    /// `TurnEnd` = 回合落定 → generation++（D17-4 记账收尾，不重排历史）。
    pub fn feed(&mut self, ev: &AgentEvent, proj: &mut UiState) {
        let settle = matches!(ev, AgentEvent::TurnEnd { .. });
        render_linear_line(ev, proj, self).expect("screen sink is an in-memory buffer");
        if settle {
            self.ledger.generation += 1;
        }
    }

    /// 把一条用户输入写进 transcript（`❯ ` 前缀，同 enhanced 用户气泡
    /// 的行首标记；raw mode 无终端回显，必须由我们落笔）。
    pub fn write_user_line(&mut self, text: &str) {
        if self.ledger.col > 1 {
            self.put_newline();
        }
        self.put_text(&format!("❯ {text}"));
        self.put_newline();
    }

    // --- dock / 上滚原语 ---

    /// dock 两行：先 EL2 清行再写（只动 dock，不碰 transcript）。
    fn push_dock(&mut self, b: &mut Vec<u8>, composer: &str, status: &str) {
        let h = self.height();
        push_cup(b, h + 1, 1);
        b.extend_from_slice(EL_ROW);
        b.extend_from_slice(clip_cells(composer, self.ledger.cols).as_bytes());
        push_cup(b, self.ledger.rows, 1);
        b.extend_from_slice(EL_ROW);
        b.extend_from_slice(clip_cells(status, self.ledger.cols).as_bytes());
        self.dock = Some((composer.to_string(), status.to_string()));
    }

    /// 越界补给（dh-rs `reserve_dock_space` `:686-697` 的滚动先行变体）：
    /// 尾行越过 transcript 底时，先清 dock 区（防旧 dock 文本随上滚混进
    /// transcript），再推 `scrolls` 个 `\n` 原生上滚、账本回钉到底行。
    /// **上滚进 scrollback 的行从此只读**——本结构再无任何寻址可及于它。
    fn ensure_visible(&mut self) {
        let h = self.height();
        if self.ledger.row > h {
            let scrolls = self.ledger.row - h;
            push_clear_rows(&mut self.staged, h + 1, self.ledger.rows);
            push_cup(&mut self.staged, self.ledger.rows, 1);
            for _ in 0..scrolls {
                self.staged.push(b'\n');
            }
            self.ledger.row = h;
            self.positioned = false;
        }
    }

    /// 换行推进：尾行 +1（lazy：真正落笔前才补上滚，避免空转滚动把
    /// dock 残迹暴露在没有新内容覆盖的行上）。
    fn put_newline(&mut self) {
        self.ledger.row = self.ledger.row.saturating_add(1);
        self.ledger.col = 1;
        self.positioned = false;
    }

    /// 行内/行间落笔一批净化文本（制表符扩到下一站、C0 控制符滤除、
    /// 按显示宽切行——行满即换行，绝不依赖终端 autowrap 押注）。
    fn put_text(&mut self, s: &str) {
        for c in s.chars() {
            if c == '\t' {
                // 终端制表位：每 8 格一站；扩成空格保持列账本与屏面一致。
                let stop = 8 - ((self.ledger.col - 1) % 8);
                for _ in 0..stop {
                    self.put_char(' ');
                }
                continue;
            }
            self.put_char(c);
        }
    }

    /// 落一个字符（写即定稿的最小单元）。
    fn put_char(&mut self, c: char) {
        if c.is_ascii_control() {
            return; // ESC/回车等不可信字节绝不进屏（线性渲染同款净化）
        }
        let w = char_cells(c);
        // 行满/放不下先换行、**再**绝对寻址（次序不可反）：pending-wrap
        // 跨帧时 ledger.col = cols+1，没有合法列可 CUP——终端会把越界 CUP
        // 钳到末列，紧随的 EL0 就会把末格已定稿的字擦掉（#455 同类打花）。
        if self.ledger.col > self.ledger.cols
            || (w > 0 && self.ledger.col > 1 && self.ledger.col + w > self.ledger.cols + 1)
        {
            // 行满换行：绝对寻址把终端 pending-wrap 归一化（dh-rs
            // wrap_seal 要解的同一歧义，我们用每行 CUP 直接消掉）。
            self.row_advance();
        }
        if !self.positioned {
            // 跨帧首笔：先补上滚再绝对寻址（同步更新对内原子呈现）。
            self.ensure_visible();
            let (row, col) = (self.ledger.row, self.ledger.col);
            push_cup(&mut self.staged, row, col);
            self.staged.extend_from_slice(EL_END);
            self.positioned = true;
        }
        let mut buf = [0u8; 4];
        self.staged
            .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        self.ledger.col = self.ledger.col.saturating_add(w);
    }

    /// 行满/溢出换行：尾行 +1 → 必要时原生上滚 → 寻址新行行首 + EL。
    fn row_advance(&mut self) {
        self.ledger.row = self.ledger.row.saturating_add(1);
        self.ensure_visible();
        push_cup(&mut self.staged, self.ledger.row, 1);
        self.staged.extend_from_slice(EL_END);
        self.ledger.col = 1;
        self.positioned = true;
    }

    /// 刷新 panic/退出锚点（attach / 每帧上屏 / resize 后）。
    fn refresh_anchor(&self) {
        *ANCHOR.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.anchor());
    }

    /// 可锚定坐标（transcript 尾行, 列, 屏高）。列夹到 `[1, cols]`：
    /// pending-wrap 落在 `cols+1` 时没有合法列可寻（CUP 会被终端钳到
    /// 末列），退出/panic 停靠一律按末列算，绝不发出越界 CUP。
    fn anchor(&self) -> (u16, u16, u16) {
        let col = self.ledger.col.clamp(1, self.ledger.cols.max(1));
        (self.ledger.row, col, self.ledger.rows)
    }
}

impl Write for Screen {
    /// 把净化文本流进账本：`\n` 收行，其余按宽度落笔（不落任何字节到
    /// stdout——落盘只在接线层，保证「一帧一写」的原子性）。
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        for part in text.split_inclusive('\n') {
            match part.strip_suffix('\n') {
                Some(line) => {
                    self.put_text(line);
                    self.put_newline();
                }
                None => self.put_text(part),
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 退出/panic 还原序列（route §3 T8 退出序列正本；`anchor = None` 时
/// 只做模式复位——attach 之前的 panic 无可锚定坐标）。
pub fn detach_bytes(anchor: Option<(u16, u16, u16)>) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(SYNC_END); // ① DEC 同步更新收尾
    if let Some((row, col, rows)) = anchor {
        let h = rows.saturating_sub(DOCK_ROWS);
        // 清 dock 两行（transcript 行零触碰），光标停到 transcript 尾之后。
        push_clear_rows(&mut b, h.saturating_add(1), rows);
        let park_row = row.min(h.saturating_add(1));
        let park_col = if park_row < row { 1 } else { col.max(1) };
        push_cup(&mut b, park_row, park_col);
        b.extend_from_slice(b"\r\n");
    }
    b.extend_from_slice(SHOW_CURSOR); // ② 恢复光标
    b.extend_from_slice(PASTE_OFF); // ③ 关 bracketed paste
    b.extend_from_slice(ORIGIN_OFF_RESET); // ④ 关 origin + 复位滚动区
    b.extend_from_slice(SGR_RESET);
    b
}

/// 落盘一帧；**半截写 → poison**：无论成败都补写
/// [`POISON_TEARDOWN_BYTES`]（ED2 清视口，半截草稿不得进原生历史），
/// 再把首个写错误原样抛回（CLI 映射非零退出码）。
pub fn apply_frame(out: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let mut res = out.write_all(bytes);
    if res.is_ok() {
        res = out.flush();
    }
    if let Err(err) = res {
        let _ = out.write_all(POISON_TEARDOWN_BYTES);
        let _ = out.flush();
        return Err(err);
    }
    Ok(())
}

/// 落盘一帧并把写失败记成 screen poison（接线层专用）。
fn apply_to(screen: &mut Screen, out: &mut impl Write, bytes: &[u8]) -> Result<(), TuiError> {
    match apply_frame(out, bytes) {
        Ok(()) => Ok(()),
        Err(err) => {
            screen.mark_poisoned();
            Err(TuiError::Io(err))
        }
    }
}

// --- 行构造器（T5 口径复用） ---

/// dock 第 1 行：单行 composer（与 `ui/composer` 同款「提示符 + 末尾
/// 可视窗口」语义，另加按格裁剪防屏底 autowrap 滚动）。
pub fn composer_line(input: &str, cols: u16) -> String {
    const PROMPT: &str = "> ";
    let visible = usize::from(cols)
        .saturating_sub(PROMPT.chars().count())
        .max(1);
    let window: String = input
        .chars()
        .rev()
        .take(visible)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    clip_cells(&format!("{PROMPT}{window}"), cols)
}

/// dock 第 2 行：状态行 = **T5 footer 字段口径**（[`footer::line`] 原样
/// 摊平：model · 耗时/运行状态等已核字段——真值安全禁则继续生效，无数据源
/// 的 tokens/cost/context 不编造）+ 排队态标记（有队首 prompt 时追加）。
pub fn status_line(app: &App) -> String {
    let line = footer::line(app);
    let mut text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    if app.queued().is_some() {
        text.push_str(" · queued");
    }
    text
}

/// T6 翻页键判定（inline 档 no-op 过滤器）：PgUp/PgDn/End/Ctrl+U 回看
/// 归终端原生 scrollback（route §3 T8 交互归属，抄 dh-rs Focus 态口径）。
pub fn scrollback_key(key: &KeyEvent) -> bool {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    matches!(key.code, KeyCode::PageUp | KeyCode::PageDown | KeyCode::End)
        || (ctrl && key.code == KeyCode::Char('u'))
}

/// bracketed paste 折进 composer：取第一行（单行 composer 定稿，route
/// §1 多行输入 Deferred）、滤控制字符；换行之后的内容丢弃（粘贴即输入，
/// 不隐式提交——Enter 语义仍只归物理回车）。
pub fn feed_paste(app: &mut App, text: &str) {
    let first = text.split(['\n', '\r']).next().unwrap_or("");
    for c in first.chars().filter(|c| !c.is_control()) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

// --- 接线层 ---

/// panic hook（含 panic 的退出序列）：先按静态锚点做干净 detach
/// （同步收尾 → 清 dock + 还光标 → 关 paste → 复位滚动区；无 alt-screen
/// 可退）+ disable raw，再落 `$TMPDIR` 崩溃报告（D15 报告口径复用）。
/// 与 poison 分野：panic 时屏面自洽 → transcript 完整留 scrollback；
/// 半截写才用 ED2 清视口。
pub fn install_inline_panic_hook() {
    let hook = crate::termguard::build_hook(
        || {
            let anchor = *ANCHOR.lock().unwrap_or_else(|e| e.into_inner());
            let _ = crossterm::terminal::disable_raw_mode();
            let mut out = io::stdout();
            let _ = out.write_all(&detach_bytes(anchor));
            let _ = out.flush();
        },
        |info| {
            let _ = crate::termguard::write_crash_report(info);
        },
    );
    std::panic::set_hook(hook);
}

/// inline 档入口（route §3 T8 第四臂）。
///
/// 顺序：解析会话（错误先于一切终端改动，退出码 2/3 语义同 T1）→ 装
/// panic hook → 取尺寸 → raw mode（本档不进 alt-screen、不启鼠标捕获）
/// → 事件循环 → **退出序列**（poison 过则跳过干净 detach，teardown 已由
/// [`apply_frame`] 补写）→ disable raw。错误合并口径同 enhanced
/// （业务 Err 优先于还原 Err）。
pub fn run_inline(
    opts: TuiOptions,
    client: WebDaemon,
    rx: Receiver<AgentEvent>,
) -> Result<(), TuiError> {
    let sid = resolve_session(&client, &opts)?;
    install_inline_panic_hook();
    let (cols, rows) = crossterm::terminal::size().map_err(TuiError::Io)?;
    crossterm::terminal::enable_raw_mode().map_err(TuiError::Io)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut screen = Screen::new(cols, rows);
    let outcome = drive(&client, &sid, &rx, &mut screen, &mut out);
    let detach = if screen.poisoned() {
        Ok(())
    } else {
        apply_frame(&mut out, &screen.detach())
    };
    let _ = crossterm::terminal::disable_raw_mode();
    match (outcome, detach) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(TuiError::Io(err)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// 事件循环（薄接线：状态机全在 [`Screen`] / [`App`]，这里只搬运）。
///
/// 每轮：收帧上屏 → 按键（T6 翻页键过滤）/ resize（先 adopt 再落笔）/
/// paste → 出站 prompt（先落库再订阅后 prompt 同序，T1 边界）→ 事件流
/// 投影（`TurnEnd` = commit + persist）→ prompt RPC 结果 → stats 轮询。
fn drive(
    client: &WebDaemon,
    sid: &str,
    rx: &Receiver<AgentEvent>,
    screen: &mut Screen,
    out: &mut impl Write,
) -> Result<(), TuiError> {
    let mut app = App::new();
    app.start_session(sid, Vec::new());
    app.set_model(footer::configured_model());
    if let Ok(summary) = client.stats_summary(STATS_RANGE) {
        app.set_in_flight(summary.in_flight_runs);
    }
    // transcript 投影（render_linear_line 的状态源；persist 也读它）。
    let mut proj = UiState::default();
    let (ptx, prx) = mpsc::channel::<Result<(), ClientError>>();
    let mut last_stats_sync = Instant::now();
    // 进屏（首帧：旧屏推入 scrollback + dock 落位）。
    let frame = screen.attach(
        &composer_line(app.input(), screen.size().0),
        &status_line(&app),
    );
    apply_to(screen, out, &frame)?;
    loop {
        // 收帧：transcript 增量 + dock 重绘，包同步更新对（无变化 = 0 字节）。
        let composer = composer_line(app.input(), screen.size().0);
        let status = status_line(&app);
        if let Some(frame) = screen.take_frame(&composer, &status) {
            apply_to(screen, out, &frame)?;
        }
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) => {
                    if scrollback_key(&key) {
                        // T6 翻页键 inline no-op：回看/搜索/复制归终端。
                    } else {
                        match app.handle_key(key) {
                            KeyAction::Quit => break,
                            KeyAction::Abort => {
                                client.abort_worker().map_err(client_error)?;
                            }
                            KeyAction::None => {}
                        }
                    }
                }
                Event::Resize(cols, rows) => {
                    // D17-3：先 adopt 当前尺寸再落笔（resize 帧内已含
                    // 旧几何 pending 的时序正确落盘，见 `Screen::resize`）。
                    let composer = composer_line(app.input(), screen.size().0);
                    let status = status_line(&app);
                    match screen.resize(cols, rows, &composer, &status) {
                        Ok(frame) => apply_to(screen, out, &frame)?,
                        Err(err) => {
                            screen.mark_poisoned();
                            let _ = apply_frame(out, POISON_TEARDOWN_BYTES);
                            return Err(err);
                        }
                    }
                }
                Event::Paste(text) => feed_paste(&mut app, &text),
                // 焦点/其余事件不改状态；鼠标事件收不到（本档不启捕获）。
                _ => {}
            }
        }
        // 出站：user 消息先落库（T1 同序）→ transcript 写用户行 → 起
        // prompt 线程（RPC 结果经 ptx/prx 回收，失败即退出码语义）。
        if let Some(delivery) = app.take_prompt() {
            if let Some(last) = app.messages().last() {
                proj.push_message(last.clone()); // 与 App 的 user 落点同步
            }
            screen.write_user_line(&delivery.text);
            push_user_message(client, &delivery.session_id, &delivery.text)?;
            spawn_prompt(
                client,
                &delivery.session_id,
                &delivery.run_id,
                delivery.text,
                &ptx,
            )?;
        }
        // 会话级事件流：投影 → TurnEnd = 回合落定（generation++ 于
        // `Screen::feed` 内）+ assistant 落库 + dock 状态收尾（下一帧上屏）。
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    let settle = matches!(ev, AgentEvent::TurnEnd { .. });
                    screen.feed(&ev, &mut proj);
                    if settle {
                        persist_assistant(client, sid, &proj)?;
                        app.note_turn_end();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(TuiError::DaemonUnreachable(
                        "daemon unreachable: event stream closed".to_string(),
                    ));
                }
            }
        }
        while let Ok(res) = prx.try_recv() {
            res.map_err(client_error)?;
        }
        if last_stats_sync.elapsed() >= STATS_SYNC {
            last_stats_sync = Instant::now();
            if let Ok(summary) = client.stats_summary(STATS_RANGE) {
                app.set_in_flight(summary.in_flight_runs);
            }
        }
    }
    // 退出前冲掉未上屏的 transcript 增量（只补写新内容，历史零重绘）。
    let composer = composer_line(app.input(), screen.size().0);
    let status = status_line(&app);
    if let Some(frame) = screen.take_frame(&composer, &status) {
        apply_to(screen, out, &frame)?;
    }
    Ok(())
}

/// 起 prompt 线程（与 enhanced `spawn_prompt` 同构：RPC 结果回事件循环，
/// 断线/结构化错误按 T1 映射退出码；app.rs 是 T6/T7 领地，此处独立实现）。
fn spawn_prompt(
    client: &WebDaemon,
    sid: &str,
    run_id: &str,
    msg: String,
    tx: &mpsc::Sender<Result<(), ClientError>>,
) -> Result<(), TuiError> {
    let client = client.clone();
    let sid = sid.to_string();
    let run_id = run_id.to_string();
    let tx = tx.clone();
    std::thread::Builder::new()
        .name("oi-tui-inline-prompt".into())
        .spawn(move || {
            let res = client.worker_prompt_run(&sid, &run_id, &msg).map(|_| ());
            let _ = tx.send(res);
        })?;
    Ok(())
}
