//! WebDaemon ↔ daemon 端到端往返测试。
//!
//! 起一个真 daemon（临时目录 socket + 临时库，后台 accept 线程由
//! `Daemon::start` 自带，Drop 收尾），然后走 create → append → list →
//! load → search → delete 全链路；每步失败即 panic。

use daemon::{Daemon, DaemonConfig};
use omenic_web_client::daemon::WebDaemon;
use omenic_web_state::types::SessionStatus;

#[test]
fn daemon_roundtrip() {
    let dir = tempfile::tempdir().expect("临时目录");
    let socket = dir.path().join("daemon.sock");
    let db_path = dir.path().join("sessions.db");

    // 临时 socket + 后台 serve：accept 循环跑在 Daemon 自己的线程里
    let server = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: "omp".into(),
        session_db_path: Some(db_path),
        orbit_model: None,
    })
    .expect("启动 daemon");

    let wd = WebDaemon::connect_to(&socket);
    assert!(wd.ping(), "daemon ping 应成功");

    // create + append(user)
    wd.create_session("rt-1", "往返测试")
        .expect("create_session");
    wd.append_message("rt-1", true, "你好，daemon")
        .expect("append user");

    // list 断言含该会话（存储无状态概念 → Idle）
    let sessions = wd.list_sessions(50).expect("list_sessions");
    let rt = sessions
        .iter()
        .find(|s| s.id == "rt-1")
        .expect("list 应含新建会话");
    assert_eq!(rt.title, "往返测试");
    assert_eq!(rt.status, SessionStatus::Idle);

    // load_messages 断言文本 / role / 时间戳
    let msgs = wd.load_messages("rt-1", 100).expect("load_messages");
    assert_eq!(msgs.len(), 1, "应恰好一条消息");
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[0].content, "你好，daemon");
    assert!(msgs[0].ts_epoch_ms > 0);

    // search 命中 → 按 session 去重映射回 Session 列表
    let hits = wd.search_sessions("daemon", 10).expect("search_sessions");
    assert!(hits.iter().any(|s| s.id == "rt-1"), "search 应命中 rt-1");

    // delete 后 list 断言为空
    wd.delete_session("rt-1").expect("delete_session");
    let sessions = wd.list_sessions(50).expect("list after delete");
    assert!(
        sessions.is_empty(),
        "删除后 list 应为空，实际: {sessions:?}"
    );

    drop(server); // 停 accept 线程 + 清理 socket/lock 文件
}
