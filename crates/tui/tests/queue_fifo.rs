//! queue_fifo — route §3 T10 运行中排队契约：FIFO 顺序、双上限（8 条 /
//! 64KiB 总量）拒收可见、首行 ↑ 召回最新项且其余不重排、`TurnEnd` 逐条
//! 自动消费、Esc 不碰队列、`queued: n` 计数三点。
//!
//! [`App`] 是无 IO 纯状态机（同 `tests/keys.rs`）：按键进去、出站队列
//! 出来——对账的就是「排队乱序或运行中丢失」这类 bug，断言在 CI 无 TTY
//! 环境下照样成立。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use omenic_tui::app::{App, KeyAction};
use omenic_tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn enter() -> KeyEvent {
    key(KeyCode::Enter)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 打一行并回车（是否真出站由调用方 `next_to_send` 决定；运行中 = 入队）。
fn submit(app: &mut App, text: &str) {
    type_str(app, text);
    assert_eq!(
        app.handle_key(enter()),
        KeyAction::None,
        "Enter 只提交不路由"
    );
}

/// 起一轮在跑：空闲提交 `first` 并出站（`running` 置位），返回出站正文。
fn start_turn(app: &mut App, first: &str) -> String {
    submit(app, first);
    let text = app.next_to_send().expect("空闲提交即刻出站");
    assert!(app.is_running());
    text
}

/// TestBackend 缓冲 → 逐行文本（取样法同 `tests/layout.rs`）。
fn buffer_text(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    out
}

/// 渲染一帧并取全文（`queued: n` 计数的渲染面断言用）。
fn rendered(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw must fit without panicking");
    buffer_text(terminal.backend().buffer())
}

/// Done when 1（不打断）：运行中 Enter 不中断当前回合、不出站，消息入队。
#[test]
fn running_enter_queues_without_interrupting_turn() {
    let mut app = App::new();
    start_turn(&mut app, "seed");

    submit(&mut app, "more");

    assert_eq!(app.queued_count(), 1, "运行中 Enter 入队");
    assert!(app.is_running(), "不许打断当前回合");
    assert_eq!(app.next_to_send().as_deref(), None, "运行中不许出站");
    assert_eq!(app.queued(), Some("more"), "队首 = 待发正文");
    assert_eq!(
        app.messages().len(),
        1,
        "入队不折 transcript——只有出站那条在"
    );
}

/// Done when 1（顺序）+ 4：入 A、B、C → 消费严格按 A、B、C，无乱序。
#[test]
fn queue_consumes_in_fifo_order_after_turn_end() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    for text in ["A", "B", "C"] {
        submit(&mut app, text);
    }
    assert_eq!(app.queued_count(), 3);

    // 每个 TurnEnd 放一条（事件循环 `take_prompt` 的真实节奏）。
    let mut sent = Vec::new();
    for _ in 0..3 {
        app.note_turn_end();
        sent.push(app.next_to_send().expect("队列未空必须能出站"));
    }
    assert_eq!(sent, ["A", "B", "C"], "消费必须按入队顺序");
    assert_eq!(app.queued_count(), 0, "消费后计数归零");
}

/// Done when 1（8 条上限）：第 9 条拒收可见、原文退回 composer、
/// 已排队项一条不少。
#[test]
fn queue_rejects_ninth_prompt_with_visible_status() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    for i in 0..8 {
        submit(&mut app, &format!("q{i}"));
    }
    assert_eq!(app.queued_count(), 8, "第 8 条入队（上限内）");

    submit(&mut app, "ninth");

    assert_eq!(app.queued_count(), 8, "满员拒收，不许挤掉已有项");
    assert_eq!(app.input(), "ninth", "拒收不静默丢——原文退回 composer");
    assert!(
        app.status_text().contains("queue full"),
        "状态行要明说：{}",
        app.status_text()
    );
    assert!(
        app.status_text().contains("8 prompts"),
        "报条数上限：{}",
        app.status_text()
    );
    // 拒收没打乱队列：回合结束仍按原序出站首条。
    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), Some("q0"));
}

/// Done when 1（64KiB 上限）：字节按**队列总量**记账——恰好 64KiB 允许、
/// 超 1 字节拒收；出队释放额度后可再入。
#[test]
fn queue_rejects_when_total_bytes_exceed_64kib() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    let big = "x".repeat(32 * 1024);

    // 两条 32KiB 正好顶到 64KiB 总量（≤ 上限 = 允许）。
    submit(&mut app, &big);
    submit(&mut app, &big);
    assert_eq!(app.queued_count(), 2, "总量恰达 64KiB 仍可入队");

    // 再塞一条：超额拒收，原文退回、状态行明说。
    submit(&mut app, "overflow");
    assert_eq!(app.queued_count(), 2, "超额不入队");
    assert_eq!(app.input(), "overflow", "拒收保原文");
    assert!(
        app.status_text().contains("64KiB"),
        "报字节上限：{}",
        app.status_text()
    );

    // 出队释放字节账：消费掉一条 32KiB 后，同一段文本又能入队。
    app.note_turn_end();
    assert_eq!(app.next_to_send().map(|t| t.len()), Some(32 * 1024));
    assert_eq!(app.queued_count(), 1);
    app.handle_key(enter()); // composer 里还是刚被拒的 "overflow"
    assert_eq!(app.queued_count(), 2, "出队释放额度后可再入队");
}

/// Done when 3（召回）：composer 为空 + 队列非空 → 首行 ↑ 召回**最新项**
/// （队尾）进编辑区，其余项保持顺序不重排（dh-rs `recall_latest` 口径）。
#[test]
fn first_line_up_recalls_latest_without_reordering() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    for text in ["A", "B", "C"] {
        submit(&mut app, text);
    }
    assert!(app.input().is_empty(), "composer 空 = 首行");

    assert_eq!(app.handle_key(key(KeyCode::Up)), KeyAction::None);

    assert_eq!(app.input(), "C", "召回最新项（队尾）进编辑区");
    assert_eq!(app.queued_count(), 2, "召回即出队");

    // 其余项顺序不重排：仍按 A、B 消费。
    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), Some("A"));
    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), Some("B"));
    assert_eq!(app.queued_count(), 0, "没丢也没乱");
}

/// Done when 3（让位既有键位）：composer 非空时 ↑ 不召回——落回历史导航
/// （队列一条不动，↓ 原路返回草稿证明导航在跑）。
#[test]
fn up_with_composer_text_yields_to_history_not_recall() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    for text in ["A", "B", "C"] {
        submit(&mut app, text);
    }
    type_str(&mut app, "draft");

    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.queued_count(), 3, "composer 非空不许召回");

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.input(), "draft", "↓ 原路返回草稿 = 历史导航在跑");
    assert_eq!(app.queued_count(), 3, "历史导航不碰队列");
}

/// Done when 4（逐条自动消费）：一个 TurnEnd 只放一条（同回合不许连发），
/// 队列排空无丢失。
#[test]
fn turn_end_drains_one_prompt_per_turn_without_loss() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    for text in ["A", "B", "C"] {
        submit(&mut app, text);
    }

    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), Some("A"));
    assert_eq!(app.queued_count(), 2, "本回合只消费一条");
    assert_eq!(
        app.next_to_send().as_deref(),
        None,
        "下一个 TurnEnd 之前不许再出站"
    );

    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), Some("B"));
    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), Some("C"));
    assert!(app.queued().is_none(), "队列排空无丢失");
    assert_eq!(app.queued_count(), 0);

    app.note_turn_end();
    assert_eq!(app.next_to_send().as_deref(), None, "队列空 = 无动作");
}

/// Done when 5（Esc 不变）：运行中 Esc/Ctrl+C 照旧 Abort、空闲照旧
/// 确认退出——两条路径都不碰队列，也不碰 composer。
#[test]
fn escape_never_touches_queue() {
    let mut app = App::new();
    start_turn(&mut app, "seed");
    submit(&mut app, "A");
    submit(&mut app, "B");
    type_str(&mut app, "draft");

    // 运行中 Esc = Abort（中断语义照旧），队列与 composer 原样。
    assert_eq!(app.handle_key(key(KeyCode::Esc)), KeyAction::Abort);
    assert_eq!(app.queued_count(), 2);
    assert_eq!(app.queued(), Some("A"));
    assert_eq!(app.input(), "draft", "Esc 不清 composer 文本");

    // Ctrl+C 同路由、同样不碰队列。
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        KeyAction::Abort
    );
    assert_eq!(app.queued_count(), 2);

    // 空闲 Esc 两连（确认 → 退出）：照旧不碰队列。
    app.note_turn_end();
    assert_eq!(app.handle_key(key(KeyCode::Esc)), KeyAction::None);
    assert!(app.confirm_quit());
    assert_eq!(app.queued_count(), 2);
    assert_eq!(app.handle_key(key(KeyCode::Esc)), KeyAction::Quit);
    assert_eq!(app.queued_count(), 2, "退出确认也不碰队列");
}

/// Done when 2（`queued: n` 计数三点）：入队 → 3、召回 → 2、消费 → 1、
/// 排空 → 行消失；渲染面与状态机同一数据源。
#[test]
fn queued_count_tracks_enqueue_recall_consume() {
    let mut app = App::new();
    start_turn(&mut app, "seed");

    for text in ["A", "B", "C"] {
        submit(&mut app, text);
    }
    assert_eq!(app.queued_count(), 3);
    assert!(
        rendered(&app).contains("queued: 3"),
        "入队后显 queued: 3:\n{}",
        rendered(&app)
    );

    app.handle_key(key(KeyCode::Up)); // 召回 C
    assert_eq!(app.queued_count(), 2);
    assert!(
        rendered(&app).contains("queued: 2"),
        "召回后显 queued: 2:\n{}",
        rendered(&app)
    );

    app.note_turn_end();
    app.next_to_send().expect("消费 A");
    assert_eq!(app.queued_count(), 1);
    assert!(
        rendered(&app).contains("queued: 1"),
        "消费后显 queued: 1:\n{}",
        rendered(&app)
    );

    app.note_turn_end();
    app.next_to_send().expect("消费 B");
    assert_eq!(app.queued_count(), 0);
    let text = rendered(&app);
    assert!(!text.contains("queued: "), "队列空不出排队行:\n{text}");
}
