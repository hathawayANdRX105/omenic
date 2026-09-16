# omenic 当前进度与规划

> 已交付的东西见 [ROADMAP-delivered.md](ROADMAP-delivered.md)（背景档）。本文只讲**还没做的**和**接下来做什么**。
>
> 编号体系：`C1–C8` 能力域 / `R1–R7` 并发路线 / `G1–G6` 整合门。源码注释按编号引用，勿改。

## 当前位置

**G1–G5 全部已过，main 干净**（最新 `e45cee0`）。C1–C6 六个能力域的代码与测试都在 main 里，C7（tag）等用户拍板时机，C8 已裁定不做。

但 G5 的「真装配」差最后一公里：**装配出的插件容器在唯一的生产路径（daemon worker → web UI）里零消费**。注册是真的，消费是假的。这是当前最重要的状态——C4 的压缩与 AGENTS.md 注入两項「已完成」能力，对真实用户不存在。

## 开放缺口（全部已 grep 实测）

| # | 缺口 | 证据 | 规模 | 归属 |
|---|---|---|---|---|
| 1 | **装配容器零消费** | `worker.rs` 用 `tools::builtin_tools()` + `orbit::LoopConfig::default()`（`instruction_cwd`/`maintain` 皆 None）；`DaemonConfig` 无 cwd 字段；`server.rs` 的 fiber 字段叫 `_fiber`；`assemble_plugins` 传 `Vec::new()`；全仓 `.resolve(` 生产调用者 = 0 | 中（150–250 行） | **G6** |
| 2 | **crash-repair 未接线** | `interrupted_run_closers` 唯一调用者是 `session/tests/turn_repair.rs`（7 处全测试）；`Daemon::start` 流程 assemble→lock→bind→open SessionDb→RunLedger→accept loop，无 repair 步骤 | 小（40–80 行） | **G6** |
| 3 | **orbit 两条宿主接缝超限** | 压缩接缝 56 代码行 + workspace instructions 接缝（`build_system_prompt`）17 代码行 = 73 行，超 R3 ≤20 限额 53 行 | 小（60–100 行纯搬运） | **G6** |
| 4 | **5.3/5.5 谱系分组** | web 侧零谱系代码；`SessionSummary` 无 parent 字段——**不是只缺 UI 分组，是缺数据模型**（ROADMAP 旧文「待按 `run.list` 组装」不准确，`run.list` 里没有父子关系） | 中（100–200 行 + schema） | 待规划 |
| 5 | **2.3 turn codec 零生产写入** | `encode/decode_turn_log` 只被测试调；orbit TurnStart/TurnEnd 是内存事件，持久化走 libSQL | 小（接线 40–60 行 / 删 20 行） | **决策题**：接线 or 删死代码 |
| 6 | **单 worker 事件无会话归属** | 并发 turn 事件会混写最近 `on_send` 的会话；#356 只让 web 侧 prompt 带 session_id/run_id，根治需协议加 per-run 路由 | 中 | R2 协议层，暂靠「单运行」纪律兜底 |

## 下一个整合点：G6 总装消费（R7）

**触发条件已满足**：G5 已过，无前置阻塞。

**做什么**（按顺序，有依赖）：

1. **接缝瘦身（先做）**——把 `LlmSummarizer` 桥 + DTO 转换从 orbit 搬进 `crates/harness/compaction/`，压缩接缝降到限额内。行为不变：`orbit/tests/{loop.rs(20), compaction_e2e.rs(4), instruction_prompt.rs(4)}` 零改动。
2. **装配容器消费（核心）**——daemon worker 从容器取配置而不是硬编码：
   - `DaemonConfig` 加 `cwd` 字段 → 传给 `LoopConfig.instruction_cwd`（AGENTS.md 注入首次在生产路径生效）
   - 从 fiber 解析 `harness.compaction` 的 `CharBudgetPolicy` 构造 `maintain` 钩子；`compact_context` 改成收 policy 参数（现在硬编码 `COMPACT_CHAR_BUDGET`）
   - `max_turns`/`model` 由 config 文档驱动
   - 让 `assemble_plugins` 的 fiber/registry 真正被读，`_fiber` 去掉下划线
3. **crash-repair 接线**——`Daemon::start` 在 open SessionDb 之后、bind 之前调一次 `interrupted_run_closers`，把半开 run 落库标 `ABORTED`。

**关键边界（不要越界）**：**不要**把 daemon worker 从 orbit 切到 `harness/runtime::LoopEngine`——那是另一个数量级的重写。`harness.tools`/`harness.loop` 的消费者应是 harness 域自己的 loop。G6 的形状是 **orbit 保留为生产 loop，容器提供 config 与压缩策略，worker 做最小桥接**。orbit 取 `&[Box<dyn Tool>]`，与 harness 的 `ToolCatalog` 不同型，别强行统一。

**验收条件**：
- web（8026）一次真实 run，模型回复的 system prompt 含项目 `AGENTS.md` 内容
- 会话灌到 >120k 字符后继续对话不报错且上下文被压缩（tool_call/tool_result 成对不变式成立）
- config 的 `max_turns` 改了立即生效
- `pkill -x oi-daemon` 中途杀掉 → 重启（不刷新页面）→ 半开 run 标 aborted，会话列表不再有僵尸「运行中」

**冲突记账**：改 `worker.rs`（R2 独占文件）+ orbit 接缝区（R1/R3 限额共享）。接缝限额随 G6 重新记账——压缩接缝瘦身后，两条宿主接缝合计应落到 ≤20。

## 后续 backlog（按价值÷成本排序）

| 优先 | 项 | 说明 |
|---|---|---|
| 1 | 谱系分组（5.3/5.5） | C5 最后一个 🟡，用户可见。**先加数据模型**（run 记录补 parent 字段或从 session 派生），再谈侧栏分组 |
| 2 | 插件 per-plugin Config schema + inject 依赖门控 | trait 加关联类型/serde 结构 + 从主文档切插件切片 + 校验失败拒注册；插件声明所需服务名，缺失时延迟激活。两项改同一个 trait + registry，合并做省一半 |
| 3 | 2.3 turn codec 定性 | 接线 or 删，先决策 |
| 4 | 插件反注册（单插件卸载） | `fiber.rs` 的 `unload()` 只能逆序拆全部；`EventBus::unsubscribe` 已有 |
| — | 插件发现/加载层（loader） | dsh 的 loader 绑 Node ESM hook，Rust 侧等价物是目录扫描+声明式注册表；**范围外**（C6 只对齐 cordis），暂不分配 |
| — | token-meter（C8.3） | 已裁定不做，stats 卡靠「无真数据则隐藏」兜底 |
| — | interaction 交互层（C8.1/8.2） | 已裁定不做，omenic 无 agent→用户提问通路 |

## 工作约定

- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **PR-only**：2026-09-15 裁定只开 PR 不开 issue；缺 `Fixes #` 只是 WARN 不阻塞
- **测试一律不在本地跑**：`cargo test` / `cargo clippy` / 全量 `cargo build` / `npm install` 全部交给 PR 的 CI（`.github/workflows/ci.yml`）。本地只允许 `cargo fmt --check`、单 crate `cargo check -p <crate>`（**不得**加 `--workspace` / `--all-targets`）、grep/ls/git/读写
- **唯一例外**：web UI 需要肉眼确认时的 `cpulimit -l 65 -i -- cargo build --bin oi-web`
- **提交前必跑** `cargo fmt --all`（commit checklist hook 拦不合格的 rust）
- **测试放同层 `tests/`**，不在 src/ 写 `#[cfg(test)]`；重型测试推 CI
- **改组件源码要同步** `.githooks/spec/` 下对应 UI 契约 yaml 的 `find` 锚点（新 spec 必须登记 `bin/web/tests/ui_contract.rs` 的 `SPEC_FILES`）
- **加依赖必须同步改 `Cargo.lock`**（CI `--locked` 拒绝重算）
- **`.githooks/` 只读**：gate 规则、hook 脚本、spec yaml 下的 gate 规则都不许改（UI 契约 yaml 由 `ui_contract.rs` 消费，不算 gate 规则，可改）。gate FAIL 了改自己的提交/正文去迎合，不要改规则
- **合并**：gate 拦截的 `gh pr merge --squash` + 删分支 + 清工作树；绝不本地 merge 到 main
- **领域依赖方向**：harness ✗→ agent 域；agent 域 ✓→ harness；跨域只走 contract DTO
- **无 `Co-authored-by` trailer**
