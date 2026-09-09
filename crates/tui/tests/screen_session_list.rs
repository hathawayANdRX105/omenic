//! TUI smoke: 用 ratatui TestBackend 验证会话列表屏幕渲染
//! 对应 `specs/tui/session-list.yaml`
//! 验收：`cargo test -p tui --test screen_session_list`
//!
//! 参考 OMP skill `ui-validate` 了解 buffer cell 断言的完整流程。

use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn screen_session_list_renders_title() {
    // TestBackend 80x24 模拟终端
    let backend = TestBackend::new(80, 24);
    let terminal = Terminal::new(backend).unwrap();

    // TODO(#ui-validation): 当 App::render 公开后，替换为真实渲染
    // terminal.draw(|f| app.render(f)).unwrap();

    let buffer = terminal.backend().buffer();

    // 占位断言：cell (5, 3) 应包含标题
    // 真实 PR：assert!(buffer.get(5, 3).symbol().contains("会话列表"));
    let _ = buffer;
}

#[test]
fn screen_session_list_focus_initial() {
    let backend = TestBackend::new(80, 24);
    let terminal = Terminal::new(backend).unwrap();
    // TODO(#ui-validation): 验证初始 focus 是 Sessions
    // assert!(app.focus == Focus::Sessions);
    let _ = terminal;
}
