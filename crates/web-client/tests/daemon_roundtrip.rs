//! WebDaemon ↔ daemon 端到端往返测试。
//!
//! 起一个真 daemon（临时目录 socket + 临时库，后台 accept 线程由
//! `Daemon::start` 自带，Drop 收尾），然后走 create → append → list →
//! load → search → delete 全链路；每步失败即 panic。另钉住
//! `update_session_title` 不吞 daemon 错误回复的契约。

use daemon::ClientError;
use daemon::{Daemon, DaemonConfig};
use web_client::daemon::WebDaemon;
use web_state::types::{PendingAttachment, SessionStatus};

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
        cwd: dir.path().to_path_buf(),
        max_turns: 64,
        ..Default::default()
    })
    .expect("启动 daemon");

    let wd = WebDaemon::connect_to(&socket);
    assert!(wd.ping(), "daemon ping 应成功");

    // create + append(user)
    wd.create_session("rt-1", "往返测试")
        .expect("create_session");
    wd.append_message("rt-1", true, "你好，daemon", &[])
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

/// `update_session_title` 必须把 daemon 的错误回复透传成 `Err`。
///
/// Red when: 封装退回 `call_raw`——它只解传输错误、不看 `success`，
/// 不存在会话的 `database_missing` 回复会变成 `Ok(())`，调用方
/// （page-workspace）的失败日志对这种情况成为死代码：标题更新失败
/// 与成功不可区分。
#[test]
fn update_session_title_surfaces_daemon_error() {
    let dir = tempfile::tempdir().expect("临时目录");
    let socket = dir.path().join("daemon.sock");
    let db_path = dir.path().join("sessions.db");

    let server = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: "omp".into(),
        session_db_path: Some(db_path),
        orbit_model: None,
        cwd: dir.path().to_path_buf(),
        max_turns: 64,
        ..Default::default()
    })
    .expect("启动 daemon");

    let wd = WebDaemon::connect_to(&socket);
    wd.create_session("rt-title", "占位标题")
        .expect("create_session");

    let err = wd
        .update_session_title("no-such-session", "新标题")
        .expect_err("重命名不存在的会话必须是 Err，而不是 Ok(())");
    match err {
        ClientError::Server { code, .. } => assert_eq!(
            code, "database_missing",
            "存储侧的 missing-row 错误必须透传到调用方"
        ),
        other => panic!("期望 Server 错误，实际: {other:?}"),
    }

    drop(server); // 停 accept 线程 + 清理 socket/lock 文件
}

/// 图片附件要跨 socket 往返：`session.append` 带上 `attachments`，
/// `session.load_messages` 读回的必须是同一条消息同三字段。
///
/// Red when: web-client 侧把 `attachments` 吞掉（只发 text），或 daemon
/// 侧解析成了空 vec —— 落库就断在第一步，resume 时模型看不到图。
#[test]
fn attachment_survives_socket_roundtrip() {
    let dir = tempfile::tempdir().expect("临时目录");
    let socket = dir.path().join("daemon.sock");
    let server = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: "omp".into(),
        session_db_path: Some(dir.path().join("sessions.db")),
        cwd: dir.path().to_path_buf(),
        max_turns: 64,
        ..Default::default()
    })
    .expect("启动 daemon");

    let wd = WebDaemon::connect_to(&socket);
    wd.create_session("att-1", "附件往返")
        .expect("create_session");
    let picked = vec![PendingAttachment {
        name: "shot.png".into(),
        media_type: "image/png".into(),
        data: "aGVsbG8=".into(),
    }];
    wd.append_message("att-1", true, "看这张图", &picked)
        .expect("append user with attachment");

    let msgs = wd.load_messages("att-1", 10).expect("load_messages");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].attachments.len(), 1, "附件应随消息落库");
    assert_eq!(msgs[0].attachments[0].name, "shot.png");
    assert_eq!(msgs[0].attachments[0].media_type, "image/png");
    assert_eq!(msgs[0].attachments[0].data, "aGVsbG8=");

    drop(server);
}

/// daemon 必须在 RPC 边界拒掉坏附件：非白名单 media type 不能进库，
/// 否则它会被拼进 provider 的 data: URL。
#[test]
fn daemon_rejects_non_image_attachment() {
    let dir = tempfile::tempdir().expect("临时目录");
    let socket = dir.path().join("daemon.sock");
    let server = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: "omp".into(),
        session_db_path: Some(dir.path().join("sessions.db")),
        cwd: dir.path().to_path_buf(),
        max_turns: 64,
        ..Default::default()
    })
    .expect("启动 daemon");

    let wd = WebDaemon::connect_to(&socket);
    wd.create_session("att-2", "坏附件")
        .expect("create_session");
    let bad = vec![PendingAttachment {
        name: "payload.pdf".into(),
        media_type: "application/pdf".into(),
        data: "aGk=".into(),
    }];
    let err = wd
        .append_message("att-2", true, "带坏附件", &bad)
        .expect_err("daemon 应拒绝非图片附件");
    assert!(
        err.to_string().contains("media type"),
        "错误应说明是 media type 问题，实际: {err}"
    );

    // 拒绝后库里不应留下这条消息。
    let msgs = wd.load_messages("att-2", 10).expect("load_messages");
    assert!(msgs.is_empty(), "被拒的消息不应落库");

    drop(server);
}
