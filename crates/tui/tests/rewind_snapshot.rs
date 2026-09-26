//! rewind_snapshot — route §3 T13 / issue #474 Done-when：逐轮回退的点位
//! 计算（第 n 轮 user 消息的 ledger seq）、n 校验零动作、确认态模态（y/Enter
//! 确认、n/Esc 取消、其余键一律忽略——盖过 Ctrl+R 搜索与 Ctrl+D 退出）、
//! 运行中/排队互斥拒绝、成功回填（transcript 对齐回退点 + `rewound n
//! turn(s)`）、失败可见（`rewind failed: <reason>` 且视图一个字节不动）。
//!
//! [`App`] 是无 IO 纯状态机（按键进去、状态/确认态/`take_rewind` 出来），
//! RPC 归 `app.rs` 事件循环——`session.rewind` 的落库契约由
//! `crates/daemon/tests/session_rewind.rs` 另钉，本文件钉状态机半边。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use omenic_tui::app::{App, KeyAction};
use omenic_tui::slash::{self, Action, COMMANDS};
use web_state::types::ChatMessage;

/// 单行历史消息的唯一构造点（与 `tests/message_actions.rs` 同签名——
/// 四则字段序列共用一个助手，不另起炉灶）。
fn mk_msg(id: &str, role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![],
        attachments: vec![],
        timestamp: String::new(),
        ts_epoch_ms: 0,
    }
}

/// 三轮对话（6 条单行：user/assistant 交替；id = `{sid}-{seq}` ledger
/// 口径）——user 消息 seq = 1/3/5，回退点就从这三个里取。
fn app_with_turns() -> App {
    let mut app = App::new();
    app.start_session(
        "s1",
        vec![
            mk_msg("s1-1", "user", "first question"),
            mk_msg("s1-2", "assistant", "first answer"),
            mk_msg("s1-3", "user", "second question"),
            mk_msg("s1-4", "assistant", "second answer"),
            mk_msg("s1-5", "user", "third question"),
            mk_msg("s1-6", "assistant", "third answer"),
        ],
    );
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn enter() -> KeyEvent {
    key(KeyCode::Enter)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 两段 Enter：第一段补全（带参行保留参数尾巴）、第二段执行进确认态。
fn run_rewind(app: &mut App, line: &str) {
    type_str(app, line);
    assert_eq!(app.handle_key(enter()), KeyAction::None, "第一段只补全");
    assert_eq!(app.handle_key(enter()), KeyAction::None, "第二段执行");
}

/// ① 回退点 = 倒数第 n 条 user 消息的 seq（n=1/2/3 → 5/3/1）；确认态
/// 显示的轮数 = 已校验的 n。
#[test]
fn rewind_point_is_nth_user_turn_seq() {
    for (n, from_seq) in [(1, 5), (2, 3), (3, 1)] {
        let mut app = app_with_turns();
        run_rewind(&mut app, &format!("/rewind {n}"));
        assert_eq!(
            app.rewind_confirm(),
            Some((from_seq, n)),
            "n={n} 的回退点必须是倒数第 {n} 轮 user 消息的 seq"
        );
        assert_eq!(
            app.status_text(),
            format!("discard {n} turn(s)? [y/n]"),
            "确认态显示将丢弃的回合数"
        );
        assert_eq!(app.take_rewind(), None, "未确认不产出动作");
    }
}

/// 带参行的两段 Enter：参数尾巴随补全保留（`/rewind 3` 不被补全抹成
/// `/rewind`），第二段才按 n=3 执行。
#[test]
fn arg_tail_survives_first_stage_completion() {
    let mut app = app_with_turns();
    type_str(&mut app, "/rewind 3");
    assert_eq!(app.handle_key(enter()), KeyAction::None);
    assert_eq!(app.input(), "/rewind 3", "第一段补全保留参数尾巴");
    assert_eq!(app.handle_key(enter()), KeyAction::None);
    assert_eq!(
        app.rewind_confirm(),
        Some((1, 3)),
        "n=3 的回退点 = 最早一条 user 消息"
    );
}

/// ② 校验拒绝 = 显式报错 + 零动作：非整数、零、越界、没有 user 轮——
/// 一样都不进确认态、不动视图、不挂动作。
#[test]
fn bad_n_is_rejected_with_zero_action() {
    let cases = [
        (
            "/rewind abc",
            "rewind: expected an integer turn count, got \"abc\"",
        ),
        ("/rewind 0", "rewind: n must be between 1 and 3"),
        ("/rewind 99", "rewind: n must be between 1 and 3"),
    ];
    for (line, status) in cases {
        let mut app = app_with_turns();
        run_rewind(&mut app, line);
        assert_eq!(app.status_text(), status, "{line} 的报错要可见");
        assert_eq!(app.rewind_confirm(), None, "{line} 不许进确认态");
        assert_eq!(app.take_rewind(), None, "{line} 不许挂动作");
        assert_eq!(app.messages().len(), 6, "{line} 不动视图");
    }

    // 没有 user 轮的会话（只有 assistant）：同样显式报错零动作。
    let mut app = App::new();
    app.start_session(
        "s1",
        vec![
            mk_msg("s1-1", "assistant", "hello"),
            mk_msg("s1-2", "system", "local note"),
        ],
    );
    run_rewind(&mut app, "/rewind");
    assert_eq!(app.status_text(), "rewind: no user turns in this session");
    assert_eq!(app.rewind_confirm(), None);
    assert_eq!(app.take_rewind(), None);
}

/// ② 确认态模态（route §3 触发裁决）：其余键一律忽略——Ctrl+R 不开搜索、
/// Ctrl+D 不退出、普通字符不进 composer，状态行提示不被冲掉。
#[test]
fn confirm_state_is_modal_over_every_key() {
    let mut app = app_with_turns();
    run_rewind(&mut app, "/rewind");
    assert_eq!(app.status_text(), "discard 1 turn(s)? [y/n]");

    assert_eq!(app.handle_key(ctrl('r')), KeyAction::None);
    assert!(!app.search_open(), "模态盖过 Ctrl+R 搜索 overlay");
    assert_eq!(app.status_text(), "discard 1 turn(s)? [y/n]");

    assert_eq!(app.handle_key(ctrl('d')), KeyAction::None, "模态盖过退出");
    assert!(!app.confirm_quit(), "Ctrl+D 不许进退出确认");

    assert_eq!(app.type_char('x'), KeyAction::None);
    assert_eq!(app.input(), "", "普通字符不落 composer");
    assert_eq!(app.handle_key(key(KeyCode::Tab)), KeyAction::None);
    assert!(!app.tools_expanded(), "Tab 不翻工具卡");

    assert!(app.rewind_confirm().is_some(), "被忽略的键不许把确认态弄丢");
    assert_eq!(app.take_rewind(), None, "被忽略的键不许产出动作");
}

/// ② 取消路径（n / Esc）零动作：回退点、视图、动作队列全都不动，只留一行
/// 可见的取消回执。
#[test]
fn cancel_paths_are_zero_action() {
    for cancel in [key(KeyCode::Char('n')), key(KeyCode::Esc)] {
        let mut app = app_with_turns();
        run_rewind(&mut app, "/rewind");
        assert_eq!(app.handle_key(cancel), KeyAction::None);
        assert_eq!(app.status_text(), "rewind cancelled", "取消回执可见");
        assert_eq!(app.rewind_confirm(), None, "取消撤下确认态");
        assert_eq!(app.take_rewind(), None, "取消不产出动作");
        assert_eq!(app.messages().len(), 6, "取消不动视图");
    }
}

/// ② 确认路径（y / Enter）：确认态转成动作 `(from_seq, n)`、提示行让位，
/// 视图此刻仍原样（对齐由回填负责，见下一条）。
#[test]
fn confirm_produces_the_rewind_action() {
    for confirm in [key(KeyCode::Char('y')), key(KeyCode::Char('Y')), enter()] {
        let mut app = app_with_turns();
        run_rewind(&mut app, "/rewind");
        assert_eq!(app.handle_key(confirm), KeyAction::None);
        assert_eq!(app.rewind_confirm(), None, "确认态已消费");
        assert_eq!(
            app.take_rewind(),
            Some((5, 1)),
            "动作 = 已校验的 (回退点, 轮数)"
        );
        assert_eq!(app.status_text(), "", "提示行让位给结果行");
        assert_eq!(app.take_rewind(), None, "动作一次性，不重复产出");
    }
}

/// 互斥（与 T12 同款，入口在斜杠执行路径）：排队中或运行中执行 `/rewind`
/// 即拒——状态行明说、零动作。
#[test]
fn rewind_refused_while_queued_or_running() {
    // 排队中（T10 队列非空）。
    let mut queued = app_with_turns();
    type_str(&mut queued, "queued prompt");
    queued.handle_key(enter());
    assert_eq!(queued.queued(), Some("queued prompt"));
    run_rewind(&mut queued, "/rewind");
    assert_eq!(
        queued.status_text(),
        "rewind unavailable while running or queued"
    );
    assert_eq!(queued.rewind_confirm(), None, "拒绝不进确认态");
    assert_eq!(queued.take_rewind(), None, "拒绝不挂动作");

    // 运行中（出站置 running）。
    let mut running = app_with_turns();
    type_str(&mut running, "run me");
    running.handle_key(enter());
    assert!(running.next_to_send().is_some(), "出站即 running");
    run_rewind(&mut running, "/rewind");
    assert_eq!(
        running.status_text(),
        "rewind unavailable while running or queued"
    );
    assert_eq!(running.rewind_confirm(), None);
    assert_eq!(running.take_rewind(), None);
}

/// ③ 失败可见（真值安全）：RPC 失败 → 状态行 `rewind failed: <reason>`，
/// 本地投影一个字节都不动——不存在假装成功路径。
#[test]
fn failure_reports_and_leaves_view_untouched() {
    let mut app = app_with_turns();
    run_rewind(&mut app, "/rewind");
    app.handle_key(key(KeyCode::Char('y')));
    assert_eq!(
        app.take_rewind(),
        Some((5, 1)),
        "确认产出动作（事件循环发 RPC）"
    );

    // 事件循环的失败回填（RPC 拒绝）。
    app.note_rewind_failed("daemon rejected the snapshot");
    assert_eq!(
        app.status_text(),
        "rewind failed: daemon rejected the snapshot"
    );
    assert_eq!(app.messages().len(), 6, "失败不动视图");
    assert_eq!(app.rewind_confirm(), None, "失败不回确认态");
}

/// ④ 成功回填：transcript 强制对齐回退后的 ledger（消息止于回退点之前，
/// 走切会话同一条重载路径）+ 状态行 `rewound n turn(s)`。
#[test]
fn success_aligns_transcript_and_reports_turns() {
    // ledger 回退点 = 5（丢 seq >= 5）→ 重载回来的历史止于 seq 4。
    let history = vec![
        mk_msg("s1-1", "user", "first question"),
        mk_msg("s1-2", "assistant", "first answer"),
        mk_msg("s1-3", "user", "second question"),
        mk_msg("s1-4", "assistant", "second answer"),
    ];
    let mut app = app_with_turns();
    run_rewind(&mut app, "/rewind");
    app.handle_key(key(KeyCode::Char('y')));
    assert_eq!(
        app.take_rewind(),
        Some((5, 1)),
        "确认产出动作（事件循环发 RPC）"
    );

    // 事件循环的成功回填（RPC 通过 + load_messages 重载）。
    app.note_rewind_ok(1, history);
    let msgs = app.messages();
    assert_eq!(msgs.len(), 4, "transcript 止于回退点之前");
    assert_eq!(msgs.last().map(|m| m.id.as_str()), Some("s1-4"));
    assert_eq!(app.status_text(), "rewound 1 turn(s)");
    assert_eq!(app.take_rewind(), None, "动作已消费完");
}

/// 确认态提示行不许被迟到的 TurnEnd 冲掉（活动覆写在确认期间让位）。
#[test]
fn turn_end_keeps_the_confirm_prompt() {
    let mut app = app_with_turns();
    run_rewind(&mut app, "/rewind");
    assert_eq!(app.status_text(), "discard 1 turn(s)? [y/n]");

    app.note_turn_end();
    assert_eq!(
        app.status_text(),
        "discard 1 turn(s)? [y/n]",
        "迟到的 TurnEnd 不许清掉模态提示"
    );
    assert!(app.rewind_confirm().is_some());
}

/// ⑤ 注册表 6→7 同步：`/rewind` 表尾登记、`/help` 必含、模糊过滤命中且
/// 不牵连无参命令。
#[test]
fn registry_and_help_cover_the_seventh_command() {
    let names: Vec<&str> = COMMANDS.iter().map(|cmd| cmd.name).collect();
    assert_eq!(
        names.last(),
        Some(&"/rewind"),
        "T13 追加在表尾（顺序即面板默认顺序）"
    );
    let cmd = slash::find("/rewind").expect("/rewind 必须注册");
    assert_eq!(cmd.action, Action::Rewind);
    assert!(!cmd.description.is_empty());
    assert!(slash::help_text().contains("/rewind"), "/help 必含 /rewind");
    assert_eq!(
        slash::filter("/re")
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>(),
        vec!["/rewind"],
        "模糊过滤 /re 唯一命中 /rewind"
    );
}

/// 切会话 = 确认态/待执行动作随旧台作废（回退点 from_seq 属于旧会话，
/// 留着会在新台按旧 seq 回退错台）。确认前撤的是确认态，确认后撤的是
/// 已挂起的动作——两半都要随台作废。
#[test]
fn switching_sessions_drops_pending_rewind() {
    let mut confirming = app_with_turns();
    run_rewind(&mut confirming, "/rewind");
    assert_eq!(confirming.rewind_confirm(), Some((5, 1)));
    confirming.switch_session("s2", vec![mk_msg("s2-1", "user", "hi")]);
    assert_eq!(confirming.rewind_confirm(), None, "切台撤下确认态");
    assert_eq!(confirming.status_text(), "", "切台清状态行");

    let mut armed = app_with_turns();
    run_rewind(&mut armed, "/rewind");
    armed.handle_key(key(KeyCode::Char('y')));
    assert_eq!(armed.take_rewind(), Some((5, 1)), "先确认出动作");
    // 动作被事件循环消费走后再挂一笔，切台验证这半也随台作废。
    run_rewind(&mut armed, "/rewind");
    armed.handle_key(key(KeyCode::Char('y')));
    assert_eq!(armed.rewind_confirm(), None, "第二次确认已消费");
    armed.switch_session("s2", vec![mk_msg("s2-1", "user", "hi")]);
    assert_eq!(armed.take_rewind(), None, "切台撤下待执行动作");
}
