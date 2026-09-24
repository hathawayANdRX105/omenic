//! layout — route §4 `dock_layout_fits`：80×24 与 44×20 两档下 dock 四件套
//! （活动 / 排队 / composer / 按键提示）俱全且顺序不乱、transcript 在上、
//! hints 压在最底一行。TestBackend 渲染，无真终端。

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use omenic_tui::app::{App, KeyAction};
use omenic_tui::ui;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

/// 首个包含 `needle` 的行号（0-based）。
fn row_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|line| line.contains(needle))
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 满配 dock：一轮在跑（活动 running + transcript 有 user 消息）、
/// 一条排队 prompt、composer 留着未提交文本。
fn loaded_app() -> App {
    let mut app = App::new();
    type_str(&mut app, "hello");
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert_eq!(app.next_to_send().as_deref(), Some("hello"));
    type_str(&mut app, "second");
    assert_eq!(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert_eq!(app.queued(), Some("second"), "运行中提交进排队");
    type_str(&mut app, "world"); // composer 保留未提交文本
    app
}

#[test]
fn dock_layout_fits_80x24_and_44x20() {
    for (width, height) in [(80u16, 24u16), (44u16, 20u16)] {
        let app = loaded_app();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| ui::draw(frame, &app))
            .expect("draw must fit without panicking");

        let text = buffer_text(terminal.backend().buffer());
        let row = |needle: &str| {
            row_of(&text, needle).unwrap_or_else(|| {
                panic!("{width}x{height}: `{needle}` missing from buffer:\n{text}")
            })
        };

        // 四件套齐全且自上而下：transcript → 活动 → 排队 → composer → hints。
        let transcript = row("❯ hello");
        let activity = row("● running");
        let queued = row("queued: second");
        let composer = row("> world");
        let hints = row("history");

        assert!(transcript < activity, "transcript 在 dock 之上:\n{text}");
        assert!(activity < queued, "活动行在排队行之上:\n{text}");
        assert!(queued < composer, "排队行在 composer 之上:\n{text}");
        assert!(composer < hints, "composer 在按键提示之上:\n{text}");
        // dock 恒压底：hints 是屏幕最后一行（44×20 下也不被挤掉）。
        assert_eq!(
            hints,
            text.lines().count().saturating_sub(1),
            "hints 必须在最底一行:\n{text}"
        );
    }
}
