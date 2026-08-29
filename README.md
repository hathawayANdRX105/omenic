# omenic

<!-- demo note (issue #75): compliance gate deployed -->
Omenic is a task-driven agent orchestrator where agents act as functions following Prompt → Result.

## Commands

`cargo install --path .` installs both `omenic` and the short alias `cli`.

```bash
oi plan
oi task add "implement login"
```

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
