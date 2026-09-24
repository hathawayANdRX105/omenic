//! footer — route §4 T3：footer 只许出现已核字段——开工先核结论（`stats.
//! summary` 与消息 usage 均无 per-context 占用数据）→ 空闲段只有
//! `model · run 状态`，任何 context 百分比 / token / 计费字段都是编造，
//! 必须让本测试变红。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use omenic_tui::app::App;
use omenic_tui::ui;

/// TestBackend 缓冲 → 逐行文本（同 dsh `tests/chat_flow.rs` 的取样法）。
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

/// 渲染一帧 80×24 并取整屏文本。
fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw must fit without panicking");
    buffer_text(terminal.backend().buffer())
}

/// 含 `needle` 的那一行（footer 是唯一带 model 的行）。
fn footer_row<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line containing `{needle}` in:\n{text}"))
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

#[test]
fn footer_shows_only_verified_fields() {
    let mut app = App::new();
    app.set_model("test-model-x");

    // 运行中：`model · 耗时 · Esc 中断提示`（route §3 footer 契约）。
    type_str(&mut app, "hello");
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        omenic_tui::app::KeyAction::None
    );
    assert_eq!(app.next_to_send().as_deref(), Some("hello"));
    let running = screen(&app);
    let running_footer = footer_row(&running, "test-model-x");
    assert!(
        running_footer.contains("ms"),
        "运行中出耗时段:\n{running_footer}"
    );
    assert!(
        running_footer.contains("esc abort"),
        "运行中出 Esc 中断提示:\n{running_footer}"
    );

    // 空闲：`model · run 状态`；未核字段（context 占用/百分比/token/费用）
    // 一律不许出现——出现即「footer 编造数据」的 bug。
    app.note_turn_end();
    app.set_in_flight(0);
    let idle = screen(&app);
    let idle_footer = footer_row(&idle, "test-model-x");
    assert!(
        idle_footer.contains("idle"),
        "空闲出 run 状态:\n{idle_footer}"
    );
    for forbidden in ["%", "context", "token", "$"] {
        assert!(
            !idle.contains(forbidden),
            "footer 只许出已核字段，`{forbidden}` 是编造数据:\n{idle}"
        );
    }

    // 已核字段换档：`stats.summary` 的半开 run 计数 → `{n} in flight`。
    app.set_in_flight(1);
    let inflight = screen(&app);
    let inflight_footer = footer_row(&inflight, "test-model-x");
    assert!(
        inflight_footer.contains("1 in flight"),
        "run 状态来自 stats.summary:\n{inflight_footer}"
    );
    assert!(!inflight_footer.contains('%'), "照样不许有百分比");
}
