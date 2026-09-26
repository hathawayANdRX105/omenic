//! message_actions — route §3 T12 / issue #473 Done-when：消息操作的入口
//! 仲裁（BackTab 聚焦环绕、Esc 优先级 overlay > 面板 > 清焦 > 退出确认、
//! 聚焦态 r/e/c 拦截而非聚焦原样落 composer）、运行中与队列非空的**单点**
//! 互斥、retry/edit 的截断点时序（retry 入队即挂、edit 提交才挂、清空
//! composer 即放弃）、copy 的 OSC52 载荷与 `copied` 反馈生命周期、焦点行
//! 标记渲染。
//!
//! [`App`] 是无 IO 纯状态机（按键进去、状态/队列/`KeyAction` 出来），RPC
//! 归 `app.rs` 事件循环——测试只断状态机与渲染，CI 无 TTY 照样成立；渲染
//! 断言走 ratatui `TestBackend`（同 `tests/search_overlay.rs` 取样法）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use omenic_tui::app::{App, KeyAction, osc52_bytes};
use omenic_tui::ui;
use web_state::types::{ChatMessage, MessagePart, ToolCall};

/// 80×24 下的 transcript 几何（同 `tests/search_overlay.rs`：transcript
/// 满宽 80、高 24 − dock 3 = 21，footer 再让 1 行 → 视口 20 行）。
const SCREEN_W: u16 = 80;
const SCREEN_H: u16 = 24;

/// 单行历史消息的唯一构造点（四则字段序列共用：无推理、无工具、无附件、
/// 零时间戳——变的只有 id / 角色 / 正文）。
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

/// 四条单行历史（user/assistant 交替；id = `{sid}-{seq}` ledger 口径，
/// retry/edit 的截断点就从这个 id 解析）。
fn app_with_history() -> App {
    let mut app = App::new();
    app.start_session(
        "s1",
        vec![
            mk_msg("s1-1", "user", "first question"),
            mk_msg("s1-2", "assistant", "first answer"),
            mk_msg("s1-3", "user", "second question"),
            mk_msg("s1-4", "assistant", "second answer"),
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

fn esc() -> KeyEvent {
    key(KeyCode::Esc)
}

/// Shift+Tab（正常模式空闲键，T12 的聚焦入口）。
fn backtab() -> KeyEvent {
    KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 每帧几何同步（事件循环 `draw` 前的那一步，测试走同一入口）。
fn sync(app: &mut App) {
    ui::sync_viewport(app, Rect::new(0, 0, SCREEN_W, SCREEN_H));
}

/// 画一帧，返回带下划线（[`omenic_tui::theme::focus`] 的标记属性）的行号。
fn underlined_rows(app: &App) -> Vec<u16> {
    let mut terminal = Terminal::new(TestBackend::new(SCREEN_W, SCREEN_H)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw must fit without panicking");
    let buf = terminal.backend().buffer();
    (0..buf.area.height)
        .filter(|&y| {
            (0..buf.area.width).any(|x| {
                buf.cell((x, y))
                    .is_some_and(|c| c.modifier.contains(Modifier::UNDERLINED))
            })
        })
        .collect()
}

/// 首次 BackTab 落**最新一条**，随后前向推进、到尾回绕到头（route §3
/// 注记④「前向环绕」）。
#[test]
fn backtab_focuses_latest_then_wraps_forward() {
    let mut app = app_with_history();
    assert_eq!(app.focused_message(), None, "起始无焦点");
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(3), "首次聚焦最新一条");
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(0), "到尾回绕到头");
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(1), "随后前向推进");
}

/// 本地 `system` 输出（/help 等）不是消息，不可聚焦——遍历只在
/// user/assistant 之间进行。
#[test]
fn backtab_skips_local_system_rows() {
    let mut app = App::new();
    app.start_session(
        "s1",
        vec![
            mk_msg("s1-1", "user", "question"),
            mk_msg("local-9", "system", "model: unset"),
            mk_msg("s1-2", "assistant", "answer"),
        ],
    );
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(2), "assistant 可聚焦");
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(0), "跳过中间的 system 行");
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(2), "回绕同样跳过 system 行");
}

/// Esc 优先级（route §3 注记④）：焦点存在时第一下只清焦、**不进**退出
/// 确认；无焦点后才两段式退出。
#[test]
fn esc_clears_focus_before_quit_confirm() {
    let mut app = app_with_history();
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(3));

    assert_eq!(app.handle_key(esc()), KeyAction::None);
    assert_eq!(app.focused_message(), None, "第一下清焦");
    assert!(!app.confirm_quit(), "清焦这一下不许同时进退出确认");

    assert_eq!(app.handle_key(esc()), KeyAction::None);
    assert!(app.confirm_quit(), "无焦点后才开始退出确认");
    assert_eq!(app.handle_key(esc()), KeyAction::Quit, "第二下才退出");
}

/// 优先级前两位压过清焦：搜索 overlay 收口全部按键（含 c，期间复制不出
/// 去），斜杠面板收口 Esc——两者各关一次后，Esc 才轮到清焦。
#[test]
fn overlay_and_panel_win_over_focus_clear() {
    let mut app = app_with_history();
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(3));

    app.handle_key(ctrl('r'));
    assert!(app.search_open(), "Ctrl+R 开搜索 overlay");
    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(app.take_copy(), None, "overlay 期间 c 不产生 copy");
    assert_eq!(app.handle_key(esc()), KeyAction::None);
    assert!(!app.search_open(), "Esc 关 overlay");
    assert_eq!(app.focused_message(), Some(3), "关 overlay 不清焦");

    app.handle_key(key(KeyCode::Char('/')));
    assert!(app.slash_visible(), "行首 / 开面板");
    assert_eq!(app.handle_key(esc()), KeyAction::None);
    assert!(!app.slash_visible(), "Esc 先关面板");
    assert_eq!(app.focused_message(), Some(3), "关面板不清焦");

    app.handle_key(esc());
    assert_eq!(app.focused_message(), None, "面板已关，Esc 才轮到清焦");
    assert!(!app.confirm_quit(), "这一下仍然只清焦");
}

/// 非聚焦态 r/e/c 照常落 composer（零打字干扰）。
#[test]
fn unfocused_ops_keys_type_into_composer() {
    let mut app = app_with_history();
    type_str(&mut app, "rec");
    assert_eq!(app.input(), "rec");
}

/// 聚焦态三键被操作路由拦截，一个字都进不了 composer；目标不对时是
/// 可见的 no-op（不入队、不截断、视图不动）。
#[test]
fn focused_ops_keys_never_reach_composer() {
    let mut app = app_with_history();
    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(3), "聚焦的是 assistant 消息");

    app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(app.input(), "", "r 不进 composer");
    assert_eq!(app.queued(), None, "assistant 不可 retry，不入队");
    assert_eq!(app.take_truncate(), None);

    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.input(), "", "e 不进 composer（目标必须是 user）");

    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(app.input(), "", "c 不进 composer");
    assert_eq!(app.messages().len(), 4, "聚焦操作不改视图");
}

/// copy 反馈：`copied` 短状态 + OSC52 正文入队，随下一次按键退场
///（route §3 注记①：只陈述「本方已写 stdout」这一已证明事实）。
#[test]
fn copy_reports_copied_and_clears_on_next_key() {
    let mut app = app_with_history();
    app.handle_key(backtab());
    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(app.status_text(), "copied");
    assert_eq!(app.take_copy(), Some("second answer".to_string()));

    app.type_char('x');
    assert_eq!(app.status_text(), "", "copied 随下一次按键恢复常态");
    assert_eq!(app.input(), "x");
}

/// copy 正文 = 可见文本：parts 非空按 Text 顺序拼接，tool part 跳过
///（卡片是界面物，不是消息文本）。
#[test]
fn copy_joins_text_parts_and_skips_tools() {
    let mut app = app_with_history();
    let mut msg = mk_msg("s1-4", "assistant", "");
    msg.parts = vec![
        MessagePart::Text("alpha".to_string()),
        MessagePart::Tool(ToolCall {
            id: "t1".to_string(),
            title: "bash".to_string(),
            kind: "bash".to_string(),
            summary: "ls".to_string(),
            detail: "file.txt".to_string(),
            status: "success".to_string(),
        }),
        MessagePart::Text("beta".to_string()),
    ];
    app.start_session(
        "s1",
        vec![
            mk_msg("s1-1", "user", "question"),
            mk_msg("s1-2", "assistant", "answer"),
            msg,
        ],
    );
    app.handle_key(backtab());
    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(app.take_copy(), Some("alpha\nbeta".to_string()));
}

/// 互斥（route §3 注记⑤，单点在 `message_key`）：队列非空时 r/e/c 全
/// no-op——聚焦本身不受限，三键被吞但不产生任何动作。
#[test]
fn ops_are_noop_while_prompt_queued() {
    let mut app = app_with_history();
    type_str(&mut app, "queued prompt");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.queued(), Some("queued prompt"), "T10 排队成功");

    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(3), "聚焦不参与互斥");

    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(app.take_copy(), None, "排队中 copy 同禁");
    app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(app.take_truncate(), None, "排队中 retry 同禁");
    assert_eq!(app.queued(), Some("queued prompt"), "没有入新队");
    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.input(), "", "排队中 edit 同禁");
    assert_eq!(app.messages().len(), 4, "排队中三键不改视图");
}

/// 互斥另一半：运行中（`running == true`）三键同样全 no-op，且 copy 不置
/// `copied`（活动行被 running 占用，反馈无处安放）。
#[test]
fn ops_are_noop_while_running() {
    let mut app = app_with_history();
    type_str(&mut app, "run me");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.queued(), Some("run me"), "Enter 只入队，出站归事件循环");
    // 出站 = 消费队首（事件循环每次迭代的那一步）：置 running、把 user
    // 消息折进 transcript 投影（焦点就落在它上面）。
    assert!(app.next_to_send().is_some(), "出站即 running");
    assert!(app.is_running());

    app.handle_key(backtab());
    assert_eq!(app.focused_message(), Some(4), "聚焦刚出站的那条 user");

    app.handle_key(key(KeyCode::Char('c')));
    assert_eq!(app.take_copy(), None, "运行中 copy 同禁");
    assert_ne!(app.status_text(), "copied", "运行中不置 copied 反馈");
    app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(app.take_truncate(), None, "运行中 retry 同禁");
    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.input(), "", "运行中 edit 同禁");
}

/// retry：聚焦 user 按 r → 截断点入队即挂（`seq >=` 该条）、原文进既有
/// 出站队列、视图裁到改写点；出站后落库回执把新消息回填成 ledger 序号。
#[test]
fn retry_truncates_requeues_and_stamps_seq() {
    let mut app = app_with_history();
    for _ in 0..4 {
        app.handle_key(backtab());
    }
    assert_eq!(app.focused_message(), Some(2), "聚焦第三条（user）");

    app.handle_key(key(KeyCode::Char('r')));
    assert_eq!(app.take_truncate(), Some(3), "截断点 = 该条的 ledger seq");
    assert_eq!(app.queued(), Some("second question"), "原文入出站队列");
    assert_eq!(app.messages().len(), 2, "视图裁到改写点（含其后全部）");
    assert_eq!(app.messages()[1].content, "first answer", "前段原样保留");
    assert_eq!(app.focused_message(), None, "被裁消息的焦点作废");

    assert_eq!(
        app.next_to_send(),
        Some("second question".to_string()),
        "三元组重发原文"
    );
    app.note_appended_seq(9);
    assert_eq!(
        app.messages().last().map(|m| m.id.as_str()),
        Some("s1-9"),
        "落库回执把出站消息回填成 {{sid}}-{{seq}} 口径"
    );
}

/// edit：聚焦 user 按 e → 文本回填 composer（截断**不**在此刻），提交才
/// 挂截断点并裁视图；改文进队列。
#[test]
fn edit_refills_composer_and_truncates_on_submit() {
    let mut app = app_with_history();
    for _ in 0..4 {
        app.handle_key(backtab());
    }
    assert_eq!(app.focused_message(), Some(2));

    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.input(), "second question", "原文回填 composer");
    assert_eq!(app.focused_message(), None, "焦点让位给编辑");
    assert_eq!(app.take_truncate(), None, "e 不挂截断点（提交才消费）");
    assert_eq!(app.messages().len(), 4, "e 不动视图（尚未截断）");

    app.type_char('!');
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.take_truncate(), Some(3), "提交才挂截断点");
    assert_eq!(app.messages().len(), 2, "视图裁到改写点");
    assert_eq!(app.queued(), Some("second question!"), "改文入出站队列");
}

/// edit 放弃：composer 被清空即撤下挂起——之后一次无关发送**不许**把历史
/// 尾段截掉（route §3 注记③只定「提交时消费」，放弃判据在这里补）。
#[test]
fn edit_discarded_when_composer_emptied() {
    let mut app = app_with_history();
    for _ in 0..4 {
        app.handle_key(backtab());
    }
    app.handle_key(key(KeyCode::Char('e')));
    assert_eq!(app.input(), "second question");

    for _ in 0.."second question".len() {
        app.handle_key(key(KeyCode::Backspace));
    }
    assert_eq!(app.input(), "");
    assert_eq!(app.status_text(), "edit discarded", "清空即放弃，且可见");
    assert_eq!(app.take_truncate(), None);

    type_str(&mut app, "fresh");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.take_truncate(), None, "放弃后发送不再截断历史");
    assert_eq!(app.messages().len(), 4, "历史原样保留");
    assert_eq!(app.queued(), Some("fresh"), "新消息照常排队");
}

/// OSC52 载荷 = `ESC ] 52 ; c ; <base64> BEL`（route §3 注记①定案：载体
/// OSC52、终端原生、零 daemon 依赖）。字节全用十进制 const 拼装（D11）。
/// base64 向量取 RFC 4648 §10（`f`/`fo`/`foo`/`fooba`/`foobar` 覆盖无 padding
/// 与两级 padding）+ 一条 UTF-8 多字节正文（中文消息是常态输入）。
#[test]
fn osc52_bytes_match_known_base64_vectors() {
    for (text, b64) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
        ("中文", "5Lit5paH"),
    ] {
        let mut expect = vec![27u8];
        expect.extend_from_slice(b"]52;c;");
        expect.extend_from_slice(b64.as_bytes());
        expect.push(7);
        assert_eq!(osc52_bytes(text), expect, "payload for {text:?}");
    }
}

/// 焦点行标记（route §3 注记④「最小标记」）：聚焦消息的首条可见行整体
/// 下划线，无焦点时一格都不画——transcript 只读叠加，不改渲染内核。
#[test]
fn focus_marker_underlines_focused_row() {
    let mut app = app_with_history();
    sync(&mut app);
    assert_eq!(underlined_rows(&app), Vec::<u16>::new(), "无焦点不画标记");

    for _ in 0..4 {
        app.handle_key(backtab());
    }
    assert_eq!(app.focused_message(), Some(2));
    // 行坐标：msg0/msg1 各 1 行 → 焦点消息首行 = 视口第 2 行（80×24 视口
    // 20 行、钉底 top=0）。
    assert_eq!(underlined_rows(&app), vec![2u16]);
}

/// inline 档总闸（route §3 T8 设计注记：回看/复制归终端）：关闸后聚焦与
/// 三键全部熄火，键位行为与合入前一致。
#[test]
fn message_ops_gate_shuts_focus_and_keys() {
    let mut app = app_with_history();
    app.disable_message_ops();

    app.handle_key(backtab());
    assert_eq!(app.focused_message(), None, "关闸后 BackTab 不聚焦");

    type_str(&mut app, "rec");
    assert_eq!(app.input(), "rec", "关闸后三键纯打字");
}
