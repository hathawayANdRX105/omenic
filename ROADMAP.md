# omenic ROADMAP

> dsh（deepseek-harness）复刻进度，`todo/dsh/README.md` 为完整计划，此文件只记状态。
> 最后更新：2026-09-13

## 进度

### 阶段 0：目录结构重组 ✅

- [x] 15 crate 平铺 → 域子目录（agent/infra/ui/evidence），`53419ec`
- [x] 新建 `crates/composition`（omenic-composition 组装根占位，依赖 daemon+orbit），`53419ec`
- [x] 根 Cargo.toml members 全换域路径，跨域依赖改 `../../<domain>/<crate>`，`53419ec`
- [x] 28 处 src/ 内联 `#[cfg(test)]` 抽到各 crate tests/（gate rust_tests_in_tests_dir 清零），`53419ec`
- [x] cargo fmt --all + 16/16 cargo check -p 绿

### 阶段 1：harness 四 crate（给 ferrite 复用的核心库）✅

- [x] 占位骨架：omenic-harness-{core,prompt,tools,runtime}，trait 签名对照 dsh + omenic 调用方反推，`906ea2e`
- [x] 4 个 `todo!` 占位填成真实实现：`1b405f0`
  - [x] core::new_run（构造 Run，AbortSignal::flag() 透传取消）
  - [x] tools::default_catalog（包装 agent/tools 的 builtin_tools，10 个内置工具）
  - [x] prompt::render_with_template（system/user/history 三槽位，history → OpenAI JSON）
  - [x] runtime::run_agent_loop（call → tool_call → 回填 → repeat，max_turns + abort）
  - [x] 22/22 集成测试过（含真实 run_bash 断言、tool_call 回填、MaxTurns、预 abort）

### 阶段 2：插件面四件套（§3.2）⬜ 未开工

- [ ] 新建 `crates/harness/plugin`（或铺进 composition）：
  - [ ] `DshPlugin` trait（name + register）
  - [ ] `ServiceRegistry` trait（provide/resolve，HashMap<TypeId, Arc<dyn Any>> 静态注册）
  - [ ] `EventBus` trait（subscribe/emit，至少 emit + 顺序 handlers 两种模式）
  - [ ] `PluginLifecycle` trait（on_load/on_unload，无 effect unwind，手动 Drop）
- [ ] agent 域 crate 补插件 manifest：orbit/adaptor/tools 经 DshPlugin::register 注册
- [ ] 每个插件 trait 至少 1 集成测试

### 阶段 3：ferrite 接线 ⬜ 未开工（等阶段 2 完成后再做）

- [ ] ferrite 根 Cargo.toml 删 crates/harness/* member，加 git 依赖 omenic-harness-*（tag 锁定）
- [ ] 本地开发 [patch] 指向 ../omenic/crates/harness/*
- [ ] /tavern/generate 回归（SSE 事件流形状不变）+ CI 绿
- [ ] tag omenic-harness-v0.1.0 后删 [patch]

### 阶段 4：tavern 域包翻译（ferrite 侧，另开会话）⬜ 未开工

- [ ] dsh-tavern 13 个 TS 包 → ferrite crates/agent/{character,world-book,preset,play,user,session-template,ui-settings,tavern-loader,...}
- [ ] 每个包 ≥1 集成测试，对照 dsh-tavern 行为
- [ ] character + world-book 可并行；play 依赖两者，串行

## 开新会话须知

- **起点**：`1b405f0`（harness 四 crate 可运行，agent loop 完整）
- **harness trait 签名已定**：Provider / ToolExecutor / PromptRenderer / RunState。改签名必须广播所有会话
- **依赖铁律**：harness 禁止依赖 agent 域；agent 域可单向依赖 harness
- **工作目录**：`.wt/<branch>`（git worktree add 必须在仓库根执行，防嵌套）
- **测试**：放同层 tests/，不在 src/ 写 #[cfg(test)]；重型测试推 CI
- **编译**：cargo 命令套 `cpulimit -l 70 -i --`，只跑 `-p <crate>`，禁止全仓 build
