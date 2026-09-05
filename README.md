# omenic

<!-- demo note (issue #75): compliance gate deployed -->
Omenic is a task-driven agent orchestrator where agents act as functions following Prompt → Result.

## Commands

`cargo install --path .` installs both `omenic` and the short alias `cli`.

```bash
oi plan
oi task add "implement login"
```


## Web UI

Dioxus fullstack Web 界面（工作区 / 数据统计 / 模型配置）：

```bash
# 启动 Web 服务
cargo run -p web-cli
```

启动后在浏览器访问 `http://127.0.0.1:8026`（或 Dioxus 默认分配端口）。

- **工作区**：左侧会话列表、右侧聊天流 + 任务编排看板、输入框上方状态栏（模型/分支/Token/费用/上下文占用）
- **数据统计**：仿 OH MY PI Observability 面板（KPI 指标卡、Token 分布、吞吐趋势折线图、最近请求 Feed）
- **配置**：模型与 Provider 渠道配置
## Subagent exploration (opt-in)

Read-only parallel exploration via `crates/subagent`. Each subagent only sees
`read` / `grep` / `glob`; no write, edit, or bash. Use it for "where is X" /
"list functions in Y" questions instead of stuffing large files into the main
context.

```bash
# single prompt (inline)
oi subagent run --prompt "列出 src/agent.rs 里的函数签名" --max-turns 5

# multiple prompts (parallel via std::thread::scope, 5min wall clock cap)
oi subagent run --prompt "X 在哪" --prompt "Y 怎么调" --max-turns 3
```

Limits (see `crates/subagent/src/config.rs`):

- 5-minute wall clock per `subagent run` call
- 128KB per-subagent output, with spill to `/tmp/oi-subagent-<pid>-<id>.txt`
- 10 turn loop cap per subagent (override with `--max-turns`)
