//! notify — T15 完成通知：批次状态机 + BEL / OSC9 写出字节（route §3 T15）。
//!
//! 批次模型（任务书 §3 裁决）：**批次 = 出站队列排空段**——首个出站派发
//! 起计时（[`BatchNotify::note_dispatch`]），到**队列空的最终 `TurnEnd`**
//! 止（[`BatchNotify::note_turn_end`]）。通知点唯一 = 批次完成：阈值与抑制
//! 的判定只发生在收口这一处，**不存在逐 turn / 逐 chunk 通知路径**（bug 本体，
//! `tests/turn_notify.rs` 钉住）。批次进行中的中间 `TurnEnd` 只判队列是否
//! 排空（非空 = T10 还会逐放后续 prompt，批次继续，不通知）。
//!
//! 无 IO：状态机只产出字节，写 stdout 归接线层（事件循环 / linear 循环，
//! 同 T12 copy 的分工）。时钟以参数注入（T7 `wheel_tick(now)` 同款范式），
//! 测试不真睡阈值。

use std::time::{Duration, Instant};

use crate::app::{OSC_BEL, OSC_ESC};

/// 批次总时长阈值：完成时 ≥ 该值才响（首版常量不进配置——阈值用户可配置
/// 化属任务书 §8 禁止项，配置面只有 OSC9 开关）。
pub const BATCH_NOTIFY_MIN: Duration = Duration::from_secs(10);

/// OSC9 通知文案（`ESC ] 9 ; <msg> BEL` 的载荷）：只陈述「回合批次已结束」，
/// 不编造内容。
const OSC9_MSG: &str = "oi tui: turn finished";

/// T15：完成通知批次状态机（无 IO、时钟注入；`App` 持有，linear 路径直接
/// 持有本类型）。
#[derive(Default)]
pub struct BatchNotify {
    /// 批次起点 = 首个出站派发时刻；`None` = 无进行中批次（非批次 `TurnEnd`
    /// 到收口处直接空手而归）。
    started: Option<Instant>,
    /// 本批次已被输入抑制；新批次首个派发时重新武装（任务书 §3 裁决 4）。
    suppressed: bool,
    /// 批次完成待写出的通知字节（接线层 `take` 后旁路写 stdout）。
    pending: Option<Vec<u8>>,
    /// 累计响铃次数（「恰一次 per 批次」的测试观察缝，任务书 §4 断言口径）。
    bells: u64,
    /// OSC9 开关（启动读定不热载；默认关，配置读不到按默认关）。
    osc9: bool,
}

impl BatchNotify {
    /// 空批次状态（事件循环与测试的同一入口）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 首个出站派发 = 批次计时起点。批次进行中（T10 逐放的后续 prompt）
    /// 不重置计时与抑制标志——批次是一个连续排空段，不是一个 turn。
    pub fn note_dispatch(&mut self, now: Instant) {
        if self.started.is_none() {
            self.started = Some(now);
            self.suppressed = false;
        }
    }

    /// 批次进行中收到 composer 可打印输入 → 本批次通知抑制（jcode 失焦
    /// 语义的终端版裁决：收到输入即抑制——用户就在终端前，无需提醒）。
    /// 无进行中批次时不记账（批次前的输入不预埋抑制）。
    pub fn note_input(&mut self) {
        if self.started.is_some() {
            self.suppressed = true;
        }
    }

    /// `TurnEnd` 判定：只有**队列排空的最终 `TurnEnd`** 才收口批次——队列
    /// 非空 = T10 还会逐放，批次继续；收口这一处是唯一的阈值判定与通知点。
    /// 无派发记录（`started` 为空）的 `TurnEnd` 零通知。
    pub fn note_turn_end(&mut self, queue_empty: bool, now: Instant) {
        if !queue_empty {
            return;
        }
        let Some(started) = self.started.take() else {
            return;
        };
        if self.suppressed {
            return;
        }
        if now.saturating_duration_since(started) >= BATCH_NOTIFY_MIN {
            self.pending = Some(notice_bytes(self.osc9));
            self.bells += 1;
        }
    }

    /// 取走待写出的通知字节（接线层消费；一个批次至多一条）。
    pub fn take_bytes(&mut self) -> Option<Vec<u8>> {
        self.pending.take()
    }

    /// OSC9 开关注入（启动读定一次，见 [`osc9_from_config`]）。
    pub fn set_osc9(&mut self, on: bool) {
        self.osc9 = on;
    }

    /// 累计响铃次数（测试断言「恰一次 / 零次」）。
    pub fn bells(&self) -> u64 {
        self.bells
    }

    /// 本批次是否已被输入抑制（测试断言抑制标志可见）。
    pub fn suppressed(&self) -> bool {
        self.suppressed
    }

    /// 是否有进行中批次（测试断言批次在途与收口边界）。
    pub fn batch_active(&self) -> bool {
        self.started.is_some()
    }
}

/// 启动读定的 OSC9 开关：`Config::load()` 的 `[tui] notify_osc9`——配置
/// 读不到 / 解析失败一律按默认关（不 panic、不刷错误；route §1 已知坑：
/// 启动读定，不热载）。三个接线点（enhanced / inline / linear）共用这一个
/// 读取口，不各自拼解析。
pub fn osc9_from_config() -> bool {
    config::Config::load().is_ok_and(|c| c.tui_notify_osc9)
}

/// 批次完成的写出字节：恒一次 `\a`（响铃主路径，与 OSC9 开关无关）；开关
/// 开时**附** OSC9 序列（`ESC ] 9 ; <msg> BEL`）。ESC / BEL 走 T12 的十进制
/// const（D11：`theme_lint` 禁转义与十六进制字面量，T8 / T12 先例）。
fn notice_bytes(osc9: bool) -> Vec<u8> {
    let mut out = vec![OSC_BEL];
    if osc9 {
        out.push(OSC_ESC);
        out.extend_from_slice(b"]9;");
        out.extend_from_slice(OSC9_MSG.as_bytes());
        out.push(OSC_BEL);
    }
    out
}
