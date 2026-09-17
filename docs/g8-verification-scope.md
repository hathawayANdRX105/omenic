# G8 Verification Scope

Base: `a91caee` (already on `main` as `71a0889`)
Branch: `fix/g8-verify-rebased`

## G8-A: run 收尾过早
- daemon event pump 在 `AgentEnd` 才 finish run
- `stop_reason` 通过 `AgentEnd` 透传
- pump 在 orbit prompt 前启动

Files:
- `crates/infra/daemon/src/dispatch.rs`
- `crates/infra/daemon/src/state.rs`
- `crates/infra/rpc/src/worker.rs`

## G8-B: 配置写回抹段
- `toml_edit` 增量写回
- 未管理段/注释/排版逐字节保留

Files:
- `crates/web/client/src/llm.rs`

## G8-C: spill 文件碰撞
- `oi-output-{pid}-{seq}.txt`
- pid 隔进程、seq 进程内单调

Files:
- `crates/agent/tools/src/lib.rs`

## Tests
- `crates/infra/daemon/tests/run_lifecycle.rs`
- `crates/infra/rpc/tests/subscribe_pump.rs`
- `crates/web/client/tests/config_roundtrip.rs`
- `crates/agent/tools/tests/truncate.rs`
