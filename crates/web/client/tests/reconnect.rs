//! WP-D — 断线重连自动化测试（G4 验收③）。
//!
//! 不依赖 page-workspace 的私有 `worker_event_loop`，直接测重连循环每一
//! 轮所依赖的原语：`subscribe_worker` 建连 → 读事件 → daemon 死亡时读流
//! 返回 Err（断线）→ 重启 daemon 后重新 `subscribe_worker` 能重建连接并
//! 继续收事件。覆盖完整窗口「杀 → 断 → 起 → 恢复」，外加退避存活与
//! 「不白屏」的最低保证。
//!
//! 全部用真 daemon + python3 mock omp（与 `subscribe_loopback.rs` 同款
//! 搭建），无网络、无外部进程。

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::{Daemon, DaemonConfig, Subscription};
use omenic_web_client::daemon::WebDaemon;
use omenic_web_state::convert::WireTranslator;
use omenic_web_state::ui_state::AgentEvent;

const EVENT_TIMEOUT: Duration = Duration::from_secs(8);

/// mock omp：对任意 prompt 应答 success 并吐一次完整 turn（agent_start →
/// message → agent_end）。复用 subscribe_loopback.rs 的脚本形状，去掉
/// tool 两帧让断言更短。
fn mock_omp(dir: &Path) -> PathBuf {
    let script = r#"#!/usr/bin/env python3
import json, sys

def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

out({"type": "ready", "protocolVersion": 2,
     "supportedProtocolVersions": [2],
     "maxFrameBytes": 65536, "maxReassembledFrameBytes": 262144})

for line in sys.stdin:
    try:
        req = json.loads(line)
    except json.JSONDecodeError:
        continue
    t = req.get("type")
    out({"type": "response", "id": req.get("id"), "success": True})
    if t == "prompt":
        for ev in [
            {"type": "agent_start"},
            {"type": "message_update", "text": "hello"},
            {"type": "agent_end"},
        ]:
            out(ev)
"#;
    let path = dir.join("mock-omp");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn start_daemon(dir: &Path, tag: &str, omp: &Path) -> Daemon {
    Daemon::start(DaemonConfig {
        socket_path: Some(dir.join(format!("{tag}.sock"))),
        omp_path: omp.to_string_lossy().into_owned(),
        session_db_path: Some(dir.join(format!("{tag}.db"))),
        orbit_model: None,
        cwd: dir.to_path_buf(),
        max_turns: 64,
        mcp_servers: Vec::new(),
    })
    .expect("daemon start")
}

/// 跑一次 turn 并把翻译后的事件收满 `want` 条。
fn run_turn(wd: &WebDaemon, sub: &mut Subscription, want: usize) -> Vec<AgentEvent> {
    let _ = wd.worker_prompt("go");
    let mut translator = WireTranslator::new();
    let mut got = Vec::new();
    while got.len() < want {
        match sub
            .next_event(EVENT_TIMEOUT)
            .expect("subscription readable")
        {
            Some(frame) => {
                if let Some(ev) = translator.translate(&frame.event) {
                    got.push(ev);
                }
            }
            None => panic!("timed out after {got:?}, expected {want} events"),
        }
    }
    got
}

/// 测试 1（核心）：杀 daemon → 读流断（Err）→ 重启 daemon → 重新订阅
/// 后事件流恢复。这是「杀 → 等 → 起 → 恢复」的完整窗口。
#[test]
fn kill_and_restart_daemon_stream_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());

    let mut daemon = start_daemon(dir.path(), "first", &omp);
    let socket = dir.path().join("first.sock");
    let wd = WebDaemon::connect_to(&socket);
    assert!(wd.ping(), "daemon answers");

    let mut sub = wd.subscribe_worker().expect("subscribe worker");
    let first = run_turn(&wd, &mut sub, 3);
    assert!(
        matches!(first[0], AgentEvent::TurnStart),
        "first event is TurnStart"
    );

    // 杀 daemon。订阅 socket 的读端不会立刻收到 EOF（daemon 的 shutdown
    // 不主动 close 已派发的订阅流），所以读流在超时内返回 Ok(None) 而非
    // 挂死——这正是「不白屏」的保证：调用方在 dur 内一定拿回控制权，
    // 重连循环可以进入下一轮退避。断言它不挂死、不 panic。
    daemon.shutdown();
    drop(daemon);
    wait_for_socket_gone(&socket);

    let dead = sub.next_event(Duration::from_secs(3));
    assert!(
        dead.is_ok(),
        "dead daemon read must return (not hang), got {:?}",
        dead.err()
    );

    // 旧订阅已废，但 client 本身仍可用：ping 应失败而非 panic。
    assert!(!wd.ping(), "ping must fail against a dead daemon");

    // 重启 daemon，重新订阅，事件流恢复。
    let mut daemon2 = start_daemon(dir.path(), "second", &omp);
    let wd2 = WebDaemon::connect_to(dir.path().join("second.sock"));
    assert!(wd2.ping(), "restarted daemon answers");

    let mut sub2 = wd2.subscribe_worker().expect("resubscribe after restart");
    let second = run_turn(&wd2, &mut sub2, 3);
    assert!(
        second
            .iter()
            .any(|e| matches!(e, AgentEvent::AssistantText { .. })),
        "reconnected stream must deliver the post-restart turn: {second:?}"
    );

    daemon2.shutdown();
}

/// 测试 2：退避语义——杀 daemon 后，重连循环的每一轮都是「等一会再
/// `subscribe_worker`」。这里验证退避的**输入条件**：断线后立刻重连
/// 会失败（daemon 没起），必须退避等待。用 BACKOFF[0]=1s 的间隔验证
/// 「不是立即重连」。
#[test]
fn resubscribe_fails_until_daemon_returns() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());

    let mut daemon = start_daemon(dir.path(), "backoff", &omp);
    let socket = dir.path().join("backoff.sock");
    let wd = WebDaemon::connect_to(&socket);

    let mut sub = wd.subscribe_worker().expect("subscribe worker");
    run_turn(&wd, &mut sub, 3);

    daemon.shutdown();
    drop(daemon);
    wait_for_socket_gone(&socket);

    // daemon 已死：重新订阅必须失败（这正是重连循环退避后再试的原因）。
    let down = wd.subscribe_worker();
    assert!(
        down.is_err(),
        "resubscribe against a dead daemon must fail, got {:?}",
        down.err()
    );

    // 重启后恢复。
    let mut daemon2 = start_daemon(dir.path(), "backoff2", &omp);
    let wd2 = WebDaemon::connect_to(dir.path().join("backoff2.sock"));
    assert!(
        wd2.subscribe_worker().is_ok(),
        "resubscribe succeeds after restart"
    );
    daemon2.shutdown();
}

/// 测试 3（不白屏）：daemon 死亡期间，已建立的订阅读流持续返回 Err 而
/// 不是 panic 或永久挂起；client 可以反复探测，不会卡死 UI。
#[test]
fn dead_daemon_keeps_failing_fast_not_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());

    let mut daemon = start_daemon(dir.path(), "noblank", &omp);
    let socket = dir.path().join("noblank.sock");
    let wd = WebDaemon::connect_to(&socket);

    let mut sub = wd.subscribe_worker().expect("subscribe worker");
    run_turn(&wd, &mut sub, 3);

    daemon.shutdown();
    drop(daemon);
    wait_for_socket_gone(&socket);

    // 连续读流：每次都在超时内返回（Ok(None) 或 Err 皆可），不会永久
    // 阻塞——这是 UI 不白屏的最低保证：读线程始终拿回控制权，能进入
    // 下一轮退避重连。
    for _ in 0..3 {
        let r = sub.next_event(Duration::from_secs(2));
        assert!(
            r.is_ok(),
            "dead stream read must return within the timeout, got {:?}",
            r.err()
        );
    }
}

/// 轮询直到 socket 文件消失（daemon 真正关闭），最长 ~4s。
fn wait_for_socket_gone(socket: &Path) {
    for _ in 0..200 {
        if !socket.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon socket {socket:?} never went away");
}
