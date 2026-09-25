//! scroll_follow — route §3 T6 跟尾三态与追尾滑动：上滚即脱钩（流式输出
//! 不拽视口回底）、End / PgDn 落底重挂、`↑N 行` 精确指示、滑动三语义
//! （小追加 snap / 每帧 +3 / 滞后 ≤1 视口）、半页键仲裁（composer 空 /
//! 有文本）。
//!
//! 对照 bug：流式中滚上去被拽回底、重挂失效、滑动整屏蹦、滞后重播多页、
//! composer 有字时 Ctrl+U 吞了输入、窗口计数与物化分叉（历史行错位）。
//! 滑动三语义的测试名照抄 jcode `ui_viewport.rs:1595/1641/1651`。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use omenic_tui::app::{App, KeyAction};
use omenic_tui::scroll::ScrollModel;
use omenic_tui::ui;
use web_state::types::{ChatMessage, MessagePart};
use web_state::ui_state::AgentEvent;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

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

/// 首个包含 `needle` 的行号（0-based）。
fn row_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|line| line.contains(needle))
}

/// 画一帧 80×24（或给定尺寸）取整屏文本（不喂几何——喂几何走 [`sync`]）。
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
    }
}

/// 60 条单行历史（每条 1 行 → 总 60 行；80×24 下 transcript 视口 20 行、
/// max = 40、整页 19、半页 10——下文断言的精确数字都来自这三个值）。
fn scrollable_app(lines: usize) -> App {
    let mut app = App::new();
    let history = (0..lines)
        .map(|i| user_msg(&format!("mid-{i:03}")))
        .collect();
    app.start_session("s-1", history);
    app
}

/// footer 单行文本（model 段做唯一锚，dock 活动行也有 "idle" 不能当锚）。
fn footer_row(app: &App) -> String {
    screen(app, 80, 24)
        .lines()
        .find(|line| line.contains("scroll-model-x"))
        .unwrap_or_else(|| panic!("footer row missing"))
        .to_string()
}

// --- 模型级：追尾滑动三语义（jcode 测试语义照抄，refs ui_viewport.rs:1595+） ---

/// 小追加（≤4 行）直接 snap 贴底：流式小步不整屏蹦。
#[test]
fn tail_follow_small_appends_snap_to_bottom() {
    let mut model = ScrollModel::new();
    model.sync(100, 30);
    assert_eq!(model.offset(), 100, "首帧钉底");
    model.sync(103, 30);
    assert_eq!(model.offset(), 103, "≤4 行追加直接贴底，不起滑动");
    assert!(model.is_following());
}

/// 大块追加：每帧最多 +3 行滑入，且必须收敛到底（不整屏蹦、不卡死）。
#[test]
fn tail_follow_large_append_slides_in_bounded_steps() {
    let mut model = ScrollModel::new();
    model.sync(100, 30);
    model.sync(112, 30);
    let first = model.offset();
    assert!(first < 112, "12 行追加不许一步贴底: {first}");
    assert!(first - 100 <= 3, "首帧步长 ≤3 行: {first}");

    let mut prev = first;
    let mut guard = 0;
    while model.offset() < 112 {
        model.sync(112, 30);
        let step = model.offset() - prev;
        assert!(step <= 3, "每帧步长 ≤3 行: {prev} → {}", model.offset());
        prev = model.offset();
        guard += 1;
        assert!(guard < 50, "滑动必须收敛到底");
    }
    assert_eq!(model.offset(), 112, "滑动终点 = 内容底");
    assert!(model.is_following());
}

/// 巨大块追加：滞后封顶 1 个视口（不重播整屏），随后滑到底。
#[test]
fn tail_follow_caps_lag_to_one_viewport() {
    let mut model = ScrollModel::new();
    model.sync(100, 30);
    model.sync(400, 30);
    assert!(
        model.offset() >= 400 - 30,
        "滞后封顶 1 视口: offset={}",
        model.offset()
    );
    assert!(model.offset() < 400, "封顶后仍从滞后位起滑，不直接贴底");

    let mut guard = 0;
    while model.offset() < 400 {
        model.sync(400, 30);
        guard += 1;
        assert!(guard < 50, "封顶后的滑动必须收敛");
    }
    assert_eq!(model.offset(), 400);
}

/// 内容收缩（落后位 > 新底）：直接 snap 回新底，不倒着播动画。
#[test]
fn tail_follow_backward_motion_snaps() {
    let mut model = ScrollModel::new();
    model.sync(100, 30);
    model.sync(80, 30);
    assert_eq!(model.offset(), 80, "内容收缩直接 snap，不滑动");
    assert!(model.is_following());
}

// --- App 级：脱钩 / 重挂 / 流式不拽回 ---

/// 脱钩不被流式拽回：PgUp 上滚脱钩后，流式追加只让 `↑N` 增长、视口纹丝
/// 不动（bug：滚上去被拽回底）。
#[test]
fn scroll_up_detaches_and_streaming_does_not_drag_back() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);
    assert!(app.viewport().is_following(), "跟尾起步");
    assert_eq!(app.viewport().offset(), 40, "跟尾 = 视口钉内容底");

    assert_eq!(
        app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert!(!app.viewport().is_following(), "上滚即脱钩");
    assert_eq!(app.viewport().offset(), 21, "整页步长 = 视口高 − 1");
    assert_eq!(app.viewport().lift(), Some(19), "指示 = 距底精确行数");

    // 流式追加 5 行（delta 含 4 个换行 → 5 行）。
    app.apply_event(&AgentEvent::AssistantText {
        delta: "s1\ns2\ns3\ns4\ns5".to_string(),
    });
    sync(&mut app, 80, 24);
    assert_eq!(app.viewport().offset(), 21, "流式输出不许拽视口回底");
    assert_eq!(
        app.viewport().lift(),
        Some(24),
        "指示随追加精确 +5（max 涨了、offset 没动）"
    );

    // 视口内容真的停在原窗口：首行还是 mid-021。
    let text = screen(&app, 80, 24);
    assert_eq!(row_of(&text, "mid-021"), Some(0), "脱钩后窗口钉在原位");
}

/// 重挂两路：End 单点跳底恢复跟尾；PgDn 一页落底也恢复跟尾（route §3）。
#[test]
fn end_and_pagedown_to_bottom_reattach_tail() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);

    // End：脱钩态 → 跳底 + 恢复跟尾，指示消失。
    app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(!app.viewport().is_following());
    assert_eq!(
        app.handle_key(key(KeyCode::End, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert!(app.viewport().is_following(), "End 恢复跟随");
    assert_eq!(app.viewport().lift(), None, "回底后指示消失");
    assert_eq!(app.viewport().offset(), app.viewport().max_offset());

    // Ctrl+U 半页脱钩（lift = 10）→ PgDn 一页落底 → 重挂。
    app.handle_key(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert!(!app.viewport().is_following());
    assert_eq!(
        app.viewport().lift(),
        Some(10),
        "半页步长 = 视口 / 2 = 10 行"
    );
    assert_eq!(
        app.handle_key(key(KeyCode::PageDown, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert!(app.viewport().is_following(), "PgDn 落底恢复跟尾");
    assert_eq!(app.viewport().lift(), None);
    sync(&mut app, 80, 24);
    assert_eq!(app.viewport().offset(), app.viewport().max_offset());
}

/// 翻页顶对齐（grok `page_up_selects_top_of_viewport` 语义）：视口顶上移
/// 一页 = 视口高 − 1 行，窗口只物化可见段。
#[test]
fn page_up_moves_viewport_top_by_one_page() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);
    let top_before = app.viewport().offset();
    assert_eq!(top_before, 40, "跟尾已同步到内容底");

    app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(
        app.viewport().offset(),
        top_before - 19,
        "步长 = 视口高 − 1（20 − 1）"
    );

    let text = screen(&app, 80, 24);
    assert_eq!(row_of(&text, "mid-021"), Some(0), "视口顶 = 原顶上移一页");
    assert!(row_of(&text, "mid-040").is_some(), "底行仍在窗口内");
    assert!(row_of(&text, "mid-000").is_none(), "窗口外历史不渲染");
}

/// 半页键（Ctrl+U）仲裁：composer 空 = 半页上滚脱钩；有文本 = 归编辑语义
/// （不滚、不吞字）；PgUp/PgDn 无论 composer 空否都滚动 transcript。
#[test]
fn half_page_key_arbitrates_on_composer() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);

    // composer 空：Ctrl+U = 半页上滚（40 − 10 = 30）+ 脱钩。
    assert_eq!(
        app.handle_key(key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        KeyAction::None
    );
    assert!(!app.viewport().is_following(), "上滚即脱钩");
    assert_eq!(app.viewport().offset(), 30);

    // composer 有文本：半页键归编辑——既不滚动也不吞字（编辑忽略 Ctrl+U）。
    type_str(&mut app, "draft");
    let offset_before = app.viewport().offset();
    let lift_before = app.viewport().lift();
    assert_eq!(
        app.handle_key(key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        KeyAction::None
    );
    assert_eq!(app.input(), "draft", "有文本时 Ctrl+U 不许吞输入");
    assert_eq!(app.viewport().offset(), offset_before, "有文本时不滚动");
    assert_eq!(app.viewport().lift(), lift_before, "脱钩态不变");

    // PgUp：有文本照样滚动（单行输入无翻页语义，不冲突）。
    assert_eq!(
        app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert_eq!(
        app.viewport().offset(),
        offset_before - 19,
        "PgUp 不受 composer 内容影响"
    );
    assert_eq!(app.input(), "draft", "PgUp 不碰输入");
}

/// `↑N 行` 指示：跟尾不出、脱钩出精确数字（内容涨 N 行指示同步涨 N）、
/// 回底消失（bug：指示错数 / 挂着不走）。
#[test]
fn lift_indicator_shows_precise_rows_and_clears() {
    let mut app = scrollable_app(60);
    app.set_model("scroll-model-x");
    sync(&mut app, 80, 24);
    assert!(
        !footer_row(&app).contains('↑'),
        "跟尾时不出指示:\n{}",
        footer_row(&app)
    );

    app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(
        footer_row(&app).contains("↑19 行"),
        "脱钩出精确指示:\n{}",
        footer_row(&app)
    );

    app.apply_event(&AgentEvent::AssistantText {
        delta: "t1\nt2\nt3\nt4\nt5".to_string(),
    });
    sync(&mut app, 80, 24);
    assert!(
        footer_row(&app).contains("↑24 行"),
        "追加 5 行后指示 = 19 + 5:\n{}",
        footer_row(&app)
    );

    app.handle_key(key(KeyCode::End, KeyModifiers::NONE));
    assert!(
        !footer_row(&app).contains('↑'),
        "回底后指示消失:\n{}",
        footer_row(&app)
    );
}

/// 窗口计数与物化一致（count/render 一致性钉）：总行数 `total` 与实际画
/// 出的行必须严丝合缝——首行在第 0 行、末行在第 `total − 1` 行；计数多算
/// （末行提前）或少算（末行被截）都会让本测试变红。内容含长单词硬切、
/// 空行、制表符、工具卡折叠四种断行路径，且在宽/窄两档下列宽断行不同
/// （窄档 total 更大），resize 后拿旧数就会错位。
#[test]
fn window_render_matches_line_count() {
    for width in [80u16, 44u16] {
        let mut app = tricky_app();
        sync(&mut app, width, 24);
        let total = app.viewport().total();
        assert!(total >= 2, "内容至少两行");

        // 屏幕高度 = total + footer 1 + dock 3 → transcript 正好装下全部内容。
        let screen_h = total as u16 + 4;
        sync(&mut app, width, screen_h);
        assert_eq!(
            app.viewport().height(),
            total,
            "视口高度 = 内容全高（列宽 {width}）"
        );

        let text = screen(&app, width, screen_h);
        assert_eq!(
            row_of(&text, "FIRST-ROW"),
            Some(0),
            "width={width}: 首行必须在屏幕顶"
        );
        assert_eq!(
            row_of(&text, "LAST-ROW"),
            Some(total - 1),
            "width={width}: 末行必须在第 total−1 行——计数与物化分叉这里红"
        );
    }
}

/// 切会话：视口复位回底重挂（回填后新会话钉底可见，不残留旧滚动位）。
#[test]
fn switch_session_reanchors_viewport_to_tail() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);
    app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(!app.viewport().is_following());

    app.switch_session("s-2", vec![user_msg("only-in-new")]);
    assert!(app.viewport().is_following(), "切台回底重挂");
    assert_eq!(app.viewport().lift(), None, "切台后指示消失");
    sync(&mut app, 80, 24);
    let text = screen(&app, 80, 24);
    assert_eq!(row_of(&text, "only-in-new"), Some(0), "新会话钉底可见");
}

/// 断行路径齐全的样本：长单词硬切、空行、制表符、工具卡折叠（头 + 隐藏
/// 行数 + 尾，尾行带 `LAST-ROW` 锚）。
fn tricky_app() -> App {
    let mut app = App::new();
    app.start_session(
        "s-1",
        vec![
            user_msg("FIRST-ROW marker"),
            user_msg(&"x".repeat(100)),
            user_msg("alpha beta\n\ngamma delta"),
            user_msg("col\tcol\tvalue"),
        ],
    );
    app.apply_event(&AgentEvent::ToolCall {
        id: "c1".to_string(),
        name: "run_bash".to_string(),
        args: serde_json::json!({ "command": "ls" }),
    });
    app.apply_event(&AgentEvent::ToolResult {
        id: "c1".to_string(),
        name: "run_bash".to_string(),
        result: long_result(),
    });
    app
}

/// 15 行工具结果（> 折叠阈值 6）：折叠渲染 = 头 3 + 隐藏行数 + 尾 2，
/// 末行即整个 transcript 的末行。
fn long_result() -> String {
    let mut lines: Vec<String> = (0..14).map(|i| format!("res-line-{i:02}")).collect();
    lines.push("LAST-ROW marker tail".to_string());
    lines.join("\n")
}
