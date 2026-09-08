# UI 验证约定

## 目标

agent 验证终端 UI 时**不依赖截图**，用结构化断言。

## 工具集

OMP 提供 `ui-validate` skill (managed)，封装以下能力：
- `ratatui::backend::TestBackend` - 渲染到 buffer
- `tui-spec.yaml` - 屏幕契约文件
- `buffer.get(x, y).symbol()` - 断言 cell 内容

## 必做项

1. **TestBackend 渲染**
   - 每个屏幕测试用 `TestBackend::new(W, H)`
   - `terminal.draw(|f| app.render(f))` 渲染
   - 断言 `buffer.get(x, y).symbol()` 包含期望文字

2. **tui-spec.yaml 每屏一个**
   - 放 `specs/tui/<screen>.yaml`
   - 列出坐标 (x, y)、期望文字、focus 状态、关键 state 字段

3. **PR 验证用 TestBackend**
   - 永远先断言 buffer cell 内容，再考虑截图
   - 测试放 `crates/tui/tests/*.rs`，3 秒内跑完

4. **截图只作辅助**
   - 视觉风格/颜色相关才用截图
   - 失败时附图，主验证靠 buffer 断言

## 示例

```yaml
# specs/tui/session-list.yaml
screen: "会话列表"
size: "80x24"

asserts:
  - at: [5, 3]
    contains: "会话列表"
  - at: [10, 8]
    contains: "新会话"
  - focus: "Sessions"
  - state:
      active_session: 0
      streaming: false
```

```rust
// crates/tui/tests/session_list.rs
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[test]
fn session_list_renders_title() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    // terminal.draw(|f| app.render(f)).unwrap();
    let buffer = terminal.backend().buffer();
    // assert!(buffer.get(5, 3).symbol().contains("会话列表"));
}
```

## 不要做

- 只用 `script` 录制肉眼判断
- 不写 tui-spec.yaml 直接 PR
- 测试超过 3 秒跑完（CI 阻塞）
