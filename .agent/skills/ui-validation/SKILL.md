---
name: ui-validation
description: 'Project UI validation conventions for web (dioxus) and TUI (ratatui) PRs. Enforces data-testid + ui-spec.yaml for web pages, TestBackend + tui-spec.yaml for terminal screens. Replaces blind pixel screenshots with structured role/coord assertions.'
license: MIT
---

# UI 验证约定

## 目标

agent 验证 UI 时**不依赖截图**，用结构化断言。

## Web 流程（dioxus）

1. **交互元素加 `data-testid`**：所有 `button` / `input` / `tab` 用 `name` 属性作为 ID
2. **容器/标签加 ARIA**：`role="tab"` + `aria_selected="{i == active}"` + `aria-label`
3. **每页一个 `specs/ui/<page>.yaml`** 契约
4. **PR smoke 用 `tab.ariaSnapshot()`**：断言 role+name+testid
5. **截图仅作辅助**：视觉风格/品牌相关才用

## TUI 流程（ratatui）

1. **每屏一个 `specs/tui/<screen>.yaml`** 契约（坐标+文字+focus+state）
2. **测试用 `ratatui::backend::TestBackend`**：渲染到 buffer，断言 cell 内容
3. **测试放 `crates/tui/tests/*.rs`**：3 秒内跑完
4. **截图（`script` 录制）仅作辅助**

## Web Spec 格式（`specs/ui/<page>.yaml`）

```yaml
page: "/admin/overview"
name: "管理总览"

elements:
  - role: heading
    name: "系统状态"
    required: true
    testid: "heading-system-status"

  - role: tab
    name: "用户管理"
    testid: "tab-users"
    on_select:
      - role: table
        name: "用户列表"
        min_rows: 1
        testid: "table-users"

  - role: button
    name: "新建渠道"
    testid: "btn-new-channel"
    action: click
    expect_navigate: "/admin/channels/new"
```

## TUI Spec 格式（`specs/tui/<screen>.yaml`）

```yaml
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

actions:
  - key: "Tab"
    expect_focus: "Input"
```

## Web 验证示例

```rust
let snap = tab.ariaSnapshot().await?;
assert!(snap.contains("role=\"tab\" name=\"用户管理\" data-testid=\"tab-users\""));
tab.run("document.querySelector('[data-testid=\"tab-users\"]').click()").await?;
let snap2 = tab.ariaSnapshot().await?;
assert!(snap2.contains("role=\"table\" name=\"用户列表\""));
```

## TUI 验证示例

```rust
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn screen_session_list_renders_title() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    // terminal.draw(|f| app.render(f)).unwrap();
    let buffer = terminal.backend().buffer();
    assert!(buffer.get(5, 3).symbol().contains("会话列表"));
}
```

## 禁做项

- ❌ 只用截图肉眼判断（agent 看不清/看不全）
- ❌ 用 CSS class 选择器（会因样式调整失效）
- ❌ 不写 `*spec.yaml` 直接 PR
- ❌ TUI 测试超过 3 秒跑完

## 共享 skill

OMP `ui-validate` skill（managed-skills）封装上述流程，触发词：ui-spec、tui-spec、ariaSnapshot、data-testid、TestBackend、validate ui、tab verification。

## 相关文件

- Web 契约：`specs/ui/*.yaml`
- TUI 契约：`specs/tui/*.yaml`
- 已加 testid 的组件：`crates/web/ui-components/src/`
- TUI 测试：`crates/tui/tests/screen_*.rs`
