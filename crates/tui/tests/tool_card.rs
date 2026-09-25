//! tool_card — route §4 T3：一次 `ToolCall→ToolStart→ToolResult` 序列折叠
//! 成一张卡（不是三行）、`ToolResult` 后卡片不卡 running；长结果折叠成
//! 「头尾预览 + 隐藏行数」，一个键全部展开/折叠。TestBackend 渲染断言
//! （取样法同 `tests/layout.rs`）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use omenic_tui::app::App;
use omenic_tui::ui;
use web_state::ui_state::AgentEvent;

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

/// 卡头计数（`▸` 只出现在工具卡卡头）。
fn cards(text: &str) -> usize {
    text.matches('▸').count()
}

#[test]
fn tool_sequence_folds_into_one_card() {
    let mut app = App::new();
    app.apply_event(&AgentEvent::ToolCall {
        id: "call-1".to_string(),
        name: "run_bash".to_string(),
        args: serde_json::json!({ "command": "ls -la" }),
    });
    // 序列中途：卡已出现且是 running——不是「每个事件一行/一卡」。
    let mid = screen(&app);
    assert_eq!(cards(&mid), 1, "ToolCall 只该出一张卡:\n{mid}");
    assert!(mid.contains("running"), "卡头要带 running 三态:\n{mid}");

    app.apply_event(&AgentEvent::ToolStart {
        id: "call-1".to_string(),
    });
    let started = screen(&app);
    assert_eq!(cards(&started), 1, "ToolStart 不许渲出第二张卡:\n{started}");
    assert!(
        started.contains("running"),
        "ToolStart 后仍在同张卡上跑:\n{started}"
    );

    app.apply_event(&AgentEvent::ToolResult {
        id: "call-1".to_string(),
        name: "run_bash".to_string(),
        result: "file1\nfile2".to_string(),
    });
    let done = screen(&app);
    assert_eq!(
        cards(&done),
        1,
        "一次 ToolCall→ToolStart→ToolResult 只该有一张卡:\n{done}"
    );
    assert!(done.contains("done"), "ToolResult 后卡头翻到 done:\n{done}");
    assert!(
        !done.contains("running"),
        "ToolResult 后卡片不能卡在 running:\n{done}"
    );
    assert!(done.contains("file1"), "结果正文要显示:\n{done}");

    // 同 id 重复 ToolCall → 新卡（不合并，route §3 边界）。
    app.apply_event(&AgentEvent::ToolCall {
        id: "call-1".to_string(),
        name: "run_bash".to_string(),
        args: serde_json::json!({ "command": "pwd" }),
    });
    let second = screen(&app);
    assert_eq!(cards(&second), 2, "同 id 重复 ToolCall 应是新卡:\n{second}");
}

#[test]
fn long_tool_result_collapses_to_header() {
    let mut app = App::new();
    assert!(!app.tools_expanded(), "默认折叠（展开键尚未按下）");
    let result = (0..20)
        .map(|i| format!("line-{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.apply_event(&AgentEvent::ToolCall {
        id: "call-long".to_string(),
        name: "run_bash".to_string(),
        args: serde_json::json!({ "command": "seq 1 20" }),
    });
    app.apply_event(&AgentEvent::ToolResult {
        id: "call-long".to_string(),
        name: "run_bash".to_string(),
        result,
    });

    let collapsed = screen(&app);
    assert!(collapsed.contains('▸'), "卡头在:\n{collapsed}");
    assert!(
        collapsed.contains("line-00"),
        "头预览保留首行:\n{collapsed}"
    );
    assert!(
        collapsed.contains("line-19"),
        "尾预览保留末行:\n{collapsed}"
    );
    assert!(
        !collapsed.contains("line-10"),
        "长结果默认折叠，中段不许撑爆 dock:\n{collapsed}"
    );
    assert!(
        collapsed.contains("15 lines hidden"),
        "折叠要报隐藏行数（20 行 - 头 3 - 尾 2）:\n{collapsed}"
    );

    // 一个键全部展开（route §3：一个键展开/折叠，Tab 不进 composer）。
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.tools_expanded(), "该键要翻转展开状态");
    let expanded = screen(&app);
    assert!(expanded.contains("line-10"), "展开后中段可见:\n{expanded}");
    assert!(
        !expanded.contains("lines hidden"),
        "展开后不再报隐藏行数:\n{expanded}"
    );

    // 同一键再按 = 全部折回。
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.tools_expanded(), "再按一次折回折叠态");
}
