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

## Daemon

本地可重连的 Agent daemon：后台进程持有会话与运行记录，CLI 是薄客户端，断开重连不丢状态。

```bash
oi daemon start    # 未在运行则拉起 daemon 进程（sibling 二进制，或 OMENIC_DAEMON_PATH 指定）
oi daemon status   # 存活 / pid / uptime
oi daemon stop     # 有序退出（SIGTERM 同样有效）

oi session attach <id> [--limit N]        # 查看会话摘要 + 最近消息
oi session resume <id> "follow-up prompt" # 向 worker 续发 prompt，run 记入 ledger（run.list 可查）
```

路径与环境变量：

- 配置文件：`.oi/config.toml`（或 `omenic.toml`）；`OMENIC_DATA_DIR` 改数据目录
- socket：`$XDG_CONFIG_HOME/omenic/daemon.sock`，`OMENIC_DAEMON_SOCKET` 覆盖
- 会话数据库：`$XDG_CONFIG_HOME/omenic/sessions.db`，`OMENIC_SESSION_DB` 覆盖
- worker 后端：`OMENIC_OMP_PATH` 指定 omp 可执行文件（缺省时 prompt 会失败并在 ledger 记 `spawn_failed`）

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
