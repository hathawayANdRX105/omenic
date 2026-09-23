<!-- canon: hathawayANdRX105/canon @ 6a7370c (synced 2026-09-23) -->
---
name: scaffold-dsh
description: "铺地基：对照参考实现（dsh）用 codegraph 找出 gap，在目标 crate 里写 todo!/unimplemented! 占位（带出处+类型约束+issue 号），让全仓 cargo check 绿。适用场景：把某个外部项目（deepseek-harness / oh-my-pi / jcode）的某个模块复刻进本仓库 crate；对已有 crate 补充功能时参考现有实现铺骨架。触发词：铺地基、scaffold、骨架、占位、对照、复刻、replicate、replica。"
license: MIT
---

# 路径约定

本文档中所有绝对路径以 `<repo根>` 为占位符，表示**使用该 skill 的仓库根目录**（ferrite 或 omenic，按使用方决定）。

示例对照（仅供参考，实际执行时替换为 `<repo根>`）：

| 占位符 | ferrite 实例 | omenic 实例 |
|---|---|---|
| `<repo根>` | `/home/hathaway/projects/ferrite` | `/home/hathaway/projects/omenic` |
| `<repo根>/.wt/<name>` | `/home/hathaway/projects/ferrite/.wt/<name>` | `/home/hathaway/projects/omenic/.wt/<name>` |
| `<repo根>/crates/harness/<module>/src` | `/home/hathaway/projects/ferrite/crates/harness/<module>/src` | `/home/hathaway/projects/omenic/crates/harness/<module>/src` |

---

# 铺地基：对照参考实现写占位骨架

## 目标

把参考项目（如 `~/projects/harness/deepseek-harness`）的某个模块**先铺进本仓库 crate 的结构**：函数签名、trait 定义、模块边界、类型定义——全部写好；具体实现用 `todo!()` / `unimplemented!()` 占位。占位完成后全仓 `cargo check` 必须绿，子代理才会在后续逐个填占位实现。

**关键原则**：铺的是"接口与结构"，不是"实现"。判断标准——每个占位函数必须有完整的参数类型、返回类型、文档注释（说明这个函数要做什么、对应参考实现的哪一行）。子代理拿到后只看签名 + 注释就能知道要写什么。

---

## 前置条件

- 参考项目路径（如 `~/projects/harness/deepseek-harness`），必须已建 codegraph：
  `cd <参考项目> && code-review-graph update`（若没有 `.code-review-graph/graph.db` 则先 build）
- 目标 crate 已建好（空 crate 或现有 crate），**若目标 crate 不存在**：先在 workspace `Cargo.toml` 加 `member`，照现有 crate 最小骨架新建 `Cargo.toml` + `src/lib.rs` + `tests/` 目录
- 知道本次要铺哪个功能模块（对照参考实现的哪个文件/目录）

**边界场景**：
- 若参考项目**没有对应模块**：必须停下询问用户确认接口设计。若继续，则出处栏写「无参考，自设计」，禁做项相应豁免。

---

## 步骤

### 1. 对比两侧结构

**目标侧**（本仓库）：
```bash
cd <repo根>/.wt/<name>
code-review-graph update
# 列出目标 crate 现有结构
find crates/harness/<module>/src -name '*.rs' | sort
grep -rnE '^\s*pub (fn|struct|enum|trait|type)|^\s*impl ' crates/harness/<module>/src/
```

**参考侧**（外部项目）：
```bash
cd <参考项目>
# 建图（若尚未建）
code-review-graph build
# 查询目标模块的符号结构（推荐，使用 codegraph CLI）
codegraph query <模块名> -k function -l 200 || true
# 没有 codegraph 时兜底（TS 项目）：
grep -rnE '^\s*export (async )?(function|class|interface|enum|type|const)' packages/<ref-module>/src/ | head -80
```

**输出**：两侧各自的符号清单（函数/struct/enum/trait/impl），用于找出 gap。

### 2. 找 gap 并记录

对照两侧，分类：

| 类型 | 含义 | 处理 |
|---|---|---|
| **已有** | 本仓库已有对应函数/类型 | 不改，直接复用 |
| **缺失** | 参考有、本仓库没有 | 新建，写 todo!() 占位 |
| **接口不同** | 都有但签名/语义不一致 | 对齐参考（从现有调用方反推类型约束），改签名 |

**类型约束规则**（重要）：
- 缺失函数的参数/返回类型 → **从本仓库现有调用方反推**，不从参考正向推
  例：本仓库 `orbit::run_loop` 调 `ToolExecutor::execute(&self, tool: &str, args: &Value)` →
  占位的 `ToolExecutor::execute` 签名必须完全匹配这个调用点，不是参考里的 TS 签名
- 只有本仓库没有任何调用方的纯新增函数，才允许直接照参考签名写

### 3. 写占位

**格式规范**（每个 todo! 必须带四项信息）：

```rust
/// <一句话说明这个函数做什么，以及对应的参考实现出处>
/// 参考: deepseek-harness/packages/<ref>/src/<file>.ts:<line> — <TS 函数名>
pub fn <function_name>(
    <参数>,
) -> <返回类型> {
    todo!("TODO(#<issue>): <函数名> — 参考 <ref_path>:<line>；约束：<类型来源说明>；非目标：<明确不做什么>")
}
```

> 注意：`#<issue>` 是占位符，实际填写时替换为真实 GitHub issue 号；若暂无 issue 则写 `#TBD`（gate 会在后续 PR 里校验）。

**示例**：

```rust
/// 把 LLM 流式事件归一化为 AgentEvent。
/// 参考: deepseek-harness/packages/agent/src/stream.ts:218 — normalizeStreamEvents
pub fn normalize_stream_event(
    raw: &StreamEvent,
    model: &Model,
) -> AgentEvent {
    todo!(
        "TODO(#<issue>): normalize_stream_event — 参考 stream.ts:218；\
         约束：调用方 orbit/src/lib.rs:48 传 &StreamEvent + &Model，返回 AgentEvent；\
         非目标：不做重试/降级（那是 adaptor 层的事）"
    )
}
```

**trait 占位**：
```rust
/// 会话持久化接口。
/// 参考: deepseek-harness/packages/session/src/persist.ts:42 — SessionPersistence
pub trait SessionPersistence: Send + Sync {
    /// 持久化一个 run 的完整步骤序列。
    /// 参考: persist.ts:44 — save(run_id, steps)
    fn save(&self, run_id: &RunId, steps: &[Step]) -> Result<(), PersistError>;

    /// 从磁盘加载历史 steps。
    /// 参考: persist.ts:50 — load(run_id)
    fn load(&self, run_id: &RunId) -> Result<Vec<Step>, PersistError>;
}
```

### 4. 全仓编译绿

```bash
cd <repo根>/.wt/<name>
cpulimit -l 70 -i -- cargo check -p <crate_name>  # 必须带 -p，禁止无 -p 全仓 check
# 有下游依赖 crate 的一并检查
cpulimit -l 70 -i -- cargo check -p <downstream_crate>  # 必须带 -p，禁止无 -p 全仓 check
```

`cargo check` 全绿 = 铺地基完成。

### 5. 登记占位清单

把所有 `todo!("TODO(#N): ...")` 汇总到目标 crate 的 `README.md` 末尾（"待实现"小节）：

```markdown
## 待实现占位清单（scaffold）

| Issue | 函数 | 参考出处 | 类型约束来源 |
|---|---|---|---|
| #12 | `normalize_stream_event` | stream.ts:218 | orbit/src/lib.rs:48 调用点 |
| #13 | `SessionPersistence::save` | persist.ts:44 | 无调用方，照 TS 签名 |
```

这个清单就是子代理的实现任务清单。

---

## 禁做项

- ❌ 把 TS 签名直接抄进 Rust（类型系统不同，必须从 Rust 调用方反推）
- ❌ 写超过 20 行的"实现骨架"（todo! 只到函数/方法级，不拆到块级）
- ❌ 占位不带 issue 号（违反 `rust_todo_needs_issue` gate 检查）
- ❌ 占位不带参考出处（子代理无法定位参考代码，会自由发挥）
- ❌ 测试放 `src/` 的 `#[cfg(test)]`（项目约定：测试放同层 `tests/`；简单单测也在 PR CI 上跑，不在本地）
- ❌ 本地裸跑 `cargo test`（必须套 `cpulimit -l 70 -i --`；重型/集成测试推给 CI 动态选包跑，本地只 `cargo check -p <crate>`）
- ❌ 跨域 crate 直接 import（违反依赖铁律，跨域只能走 `crates/contract` DTO）

> 说明：cargo 命令必须带 `-p` 指定目标 crate，禁止无 `-p` 的全仓 check。

---

## 常见 TS→Rust 类型映射

|TS 类型|Rust 对应|备注|
|---|---|---|
|string|String / &str|&str 用于函数参数，String 用于拥有权|
|number|i64 / f64|整数默认 i64，浮点 f64|
|any / unknown|serde_json::Value|动态 JSON 值|
|Promise\<T\>|impl Future\<Output=T\>|异步 trait|
|Map|std::collections::HashMap|无序|
|Array\<T\>|Vec\<T\>|固定顺序|
|undefined / null|Option\<T\>|无值 = None|
|class|enum + 状态结构体|行为拆成方法，数据拆成 struct|

## 完成标准

- [ ] `cargo check -p <crate>` 全绿（含所有下游 crate）
- [ ] 每个 `todo!` 带 issue 号 + 参考出处 + 类型约束来源
- [ ] 目标 crate `README.md` 有"待实现占位清单"小节
- [ ] 没有超过 20 行的"实现骨架"
- [ ] 没有跨域 crate 直接 import
