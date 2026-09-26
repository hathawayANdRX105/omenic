//! turn_notify — route §3 T15 完成通知契约（任务书 §4：阈值 / 聚合 /
//! 抑制 / OSC9 开关 / 非批次 TurnEnd；第六条 = 既有 T1–T14 回归，交 CI）。
//!
//! [`App`] 是无 IO 状态机（同 `tests/keys.rs`）：按键进去、通知字节数出
//! 来。时钟经 [`App::note_turn_end_at`] 注入（T7 `wheel_tick(now)` 范式），
//! 测试不真睡阈值。批次起点用回拨口径：`t0` 在派发前取，真实派发只会更
//! 晚——注入 `t0 + 2×阈值` 作完成时刻，时长最多被压短一次派发延迟（远
//! 小于 10s 余量）稳落 ≥ 一侧；注入 `t0 + 9s` 则无论派发延迟多大恒 < 阈值。

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use omenic_tui::app::{App, KeyAction};
use omenic_tui::notify::{BATCH_NOTIFY_MIN, BatchNotify};
use web_state::ui_state::AgentEvent;

/// 完成时刻：批次时长 ≥ 阈值（回拨 10s 余量，见文件头注释）。
fn long_end(t0: Instant) -> Instant {
    t0 + BATCH_NOTIFY_MIN * 2
}

/// 完成时刻：批次时长 < 阈值（无论派发延迟多大恒成立）。
fn short_end(t0: Instant) -> Instant {
    t0 + BATCH_NOTIFY_MIN - Duration::from_secs(1)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 打一行并回车（入出站队列；是否真出站由调用方 `take_prompt` 决定，
/// 语义同 `tests/queue_fifo.rs` 的同名桩）。
fn submit(app: &mut App, text: &str) {
    type_str(app, text);
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        KeyAction::None,
        "Enter 只提交不路由"
    );
}

/// 起一批次：空闲提交并出站，返回回拨过的批次起点。
fn start_batch(app: &mut App, text: &str) -> Instant {
    let t0 = Instant::now();
    submit(app, text);
    app.take_prompt().expect("空闲提交必须出站");
    t0
}

/// 1 阈值边界（上半）：≥ 阈值的批次完成恰响一次——首字节 BEL、第二取空；
/// 恰好等于阈值也响（≥ 取等，状态机直驱时钟精确）。
#[test]
fn batch_over_threshold_rings_exactly_once() {
    let mut app = App::new();
    let t0 = start_batch(&mut app, "long turn");
    app.note_turn_end_at(long_end(t0));

    assert_eq!(app.notify().bells(), 1, "≥ 阈值批次完成响一次");
    assert_eq!(
        app.take_notice().expect("恰一条通知"),
        vec![7u8],
        "缺省只写单字节 BEL"
    );
    assert!(app.take_notice().is_none(), "一条批次只产一条通知");

    // 阈值取等（恰 10s）也算 ≥：状态机直驱，时钟无回拨误差。
    let mut bare = BatchNotify::new();
    let t0 = Instant::now();
    bare.note_dispatch(t0);
    bare.note_turn_end(true, t0 + BATCH_NOTIFY_MIN);
    assert_eq!(bare.bells(), 1, "恰好等于阈值也响（≥ 取等）");
}

/// 1 阈值边界（下半）：批次总时长 < 阈值 → 零次（短 turn 不响）。
#[test]
fn batch_under_threshold_stays_silent() {
    let mut app = App::new();
    let t0 = start_batch(&mut app, "short turn");
    app.note_turn_end_at(short_end(t0));

    assert_eq!(app.notify().bells(), 0, "阈值内完成不响");
    assert!(app.take_notice().is_none(), "零通知可取");
}

/// 2 聚合：三条排队成一个批次，只在**队列排空的最终 TurnEnd** 响一次——
/// 中间 TurnEnd（时长已超阈值）与流式 chunk 全程零通知，钉住不存在逐
/// turn / 逐 chunk 通知路径（bug 本体：每次 chunk 都响）。
#[test]
fn queued_batch_aggregates_to_single_final_notice() {
    let mut app = App::new();
    // 三条先全部入队（批次开始前输入：无进行中批次，不预埋抑制）。
    submit(&mut app, "first");
    submit(&mut app, "second");
    submit(&mut app, "third");
    let t0 = Instant::now();
    let first = app.take_prompt().expect("队首出站");
    assert_eq!(first.text, "first");
    assert!(!app.notify().suppressed(), "批次前输入不预埋抑制");

    // 流式 chunk 只进投影，通知状态机纹丝不动。
    for _ in 0..16 {
        app.apply_event(&AgentEvent::AssistantText {
            delta: "tok".into(),
        });
    }
    assert_eq!(app.notify().bells(), 0, "chunk 阶段零响铃");

    // 中间 TurnEnd（时长已超阈值）：队列非空 → 批次继续，不判不响。
    app.note_turn_end_at(long_end(t0));
    assert_eq!(app.notify().bells(), 0, "中间 TurnEnd 不响");
    assert!(app.take_notice().is_none(), "中间没有可写出的通知");
    assert!(app.notify().batch_active(), "批次仍在途");

    // T10 逐放：每个 TurnEnd 后队首出一条，仍然不响。
    let second = app.take_prompt().expect("逐放第二条");
    assert_eq!(second.text, "second");
    app.note_turn_end_at(long_end(t0) + Duration::from_secs(1));
    assert_eq!(app.notify().bells(), 0, "第二个中间 TurnEnd 也不响");

    let third = app.take_prompt().expect("逐放第三条");
    assert_eq!(third.text, "third");
    // 队列排空的最终 TurnEnd → 恰一次。
    app.note_turn_end_at(long_end(t0) + Duration::from_secs(2));
    assert_eq!(app.notify().bells(), 1, "整批只响一次");
    assert!(app.take_notice().is_some(), "恰好一条通知");
    assert!(app.take_notice().is_none(), "没有第二条");
    assert!(!app.notify().batch_active(), "批次已收口");
}

/// 3 抑制：批次进行中的可打印输入 → 本批次零通知（抑制标志可见）；批次
/// 收口后重新提交 = 新批次，首个派发重新武装、恢复响铃。
#[test]
fn input_during_batch_suppresses_notice_and_rearms() {
    let mut app = App::new();
    let t0 = start_batch(&mut app, "first");
    type_str(&mut app, "typing while it runs");
    assert!(app.notify().suppressed(), "抑制标志可见（任务书 §4）");

    app.note_turn_end_at(long_end(t0));
    assert_eq!(app.notify().bells(), 0, "被抑制的批次不响");
    assert!(app.take_notice().is_none());
    assert!(!app.notify().batch_active(), "被抑制也算收口，不是挂起");

    // 批次收口后的输入不预埋：新批次首个派发即重新武装。
    submit(&mut app, "second");
    let t1 = Instant::now();
    app.take_prompt().expect("新批次出站");
    assert!(!app.notify().suppressed(), "下一批次重新武装");
    app.note_turn_end_at(long_end(t1));
    assert_eq!(app.notify().bells(), 1, "下一批次恢复响铃");
}

/// 4 OSC9 开关：缺省（字段未设）→ 只有单字节 BEL；置 true → 附 OSC9 序列
/// （`ESC ] 9 ; 文案 BEL`），响铃次数与主路径字节不受影响。
#[test]
fn osc9_flag_appends_sequence_without_extra_bells() {
    let mut default = App::new();
    let t0 = start_batch(&mut default, "default off");
    default.note_turn_end_at(long_end(t0));
    assert_eq!(default.notify().bells(), 1, "默认关同样响铃");
    assert_eq!(
        default.take_notice().expect("默认也有通知"),
        vec![7u8],
        "缺省只有 BEL"
    );

    let mut on = App::new();
    on.set_notify_osc9(true);
    let t0 = start_batch(&mut on, "osc9 on");
    on.note_turn_end_at(long_end(t0));
    assert_eq!(on.notify().bells(), 1, "开关不改响铃次数");
    let bytes = on.take_notice().expect("开关下同样有通知");
    assert_eq!(bytes.first(), Some(&7u8), "BEL 主路径照旧打头");
    let esc = bytes
        .iter()
        .position(|&b| b == 27u8)
        .expect("OSC9 开场 ESC");
    assert_eq!(&bytes[esc + 1..esc + 4], b"]9;", "ESC 后紧跟 ]9; 序列在场");
    assert_eq!(bytes.last(), Some(&7u8), "OSC9 以 BEL 收尾");
}

/// 5 边界：非批次 TurnEnd（无派发记录）零通知；批次收口后迟到的 TurnEnd
/// 也不追加（「恰一次」的另一半）。
#[test]
fn turn_end_without_dispatched_batch_stays_silent() {
    let mut app = App::new();
    assert!(!app.notify().batch_active());
    app.note_turn_end_at(Instant::now() + BATCH_NOTIFY_MIN * 2);
    assert_eq!(app.notify().bells(), 0, "无派发记录的 TurnEnd 零通知");
    assert!(app.take_notice().is_none());

    let t0 = start_batch(&mut app, "one");
    app.note_turn_end_at(long_end(t0));
    assert!(app.take_notice().is_some());
    app.note_turn_end_at(long_end(t0) + Duration::from_secs(5));
    assert_eq!(app.notify().bells(), 1, "迟到 TurnEnd 不追加响铃");
    assert!(app.take_notice().is_none(), "也不再生第二条");
}
