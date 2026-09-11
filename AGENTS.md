# Agent 行为规范

## Issue/PR 创建

创建 issue/PR 前必须读 `.github/ISSUE_TEMPLATE/` 或 `.github/PULL_REQUEST_TEMPLATE.md`，然后通过已安装 gate 拦截的 `gh` 创建，禁止绕过 gate。

```bash
# 安装/更新拦截门
gate init

# issue(正文按 .github/ISSUE_TEMPLATE/ 下模板)
gh issue create --title "..." --body "..." --label <epic|sub|...>

# PR(正文按 .github/PULL_REQUEST_TEMPLATE.md)
gh pr create --title "..." --body "..." --head <branch>
```

gate 自动做创建前校验(规则在 `.githooks/spec/`)+ 创建后现实校验，FAIL 拒绝创建。

## Demo 验证沙盒

验证 issue/PR 流程、gh-gate 拦截、规则改动时，**不要在本仓库(omenic)直接创建 demo issue/PR**，使用专用沙盒：

- 仓库：https://github.com/hathawayANdRX105/demo-githooks(本地 `~/projects/demo-githooks`)
- 用途：验证 epic/sub/PR 链路、checkbox 强制、双向关联(GT-04b)、审查强制等，避免污染 omenic
- .githooks 与 omenic 同步；规则改动后先在此仓库验证，再同步到其他项目(deskctl / new-api)

规则文件同步流程：改动 omenic `.githooks/` → 复制到 demo-githooks / deskctl / new-api 并提交。

## TUI 验证约定（Ratatui）

agent 验证终端 UI 时**不依赖截图**，用 OMP `ui-validate` skill 的结构化断言。详见项目级 skill `.agent/skills/ui-validation/SKILL.md`：

- 用 `ratatui::backend::TestBackend` 渲染到 buffer，断言 cell 内容
- 每屏一个 `specs/tui/<screen>.yaml` 契约，列出坐标/文字/状态
- 测试放 `crates/tui/tests/*.rs`，3 秒内跑完
- 截图（`cat` 输出/`script` 录制）仅作辅助，失败时附带

禁区：只用截图肉眼判断、不写 tui-spec.yaml 直接 PR。

## Web（oi-web）启动与样式缺失排查

**症状**：Web UI 打开后是「裸文本」——没有暗色主题、没有卡片/气泡样式，文字堆在一起（如 "搜索会话⌘K / 工作区 / Spaces" 全是素文本），但 JS 功能正常（能发消息、minimap 逻辑在跑）。这是**二进制里嵌进去的 CSS 为空**，不是前端没渲染、也不是没合并代码。

**根因（三个坑，按发生顺序）**：

1. `crates/web/build.rs` 需要 `crates/web/node_modules` 里的 `@tailwindcss/cli`，把 `assets/tailwind-input.css` 生成 `tailwind.gen.css`（写到 `OUT_DIR`），再由 `lib.rs` 的 `include_str!` 嵌进二进制。**没装依赖时 npx 回退拉不到 `tailwindcss` 包本体（输入里 `@import "tailwindcss"` 解析失败），build.rs 会静默写一个 0 字节 CSS 兜底**（见 build.rs 注释 "UI would simply be unstyled"）。`node_modules` 不在 git 里，CI / 新克隆 / 新 worktree 都没装 → 全是空 CSS。
2. `cargo build` 只把 `assets/tailwind-input.css` 和 `src` 标了 `rerun-if-changed`，**装了 `node_modules` 之后它感知不到，不会自动重跑 build.rs**，旧的空 CSS 会一直沿用。必须 `touch crates/web/assets/tailwind-input.css` 再编译才重新生成。
3. `cargo run` 起的是**编译前那一刻的二进制**；编译完成后产物已换，但进程还在跑旧的（进程启动时间比二进制 mtime 还早）。必须杀掉重启。

**正确启动序列**（在仓库根目录）：

```bash
cd crates/web && npm install            # 确保 node_modules/.bin/tailwindcss 存在
# 可选：手动确认能生成非空 CSS（约 40KB）
#   node_modules/.bin/tailwindcss -i assets/tailwind-input.css -o /tmp/t.css
cd <仓库根>
touch crates/web/assets/tailwind-input.css   # 强制重跑 build.rs
cargo build --bin oi-web                  # 不是 -p web；二进制在 bin/web（oi-web）
strings target/debug/oi-web | grep -c -- --color-accent   # 应 > 0（如 30）
pkill -x oi-web; sleep 1
nohup ./target/debug/oi-web > /tmp/oi-web.log 2>&1 &      # 起在默认 8026（PORT 可覆盖）
curl -s localhost:8026/ | grep -c -- --color-accent        # 页面里应 > 0
```

**快速诊断**：`curl -s localhost:<port>/ | grep -c -- --color-accent` 返回 0 = 跑的是空 CSS 的旧二进制，按上面序列重建重启。浏览器记得**硬刷新**（LiveView 按 origin 缓存 wasm，换端口/换二进制后旧缓存不失效）。

**CI / 协作**：`node_modules` 未进 git，CI 与任何新克隆/新 worktree 都要先 `npm install` 再 build，否则 UI 无样式。若要让 build 可复现，考虑把 `node_modules` 入库或让 build.rs 失败时报错而非静默写空文件。
