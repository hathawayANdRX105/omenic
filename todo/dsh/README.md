# omenic → DSH 插件式重构（M3 后续·omennn 计划）

> 前序：`direction-roadmap-v3.md` 的 M1（OMP loop）+ M2（jcode 内存）已完成。
> 本计划把 DSH（DeepSeek Harness）的“一切皆插件”核心用 Rust 复刻进 omenic，
> 同时把组装层与 crate 目录重组为可复用结构，供 ferrite（酒馆域）复用。
> 触发条件（v3 §4 三件）已全部满足：① loop 稳定且有测试（M1）；② 工具/子代理接口定型（M2+）；③ 出现真实多形态扩展需求（ferrite 酒馆域）。

## 1. 目标形态

```
omenic/
├── bin/          ① 入口（装配+启动，无逻辑）
│   ├── cli/       oi 主命令（编程工具）
│   ├── gate/      合规钩子 CLI（域内保留）
│   └── web/       web UI 入口（Dioxus）
│
├── crates/
│   ├── composition/   ② 组装根：把 ①③ 拼起来；daemon Worker/SessionDb 装配；
│   │                  宿主服务注册（sessions/slots/systemPrompt/uiWorkspace）
│   │                  — 依赖方向严格向下单行，这是整个 repo 里唯一知道所有 crate 的模块
│   │
│   └── （三层聚合，见 §2）
│
└── （Cargo workspace 成员 = bin + composition + crates/* 各 crate）
```

**依赖铁律**：入口 → 组装 → 库；库之间禁止私互依赖（跨域只允许 `contract` DTO 互通）。
箭头出现横向或向上 = 架构开始烂的那天。

## 2. crate 目录重组（第一步，纯 git mv，不改代码）

现有 15 crate 平铺 → 三层聚合：

```
crates/
├── agent/       agent 域（核心循环+会话+编排）
│   ├── adaptor/     LLM 协议层（OpenAI 兼容流式）
│   ├── orbit/       agent loop（纯函数化，M1 已做）
│   ├── session/     会话存储（libSQL）
│   ├── subagent/    子代理编排（M1 OL-4）
│   ├── task/        任务模型+图谱
│   └── mcp/         MCP 协议客户端
│
├── harness/     ③b 公共库（给 ferrite 用，独立 tag 发布单元）
│   ├── core/        Run/Step 状态机 + RunStatus + AbortSignal
│   ├── prompt/      提示词模板引擎（M1 对齐 OMP）
│   ├── tools/       工具协议（impl Tool + schema）
│   └── runtime/     循环引擎（spawn/abort/persist）
│
├── infra/       基础设施域
│   ├── config/      TOML+env 配置
│   ├── rpc/        UDS JSON-RPC
│   ├── daemon/     无头 daemon（Worker + SessionDb 装配）
│   └── memory/     记忆层（M2 jcode）
│
├── ui/          UI 域
│   ├── tui/       终端 UI（ratatui）
│   └── web/       Dioxus web（M3）
│
└── evidence/    合规域
    ├── spec/       规格合规检查
    └── prompts/    提示词内容（系统 prompt 常量）
```

**执行**：`git mv crates/adaptor crates/agent/adaptor` … 全部移完，改 `Cargo.toml` 的 `[workspace] members`，
全仓 `cargo check` 绿 = 完成。不动一行逻辑代码。

## 3. 插件系统复刻（DSH 核心）

### 3.1 DSH 插件架构拆解

- **Cordis 运行时**（9 文件，2693 行 TS）：
  - `context.ts`：`Context`（服务容器）+ `extend/isolate` 作用域
  - `service.ts`：`Service` 基类 + `provide(name)` 注册
  - `fiber.ts`：插件生命周期（PENDING→LOADING→ACTIVE→UNLOGING→DISPOSED）
  - `events.ts`：事件总线（emit/subscribe，5 种派发模式）
  - `registry.ts`：插件注册 + `inject()` 依赖解析
  - `reflect.ts`：`ctx.get(key)` 动态代理

- **DSH 包**（55+ 个）按域分三类：
  - **编排核心**（agent-loop/session/tools/llm）：omenic 对应 orbit/session/adaptor/tools —— 已有，需接口对齐
  - **扩展服务**（mcp/subagent/task/memory）：omenic 已有对应 crate，需补插件 manifest
  - **UI 渲染**（web/client）：omenic 用 Dioxus，不复刻

### 3.2 Rust 复刻面（只做必要部分）

omenic 不照搬 Cordis 运行时（TS 专属魔法：effect 自动 unwind、declaration merging、HMR），
而是实现**插件面**：

```rust
// 1. 插件 trait（注册时实现）
trait DshPlugin {
    fn name(&self) -> &str;
    fn register(&self, ctx: &mut PluginContext);  // 注册服务/事件
}

// 2. 服务注册（替代 cordis ctx.get/provide）
trait ServiceRegistry {
    fn provide<T: 'static>(&mut self, key: &str, value: T);
    fn resolve<T: Any + 'static>(&self, key: &str) -> Option<Arc<dyn Any>>;
}

// 3. 事件总线（替代 cordis events）
trait EventBus {
    fn subscribe(&self, topic: &str, handler: Box<dyn Fn(&Event) + Send + Sync>);
    fn emit(&self, topic: &str, payload: &dyn Any);
}

// 4. 生命周期（简化版，无 effect unwind）
trait PluginLifecycle {
    fn on_load(&mut self, ctx: &PluginContext);
    fn on_unload(&mut self, ctx: &PluginContext);
}
```

**不做的**：HMR、动态插件热加载、Cordis 的 context proxy（用 `HashMap<TypeId, Arc<dyn Any>>` 静态注册代替）。
**理由**：插件面 = 让扩展 crate 能注入/消费服务，不需要运行时动态发现。

## 4. 并行切分（保证 3 会话无冲突）

| 会话 | 域 | 做什么 | 前置 |
|---|---|---|---|
| **S1 omenic-core** | crates/harness + agent | §2 目录重组 + §3 插件面（trait 实现） | 无（Day 1 先出 trait 签名冻结） |
| **S2 omenic-web** | crates/ui + composition | Dioxus web（M3 GW-1~5）+ 组装层 | S1 的事件流 mock 完成 |
| **S3 ferrite-tavern** | ferrite 侧 | contract DTO + tavern-* 后端 + 删除 ferrite crates/harness | S1 的 harness tag + contract DTO 冻结 |

**S1 内部时序**（S1 会话内顺序做，不并行）：
1. Day 1：§2 git mv 全部完成 + cargo check 绿
2. Day 2：`crates/harness/{core,prompt,tools,runtime}` 四个空 crate 骨架（trait 签名，内部 `todo!("TODO(#N): …")`）
3. Day 3-5：§3.2 插件面实现 + 各 crate 内部填充（orbit→runtime、adaptor→tools、prompts→prompt）

## 5. 验收

- `cargo check --all-targets` 全绿
- `cargo test -p harness-core -p harness-prompt -p harness-tools -p harness-runtime` 各自有 ≥1 集成测试
- `oi`（bin/cli）全链路可用：init → task add → 流式 run → 事件流 → 持久化
- `git tag omenic-harness-v0.1.0`，ferrite 侧 `git pull` 验证可正常编译

## 6. 风险

| 风险 | 缓解 |
|---|---|
| 目录 git mv 改 workspace 路径，Cargo.lock 需重新生成 | `cargo check` 触发自动更新 lock；CI 动态选包会算全量兜底 |
| 插件 trait 过度设计 | 只做 §3.2 四件套，不碰 Cordis 运行时；manifest 式描述对齐现有 Tool impl |
| harness crate 与现有 agent crate 边界模糊 | 铁律：agent 域 crate 可依赖 harness（单向），harness 禁止依赖 agent 域 |
