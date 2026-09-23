<!-- canon: hathawayANdRX105/canon @ d0a4b2c (synced 2026-09-23) -->
---
name: ainotation-web
description: 'Ainotation 视觉标注反馈闭环：启动 admin-web 开发环境 + 标注同步栈（service/bridge/grant），并用 MCP 读写用户在页面上的 UI 标注。凡是用户提到「启动前端标注」「看我的标注/反馈」「ainotation」「页面标的问题改一下」，或要在 PR smoke 之外理解用户对 UI 的视觉意见，都用本技能——即使用户没说「ainotation」这个词。'
license: MIT
---

# Ainotation 视觉标注反馈闭环

## 目标

用户在浏览器里对 admin-web 页面**点选元素写反馈**（带 DOM 选择器/样式/截图），
agent 通过 MCP 读取这些标注并完成 UI 修改。本技能让你能在任何 worktree 会话里
把整条链拉起来、读出标注、并回答/闭环。

**架构一句话**：浏览器 SDK（bundle 注入 admin-web）→ IndexedDB → grant 同步到
本地 service → MCP（`@ainotation/mcp`）→ agent。
同步桥是本项目自建的（上游只给 Vite 插件提供中间件桥，`dx serve` 挂不了），
原理与协议细节见 `apps/admin-web/AINOTATION.md`，本技能只讲**怎么做**。

## 启动顺序（硬约束：service 必须先于 agent 的 ainotation MCP 可用）

main 的 `just dev-web` 不内建标注栈，按序起三件（长跑命令用运行时托管后台任务启动，
**禁止 `nohup &`**，CPU 类套 `cpulimit -l 65 -i --`）：

```bash
just aino-service        # 1. MCP service（幂等：已在跑就直接返回）
just aino-bridge         # 2. 同步桥 :44090，给页面 origin 签 grant 并每 2 分钟续租
just dev-web 8090 debug  # 3. 前端免登录预览
just aino-check          # 体检：service 注册表 + 桥端点 + 前端连通
```

端口不是 8090 时，桥要知道页面 origin：

```bash
AINO_ORIGIN=http://127.0.0.1:<端口> just aino-bridge
```

改了 `ainotation-entry.ts` 或升级 SDK 后：`just aino-bundle` 重打 bundle
（内容变化会改 manganis 指纹 → 必须重启 dx，浏览器再强刷）。

**顺序错了的症状**：omp 会话先于 service 启动时，其 ainotation MCP 的工具调用会
无限挂起（initialize 正常、tools/call 无响应）。修复：拉起标注栈后，在 omp 里
`/mcp reconnect ainotation`，或重开 omp 会话。

## 读用户的标注（agent 侧）

MCP 配置已在用户级 `~/.omp/agent/mcp.json`（`connect --directory <ferrite>`），
所有 omp 会话自动可用。工具共 14 个，常用的：

| 工具 | 用途 |
|---|---|
| `ainotation_list_projects` | 确认 `ferrite-admin` 已注册（应恒在） |
| `ainotation_list_sessions` | 列页面会话（按 URL 隔离） |
| `ainotation_get_feedback` | 拉标注全文：comment + targets(selector/ancestors/text) + styleChanges + 截图 |
| `ainotation_get_image` | 单独取标注截图 |
| `ainotation_update_annotation` / `_delete_annotation` | 回复状态 / 清理 |

读到的标注要点：`comment` 是用户原话，`targets[].selector` 可直接用于定位代码，
`text`/`nearbyText` 是元素当时的内容（对回"用户点的是哪一块"最有用）。

**标注数据真找不到时**（service 刚重建等），最后一招：直接读浏览器
IndexedDB（Firefox 系在 `~/.zen/<profile>/storage/default/http+++127.0.0.1+8090/idb/*.sqlite`，
值为结构化克隆二进制，`strings` 可提取选择器/文本）。

## 改了入口或升级 SDK 后

```bash
just aino-bundle    # bun install + 重打 assets/ainotation/ainotation.iife.js
```

bundle 内容变化会改变 manganis 指纹 → **必须重启 dx**（`--watch false` 不自动重建，
且依赖 crate 变化 dx 也不重建，见 `.agent/rules/dev-env.md`）。

## 排障速查

| 症状 | 原因 | 处理 |
|---|---|---|
| 页面白屏、无头浏览器 tab 卡死 | 旧版 DropdownMenu busy-poll 钉死 wasm 主线程（web-visual 会话修复 `67963a3`） | 确认分支含该修复，缺则 cherry-pick，重启 dx |
| `Err 404 - dx is not serving a web app` | wasm 还在编（首次 336s+，多会话抢核更久） | 等；判据是 `/wasm/admin-web.js` 200 且 `target/wasm32-unknown-unknown/wasm-dev/deps/*.wasm` 落盘 |
| ainotation 工具调用挂起 | service 不健康或晚于 omp 会话启动 | `just aino-service` + `/mcp reconnect ainotation` |
| `just aino-service` 报 already locked | 别的 service 实例持锁（可能僵尸） | `pgrep -af 'cli.mjs service'` 查归属，确认无主再清 |
| npx 拉 mcp 包超时 | 走代理慢 | 直接 `node ~/.npm/_npx/*/node_modules/@ainotation/mcp/dist/cli.mjs`（绕过 npx 解析） |
| bundle 改了页面没变化 | dx 未重建（见上） | 重启 dx 进程 |

## 边界

- 仅开发环境：release 构建无 bundle（`debug_assertions` 门控），生产无痕。
- grant token 含在 `apps/admin-web/assets/ainotation/connection.json`（已 gitignore），
  泄露面仅限本机 localhost。
- Dioxus 重渲染会替换节点：结构改动后旧标注目标失效，重新标注即可。
