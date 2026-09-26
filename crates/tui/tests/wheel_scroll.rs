//! wheel_scroll — route §3 T7 鼠标滚轮契约：入队不整段跳、3/2/1 缓出分拍
//! 前进、16ms 冲刷下限、12ms 判窗（同批归并 / 出窗换向清残留）、下滚落底
//! 重挂且 `↑N 行` 消失、四常数快照、mouse tracking 进出对称（退出后终端
//! 拖选不失灵）、linear 路径零捕获。
//!
//! 对照 bug：一次滚轮事件视口整段跳、节流写错（冲刷过快/过慢）、换向先播
//! 一段反向余量、下滚到底不重挂、退出后鼠标选择失灵（还原序列漏
//! DisableMouseCapture）、linear 误启捕获、常数静默漂移。
//!
//! [`App`] 是无 IO 纯状态机（滚轮事件带注入的 `Instant` 进、队列/视口出），
//! 这些断言在 CI 无 TTY 环境下照样成立。

use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use omenic_tui::app::{
    App, REDRAW_CADENCE_MS, STREAM_GAP_MS, WHEEL_LINES_PER_TICK, WHEEL_QUEUE_MAX,
    WHEEL_TICK_DETECT_MAX_MS,
};
use omenic_tui::scroll::ScrollModel;
use omenic_tui::ui;
use web_state::types::{ChatMessage, MessagePart};

/// TestBackend 缓冲 → 逐行文本（同 `tests/layout.rs` 的取样法）。
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

/// 画一帧 80×24 取整屏文本（不喂几何——喂几何走 [`sync`]）。
fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw must fit without panicking");
    buffer_text(terminal.backend().buffer())
}

/// 每帧几何同步（事件循环 draw 前的那一步，测试走同一入口）。
fn sync(app: &mut App, width: u16, height: u16) {
    ui::sync_viewport(app, Rect::new(0, 0, width, height));
}

fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        id: format!("m-{text}"),
        role: "user".to_string(),
        content: text.to_string(),
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![MessagePart::Text(text.to_string())],
        timestamp: String::new(),
        ts_epoch_ms: 0,
        attachments: vec![],
    }
}

/// 60 条单行历史（80×24 下 transcript 视口 20 行、max = 40——与
/// `tests/scroll_follow.rs` 同一把尺子）。
fn scrollable_app(lines: usize) -> App {
    let mut app = App::new();
    let history = (0..lines)
        .map(|i| user_msg(&format!("mid-{i:03}")))
        .collect();
    app.start_session("s-1", history);
    app
}

/// footer 单行文本（model 段做唯一锚）。
fn footer_row(app: &App) -> String {
    screen(app, 80, 24)
        .lines()
        .find(|line| line.contains("wheel-model-x"))
        .unwrap_or_else(|| panic!("footer row missing"))
        .to_string()
}

fn read_src(rel: &str) -> String {
    std::fs::read_to_string(format!("{}/src/{rel}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("read src/{rel}: {e}"))
}

// --- 缓出冲刷：不整段跳、分 tick 前进 ---

/// 上滚入队脱钩、按 3/2/1 缓出分拍推进（bug：一次滚轮事件整段跳 / 冲刷
/// 节流失效）——三个 notch 只入队不动视口，首拍 3 行、16ms 内空转、随后
/// 3/2/1 走完整段，`↑N 行` 指示精确。
#[test]
fn wheel_up_enqueues_and_eases_out_across_ticks() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);
    assert!(app.viewport().is_following(), "跟尾起步");
    assert_eq!(app.viewport().offset(), 40, "跟尾 = 视口钉内容底");

    // 三个 notch 连发（同向、同批）：只入队，不推进视口。
    let t0 = Instant::now();
    for _ in 0..3 {
        app.wheel_tick(-1, t0);
    }
    assert_eq!(app.wheel_queue(), -3 * 3, "每 notch 入队 3 行");
    assert_eq!(app.viewport().offset(), 40, "入队不直接动视口");

    // 首刷：余量 9 ≥ 6 → 3 行（缓出），且无冲刷历史、立刻执行。
    assert_eq!(app.drain_wheel(t0), -3, "首拍 3 行，不一步到 31");
    assert_eq!(app.viewport().offset(), 37);
    assert!(!app.viewport().is_following(), "上滚即脱钩");
    assert_eq!(app.wheel_queue(), -6, "余量留待下拍");

    // 16ms 下限：不到 REDRAW_CADENCE_MS 的 drain tick 必须空转。
    let early = t0 + Duration::from_millis(REDRAW_CADENCE_MS - 1);
    assert_eq!(app.drain_wheel(early), 0, "节流未到点不冲刷");
    assert_eq!(app.viewport().offset(), 37, "空转不推进视口");

    // 到点继续缓出：6 → 3 行、3 → 2 行、1 → 1 行，分拍走完整段。
    let mut now = t0 + Duration::from_millis(REDRAW_CADENCE_MS);
    assert_eq!(app.drain_wheel(now), -3);
    assert_eq!(app.viewport().offset(), 34, "第二拍再 3 行");
    now += Duration::from_millis(REDRAW_CADENCE_MS);
    assert_eq!(app.drain_wheel(now), -2, "余量 3 → 2 行");
    assert_eq!(app.viewport().offset(), 32);
    now += Duration::from_millis(REDRAW_CADENCE_MS);
    assert_eq!(app.drain_wheel(now), -1, "余量 1 → 1 行");
    assert_eq!(app.viewport().offset(), 31, "三 notch × 3 行 = 9 行全到账");
    assert_eq!(app.wheel_queue(), 0, "队列清空");
    assert_eq!(app.viewport().lift(), Some(9), "指示 = 距底精确行数");
    // 队列空后再冲刷是空转。
    now += Duration::from_millis(REDRAW_CADENCE_MS);
    assert_eq!(app.drain_wheel(now), 0);
    assert_eq!(app.viewport().offset(), 31);
}

/// 下滚落底 reattach、`↑N 行` 消失（bug：滚到底不重挂 / 指示挂着不走）。
#[test]
fn wheel_down_to_bottom_reattaches_and_clears_lift() {
    let mut app = scrollable_app(60);
    app.set_model("wheel-model-x");
    sync(&mut app, 80, 24);

    // 上滚一个 notch：脱钩 3 行，指示出精确数字。
    let t = Instant::now();
    app.wheel_tick(-1, t);
    assert_eq!(app.drain_wheel(t), -2);
    assert_eq!(
        app.drain_wheel(t + Duration::from_millis(REDRAW_CADENCE_MS)),
        -1
    );
    assert_eq!(app.viewport().offset(), 37);
    assert!(!app.viewport().is_following(), "上滚即脱钩");
    assert!(
        footer_row(&app).contains("↑3 行"),
        "脱钩出精确指示:\n{}",
        footer_row(&app)
    );

    // 换向下滚（出判窗 → 清残留后入队），逐拍向底滑。
    let mut now = t + Duration::from_millis(50);
    let mut guard = 0;
    while !app.viewport().is_following() {
        app.wheel_tick(1, now);
        app.drain_wheel(now);
        now += Duration::from_millis(REDRAW_CADENCE_MS);
        guard += 1;
        assert!(guard < 20, "下滚必须在有限拍内落底重挂");
    }
    assert_eq!(
        app.viewport().offset(),
        app.viewport().max_offset(),
        "落 max = 视口钉底"
    );
    assert_eq!(app.viewport().lift(), None, "回底后指示消失");
    assert!(
        !footer_row(&app).contains('↑'),
        "回底后指示消失:\n{}",
        footer_row(&app)
    );
}

// --- 常数快照：漂移即红 ---

/// 四常数 + 队列上限 = route §3 T7 定值（grok-build `mouse.rs:63-76`
/// 出处对齐；任何一位改动都先红这里）。
#[test]
fn wheel_constants_snapshot() {
    assert_eq!(WHEEL_LINES_PER_TICK, 3, "每 tick 行数漂移");
    assert_eq!(REDRAW_CADENCE_MS, 16, "冲刷节流漂移");
    assert_eq!(STREAM_GAP_MS, 80, "流间隔漂移");
    assert_eq!(WHEEL_TICK_DETECT_MAX_MS, 12, "tick 判窗漂移");
    assert_eq!(WHEEL_QUEUE_MAX, 128, "队列防雪崩上限漂移");
}

/// 队列防雪崩（bug：高分 wheel 事件密度让待出行数无限堆积）：同向狂打
/// 500 notch，队列 clamp 在 ±128 内。
#[test]
fn wheel_queue_clamps_to_bound_under_flood() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);
    let mut now = Instant::now();
    for _ in 0..500 {
        app.wheel_tick(-1, now);
        now += Duration::from_millis(20); // 同向：出判窗也不清残留
    }
    assert_eq!(app.wheel_queue(), -WHEEL_QUEUE_MAX, "队列 clamp ±128");
}

/// 12ms 判窗（bug：换向先播一段反向余量）：窗内反向 = 同批抖动归并对消；
/// 出窗反向 = 换向新流，先清残留再入队（反转即刻生效）。
#[test]
fn wheel_tick_merges_in_window_and_clears_stale_flipped_backlog() {
    let mut app = App::new();
    let t0 = Instant::now();
    for _ in 0..3 {
        app.wheel_tick(-1, t0);
    }
    assert_eq!(app.wheel_queue(), -9);

    // 判窗边界（恰好 12ms）内的反向：仍属同一批次 → 归并对消。
    app.wheel_tick(1, t0 + Duration::from_millis(WHEEL_TICK_DETECT_MAX_MS));
    assert_eq!(app.wheel_queue(), -6, "窗内反向归并 = -9 + 3");

    // 出判窗的反向：清掉残留上滚余量后重新入队。
    app.wheel_tick(-1, t0 + Duration::from_millis(30));
    assert_eq!(app.wheel_queue(), -3, "出窗换向清残留（-6 → 0）再入队");
    app.wheel_tick(1, t0 + Duration::from_millis(50));
    assert_eq!(app.wheel_queue(), 3, "同理：下滚不背着上滚余量起步");
}

// --- 还原对称：mouse tracking 的开与拆 ---

/// 进出序列字节级对称（bug：退出后终端鼠标选择失灵 = 还原缺
/// DisableMouseCapture）：进 = alt screen + EnableMouseCapture；还原 =
/// DisableMouseCapture → 离 alt screen → 显光标（正常 leave 与 panic hook
/// 共用同一条 [`restore_terminal`] 出口）。
#[test]
fn termguard_sequences_toggle_mouse_capture() {
    let mut enter = Vec::new();
    omenic_tui::termguard::enter_sequence(&mut enter).expect("enter sequence");
    let mut expected_enter = Vec::new();
    crossterm::execute!(
        expected_enter,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .expect("expected enter");
    assert_eq!(enter, expected_enter, "进屏必须启用鼠标捕获");

    let mut leave = Vec::new();
    omenic_tui::termguard::restore_sequence(&mut leave).expect("restore sequence");
    let mut expected_leave = Vec::new();
    crossterm::execute!(
        expected_leave,
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    )
    .expect("expected leave");
    assert_eq!(
        leave, expected_leave,
        "还原序列必须先拆 mouse tracking 再离屏"
    );

    // 两条还原路径都打到 restore_terminal（唯一含拆捕获的还原口）：
    // ① 正常退出（CrosstermOps::leave）；② panic hook 的 restore。
    let src = read_src("termguard.rs");
    assert!(
        src.contains("fn leave(&mut self) -> io::Result<()> {\n        restore_terminal()"),
        "CrosstermOps::leave 必须走 restore_terminal"
    );
    assert!(
        src.contains("let _ = restore_terminal();"),
        "panic hook 的 restore 必须走 restore_terminal"
    );
}

/// linear 路径零捕获（bug：linear 误启鼠标捕获 / 捕获开关散落在非
/// enhanced 路径）：termguard 是 enhanced 专属进出封装，渲染链与探针判据
/// 都碰不到捕获；捕获开关收敛在 termguard 一处。
#[test]
fn linear_path_never_enables_mouse_capture() {
    for file in ["linear.rs", "mode.rs", "probe.rs", "lib.rs"] {
        let src = read_src(file);
        assert!(!src.contains("MouseCapture"), "{file} 不许碰鼠标捕获");
        assert!(!src.contains("TermGuard"), "{file} 不许进 termguard");
    }
    // 事件循环只消费事件，不负责开捕获（开捕获收敛在 termguard）。
    assert!(
        !read_src("app.rs").contains("MouseCapture"),
        "捕获开关不许散进事件循环"
    );
    let tg = read_src("termguard.rs");
    assert_eq!(tg.matches("EnableMouseCapture").count(), 1, "启用仅一处");
    assert_eq!(tg.matches("DisableMouseCapture").count(), 1, "拆除仅一处");
}

// --- ScrollModel::scroll_lines 单测 ---

/// `scroll_lines` 严格镜像翻页语义（bug：滚轮不脱钩 / 落底不重挂 /
/// clamp 越界）：上滚先脱钩再 clamp 到顶不越界；下滚 clamp 到底、落 max
/// 即重挂；delta = 0 不动。
#[test]
fn scroll_lines_mirrors_page_semantics_and_clamps() {
    let mut model = ScrollModel::new();
    model.sync(100, 30); // 跟尾钉底：offset = max = 70
    assert_eq!(model.offset(), 70);

    // 上滚：先脱钩、再 clamp（镜像 page_up）。
    model.scroll_lines(-3);
    assert!(!model.is_following(), "上滚即脱钩");
    assert_eq!(model.offset(), 67);
    model.scroll_lines(-10_000);
    assert_eq!(model.offset(), 0, "上滚 clamp 到顶");
    model.scroll_lines(-5);
    assert_eq!(model.offset(), 0, "已到顶再上滚：单点 clamp 位置不变");

    // 下滚：clamp 到 max、落 max 即恢复跟尾（镜像 page_down）。
    model.scroll_lines(5);
    assert_eq!(model.offset(), 5);
    assert!(!model.is_following(), "未落底不重挂");
    model.scroll_lines(10_000);
    assert_eq!(model.offset(), model.max_offset(), "下滚 clamp 到底");
    assert!(model.is_following(), "落 max 即重挂");
    assert_eq!(model.lift(), None, "回底后指示消失");

    // delta = 0：位置与跟尾态都不动。
    model.scroll_lines(0);
    assert_eq!(model.offset(), model.max_offset());
    assert!(model.is_following());
}
