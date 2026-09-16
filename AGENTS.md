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

**`.githooks/` 是 gate 自己的领地，agent 禁止改动 gate 规则**（`hooks/`、`spec/` 下的 gate 规则 yaml、`gate` 二进制）。gate 规则的增删改由用户或 gate 自身的 `gate init` 负责；agent 遇到 gate FAIL 应改自己的提交/PR 正文去迎合规则，而不是去改规则。UI 契约 yaml（7 份，含 `anchors`/`target` 字段）也平铺在 `.githooks/spec/`，但它们不是 gate 规则，由 `bin/web/tests/ui_contract.rs` 消费。

## 构建与验证（CI 驱动）

**测试一律不在本地跑。** `cargo test` / `cargo clippy` / 全量 `cargo build` / `npm install` 全部交给 PR 的 CI（`.github/workflows/ci.yml`）。本地跑测试属违规操作，即使套了 `cpulimit` 也不允许。

本地只允许这三类轻量验证：
- `cargo fmt --check`（秒级，提交前必跑——commit checklist 会拦不合格的 rust）
- `cargo check -p <crate>`（单 crate 类型检查，**不得**加 `--workspace` / `--all-targets`）
- `grep` / `ls` / `git` / 文件读写等只读命令

验证节奏：本地 `fmt --check` + 单 crate `cargo check` → push → **CI 出结果才算验证过**。CI 红了看日志改，不要在本地复现。

**唯一例外**是 web UI 需要肉眼确认时的 `cargo build --bin oi-web`（见下文启动序列），必须套 `cpulimit -l 65 -i --`：

```bash
cpulimit -l 65 -i -- cargo build --bin oi-web
```

## 禁改区

`.githooks/` 归 gate 自身维护，**任何开发任务都不得改动 gate 规则**（包括 `.githooks/spec/` 下的 gate 规则 yaml、hook 脚本）。规则要改先去 demo 沙盒（见下文）验证，并由用户显式指派。例外：UI 契约 yaml（用户指定）与 gate 规则平铺在 `.githooks/spec/`，由 ui_contract.rs 消费，不算 gate 规则。

## Demo 验证沙盒

验证 issue/PR 流程、gh-gate 拦截、规则改动时，**不要在本仓库(omenic)直接创建 demo issue/PR**，使用专用沙盒：

- 仓库：https://github.com/hathawayANdRX105/demo-githooks(本地 `~/projects/demo-githooks`)
- 用途：验证 epic/sub/PR 链路、checkbox 强制、双向关联(GT-04b)、审查强制等，避免污染 omenic
- .githooks 与 omenic 同步；规则改动后先在此仓库验证，再同步到其他项目(deskctl / new-api)

## TUI（已删除）

终端 UI（Ratatui）已在 dsh web 复刻转正时删除：`crates/tui/` 与 `specs/tui/` 均不存在，别再去找。前端验证全部走上面的 Web（oi-web）与「Web UI 契约验收」两节。

`.agent/skills/ui-validation/SKILL.md` 仍保留，但其描述的 TestBackend/specs/tui 流程已无对应代码，用到时先核实目标是否存在。

## Web（oi-web）启动与样式缺失排查

**症状**：Web UI 打开后是「裸文本」——没有暗色主题、没有卡片/气泡样式，文字堆在一起（如 "搜索会话⌘K / 工作区 / Spaces" 全是素文本），但 JS 功能正常（能发消息、minimap 逻辑在跑）。这是**二进制里嵌进去的 CSS 为空**，不是前端没渲染、也不是没合并代码。

**根因（三个坑，按发生顺序）**：

1. `bin/web/build.rs` 需要 `bin/web/node_modules` 里的 `@tailwindcss/cli`，把 `assets/tailwind-input.css` 生成 `tailwind.gen.css`（写到 `OUT_DIR`），再由 `bin/web/src/app.rs` 的 `include_str!` 嵌进二进制。**没装依赖时 npx 回退拉不到 `tailwindcss` 包本体（输入里 `@import "tailwindcss"` 解析失败），build.rs 会静默写一个 0 字节 CSS 兜底**（见 build.rs 注释 "UI would simply be unstyled"）。`node_modules` 不在 git 里，CI / 新克隆 / 新 worktree 都没装 → 全是空 CSS。
2. `cargo build` 只把 `assets/tailwind-input.css` 和 `src` 标了 `rerun-if-changed`，**装了 `node_modules` 之后它感知不到，不会自动重跑 build.rs**，旧的空 CSS 会一直沿用。必须 `touch bin/web/assets/tailwind-input.css` 再编译才重新生成。
3. `cargo run` 起的是**编译前那一刻的二进制**；编译完成后产物已换，但进程还在跑旧的（进程启动时间比二进制 mtime 还早）。必须杀掉重启。

**正确启动序列**（在仓库根目录）：

```bash
cd bin/web && npm install                # 确保 node_modules/.bin/tailwindcss 存在
# 可选：手动确认能生成非空 CSS（约 40KB）
#   node_modules/.bin/tailwindcss -i .tailwind.gen-input.css -o /tmp/t.css
cd <仓库根>
touch bin/web/assets/tailwind-input.css   # 强制重跑 build.rs
cargo build --bin oi-web                  # 不是 -p web；二进制在 bin/web（oi-web）
pkill -x oi-web; sleep 1
nohup ./target/debug/oi-web > /tmp/oi-web.log 2>&1 &      # 起在默认 8026（PORT 可覆盖）
# 验样式：页面内联 <style> 块的字节数（2026-09-16 实测 40005，含 .flex/.mx-auto/padding-left）
curl -s localhost:8026/ | python3 -c 'import sys,re;h=sys.stdin.read();m=re.search(r"<style>(.*?)</style>",h,re.S);print("style bytes:",len(m.group(1)) if m else "NONE")'
```

**判样式不要用 `--color-accent` 计数**：dsh 复刻自研组件落地后该 token 只剩 minimap JS 别名在用，正常二进制里就 4 处，低计数不代表空 CSS。可靠判据是上面伺服页 `<style>` 块的字节数（非 0 且含真实工具类即正常）。若要直接检查生成的 CSS，按**属性值**（如 `padding-left`）而非选择器 grep——Tailwind v4 输出未压缩，选择器里的 `.` / `[]` 会被转义，按选择器 grep 容易漏判。

**快速诊断**：`<style>` 块字节数为 0 或 NONE = 跑的是空 CSS 的旧二进制，按上面序列重建重启。浏览器记得**硬刷新**（LiveView 按 origin 缓存 wasm，换端口/换二进制后旧缓存不失效）。

**CI / 协作**：`node_modules` 未进 git，CI 与任何新克隆/新 worktree 都要先 `npm install` 再 build，否则 UI 无样式。若要让 build 可复现，考虑把 `node_modules` 入库或让 build.rs 失败时报错而非静默写空文件。

## Web UI 契约验收（.githooks/spec 下 7 份 UI 契约）

web UI（dsh 设计语言复刻，C5.1 已验收）的视觉/结构锁在 `.githooks/spec/` 下的 7 份 UI 契约 yaml（workspace / chat / sidebar / stats / settings / quick-switcher / taskpanel），防止后续接线（5.2a/5.2b）破坏。改动 web 组件样式或布局时：先跑契约测试，再按下表浏览器抽查。

**契约测试**：`bin/web/tests/ui_contract.rs`，**由 CI 跑，本地不跑**（见上文「构建与验证」）。本地只做静态核对：改了组件 class 就同步改 `.githooks/spec/` 下对应 UI 契约 yaml 的 `find` 锚点，用 grep 确认锚点字符串在实现文件里真实存在。

**yaml 字段约定**：`name`（契约名）/ `target`（omenic 实现文件，相对仓库根）/ `description` / `anchors`（锚点列表，每项 `key` + `find`（源码中稳定 class 片段或静态字面量）+ `expect`（预期形态）+ `source`（omenic 实现位置 + dsh 出处）+ 可选 `file`（锚点级实现文件覆盖，默认用 target））/ `notes`。测试两类断言：① 每个 yaml 可被 serde_yaml 解析且字段齐全；② 每个 `find` 关键字在对应实现文件中出现。新增 spec 必须同步登记 `tests/ui_contract.rs` 的 `SPEC_FILES`。

**起服**：按上文「正确启动序列」起 oi-web（记得 npm install + touch css + 重建重启，浏览器硬刷新）。

**浏览器抽查点**（每屏挑核心）：

- `workspace.yaml`——三列 grid：侧栏 280px 拖拽夹取 264–420、折叠后 rail 56px；中栏只有 44px 面包屑头（无顶栏）；分支 chip 品牌蓝。
- `chat.yaml`——发送消息后「工作过程」折叠行出现（24px 头 + 计数）；用户气泡右对齐 r22、max-w 525；发送钮 34px 圆形品牌蓝；状态行 12px 居中（model · tokens · $cost · context）。
- `sidebar.yaml`——logo 行 52px + 18px 字标；新会话钮 h38 r12；项目行 34 / 会话行 32（缩进 22px，状态点 brand/dim/danger）；时间戳 hover 隐藏；底部数据统计/设置行 42px。
- `stats.yaml`——KPI 卡一行 5 张；指标带一行 7 格；主体三列 320/1fr/340；吞吐折线品牌蓝。
- `settings.yaml`——弹窗 800px r24、左导航 188px（单元 h40 r12）；「关于」页有版本号 chip。
- `quick-switcher.yaml`——⌘K/Ctrl+K 弹出 560px 顶部对齐面板；输入 h44、会话行 h40；ESC 退出。
- `taskpanel.yaml`——composer 上方 dock 卡宽随消息列（≤780）；进度条 1px 品牌蓝；filter chip h26 r7；任务卡 r10。

**已知偏差（记录不改）**：dsh 消息列 748px，omenic 消息列与 composer 统一 `max-w-[780px]`（chat.yaml notes）。
