# omenic crate 增强路线图

对照 pi-from-scratch 5 文件原版，按 crate 分析增强方向。

## 当前结构

```
bin/
├── cli/     # omenic CLI (main.rs + cli.rs + config.rs)
├── gate/    # gate CLI (main.rs + rules/ + tools/ + shared.rs)
└── tui/     # terminal chat (main.rs + app.rs + ui.rs + markdown.rs)

crates/
├── adaptor/ # LLM stream + SSE + OpenAI (lib.rs + sse.rs + openai.rs)
├── orbit/   # agent invariant loop (lib.rs)
├── tools/   # read/write/edit/bash, registry (lib.rs + 4 tool files)
├── rpc/     # omp RPC client + worker (lib.rs + client.rs + worker.rs)
├── task/    # task model + graph + store + runner + template
└── spec/    # spec tables (lib.rs + init.rs + parse.rs + render.rs + check.rs)
```

## 优先级

### P1: adaptor — 流式迭代器

**问题**：`openai::stream()` 阻塞到全部收完才返回 `Vec<StreamEvent>`。TUI 和 orbit 无法逐 delta 渲染。

**方案**：
- 加 `stream_cb(model, ctx, tools, signal, |ev: &StreamEvent|)` 回调版本
- 或返回 channel/iterator，调用方按需消费
- 保留原 `stream()` 作为 `stream_cb` 的 collect 包装，兼容现有调用方

**影响**：tui 逐字显示、orbit 实时事件处理

### P2: bin/tui — 流式接入 + 会话持久化

**问题**：当前等全部返回后一次性显示，无历史记录。

**方案**：
- 接入 adaptor 流式迭代器，逐 delta 追加到消息列表
- 会话历史存 JSONL（复用 task store 模式）
- 多会话切换（左侧列表）

### P3: bin/cli — 拆 cli.rs

**问题**：cli.rs 3737 行，所有命令在一个文件。

**方案**：
- 按命令拆模块：task.rs / dep.rs / run.rs / steer.rs / abort.rs / spec.rs / template.rs / init.rs
- main.rs 做路由，每个命令文件 100-300 行

### P4: tools — 加 grep/glob/delete

**问题**：当前只有 read/write/edit/bash，缺搜索和删除。

**方案**：
- grep.rs：正则搜索文件内容
- glob.rs：按 pattern 列文件
- delete.rs：删文件（gio trash 或 rm）
- 各自注册到 lib.rs::builtin_tools()

### P5: task — runner 多流程

**问题**：runner.rs 是硬编码单流程（dep check → spawn → prompt → event loop → result）。

**方案**：
- 流程定义为可配置（YAML 或代码 trait）
- 多流程：单次执行、批处理、并行 fan-out
- template 与 spec 联动（apply template 后自动生成 spec）

### P6: orbit — compaction 升级

**问题**：compaction 用固定阈值 50 条/保留 20 条，无 token 估算。

**方案**：
- 加 token 估算（简单字符数或 jcode）
- 动态 compaction 阈值
- ScriptedBackend 提到公开 API 方便外部测试

### P7: rpc — 重连/超时

**问题**：无重连机制，worker crash 后不恢复。

**方案**：
- 连接超时 + 自动重连
- worker crash 恢复（重启进程）
- 心跳检测

### P8: spec — 自动校验联动

**问题**：review spec 的 check 是纯文本匹配，未与 CRG 输出联动。

**方案**：
- CRG 输出 → spec check 自动化
- render_skeleton 支持 webhook 模板变量
