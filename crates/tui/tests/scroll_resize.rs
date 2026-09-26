//! scroll_resize — route §3 T6「滚动位置在 resize 后 clamp 不越界」：视口
//! 变高 / 内容骤减后 offset 夹紧不越界、渲染不 panic 不白屏（bug：resize
//! 越界 panic / 白屏），以及内容总行数随列宽重数——窄列多断行，窗口拿旧
//! 计数就会错位。
//!
//! 模型侧不碰真终端（`ScrollModel::sync` 直接喂几何），渲染侧走 TestBackend。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use omenic_tui::app::App;
use omenic_tui::scroll::ScrollModel;
use omenic_tui::ui;
use web_state::types::{ChatMessage, MessagePart};
use web_state::ui_state::AgentEvent;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
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

/// 画一帧取整屏文本（不喂几何——喂几何走 [`sync`]）。
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

/// 60 条单行历史（80×24 下视口 20 行、max = 40）。
fn scrollable_app(lines: usize) -> App {
    let mut app = App::new();
    let history = (0..lines)
        .map(|i| user_msg(&format!("mid-{i:03}")))
        .collect();
    app.start_session("s-1", history);
    app
}

/// 脱钩后屏幕变高：视口吃掉更多内容 → max 骤缩，offset 必须 clamp 进新
/// 边界（bug：旧 offset > 新 max → 越界 / 渲染错位），且窗口从新顶开始。
#[test]
fn viewport_offset_clamps_when_viewport_grows() {
    let mut app = scrollable_app(60);
    sync(&mut app, 80, 24);
    app.handle_key(key(KeyCode::PageUp, KeyModifiers::NONE));
    assert_eq!(app.viewport().offset(), 21, "脱钩在 80×24 的位置");
    assert!(!app.viewport().is_following());

    // 80×60：transcript 视口 56 行 → max = 60 − 56 = 4（原 offset 21 越界）。
    sync(&mut app, 80, 60);
    assert_eq!(app.viewport().max_offset(), 4);
    assert_eq!(
        app.viewport().offset(),
        4,
        "resize 后 offset clamp 进新边界，不越界"
    );
    assert!(app.viewport().offset() <= app.viewport().max_offset());

    // 渲染不 panic，且窗口从 clamp 后的新顶开始（mid-004 在第 0 行）。
    let text = screen(&app, 80, 60);
    assert_eq!(row_of(&text, "mid-004"), Some(0), "窗口按 clamp 后的顶渲染");
    assert!(
        row_of(&text, "history").is_some(),
        "不许白屏：dock 按键提示还在:\n{text}"
    );
}

/// 模型侧的几何边界矩阵：内容骤减、视口骤增、0 高/0 内容——一律夹紧到
/// 合法区间，不 panic、不下溢。
#[test]
fn model_offset_stays_clamped_across_geometry_jumps() {
    let mut model = ScrollModel::new();
    model.sync(500, 20);
    assert_eq!(model.offset(), 480, "跟尾钉底");
    model.page_up();
    assert!(!model.is_following());

    // 内容骤减（500 → 30 行）：clamp 到新 max = 10。
    model.sync(30, 20);
    assert_eq!(model.offset(), 10, "内容骤减 clamp 到新底");
    assert!(model.offset() <= model.max_offset());

    // 视口骤增（20 → 1000 行，比内容还高）：max = 0。
    model.sync(30, 1000);
    assert_eq!(model.offset(), 0, "视口高于内容时回顶（= 底）");
    assert_eq!(model.view_top(30, 1000), 0);

    // 0 高 / 0 内容：恒 0，不 panic、不下溢。
    model.sync(0, 0);
    assert_eq!(model.offset(), 0);
    assert_eq!(model.view_top(0, 0), 0);

    // 脱钩态 + 视口骤增同样夹紧。
    let mut detached = ScrollModel::new();
    detached.sync(500, 20);
    detached.page_up();
    detached.sync(30, 1000);
    assert_eq!(detached.offset(), 0, "脱钩位也 clamp 进新边界");
    assert_eq!(detached.view_top(30, 1000), 0);
}

/// 多档 resize × 流式 × 翻页循环：每一步都 clamp 不越界、不 panic，
/// dock 恒压底（不白屏、不塌布局）。
#[test]
fn resize_streaming_cycle_never_panics_or_blanks() {
    let mut app = scrollable_app(30);
    let mut flip = false;
    for (width, height) in [(44u16, 12u16), (100, 40), (60, 13), (80, 24), (44, 20)] {
        sync(&mut app, width, height);
        app.apply_event(&AgentEvent::AssistantText {
            delta: "chunk line one\nchunk line two".to_string(),
        });
        flip = !flip;
        let code = if flip {
            KeyCode::PageUp
        } else {
            KeyCode::PageDown
        };
        app.handle_key(key(code, KeyModifiers::NONE));
        sync(&mut app, width, height);
        assert!(
            app.viewport().offset() <= app.viewport().max_offset(),
            "{width}x{height}: sync 后 offset 必须 clamp 进 [0, max]"
        );

        let text = screen(&app, width, height);
        assert_eq!(
            row_of(&text, "history"),
            Some((height - 1) as usize),
            "{width}x{height}: hints 恒在最底一行（不白屏、不塌布局）:\n{text}"
        );
    }
}

/// 内容总行数随列宽重数：同一内容，窄列断行更多 → total 变大；resize 后
/// 拿旧计数开窗就会错位（bug：窗口错位 / 底行被截）。
#[test]
fn transcript_total_recounts_after_width_change() {
    let mut app = App::new();
    let mut history = vec![user_msg(&"x".repeat(60))];
    for i in 0..9 {
        history.push(user_msg(&format!("mid-{i:03}")));
    }
    app.start_session("s-1", history);

    sync(&mut app, 80, 24);
    let wide = app.viewport().total();
    sync(&mut app, 44, 24);
    let narrow = app.viewport().total();

    // 60 字符单词：80 列下 "❯ " + 78 可用宽一行放下（1 行），44 列下
    // 可用 42 列硬切成 2 行；其余 9 条两档都是 1 行。
    assert_eq!(wide, 10, "宽列：同一内容行数少");
    assert_eq!(narrow, 11, "窄列：同一内容多断 1 行");
    assert!(narrow > wide, "总行数必须随列宽重算");
}
