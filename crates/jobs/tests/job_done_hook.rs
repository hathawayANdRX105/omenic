//! `on_job_done` 钩子：作业进入终态时恰好触发一次，携带 summary；钩子在
//! 注册表锁之外运行（可回调注册表）。
//!
//! Red when: 钩子在锁内跑（回调注册表即死锁），或终态多次触发（模型收到
//! 重复的"作业完成"通知）。

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jobs::{JobOutput, JobRegistry, LocalJobRegistry};

#[test]
fn hook_fires_once_with_the_finished_summary() {
    let reg = LocalJobRegistry::new();
    let (tx, rx) = mpsc::channel();
    reg.on_job_done(Arc::new(move |summary| {
        let _ = tx.send((
            summary.id.as_str().to_string(),
            summary.state,
            summary.exit_code,
        ));
    }));

    let id = reg
        .start(
            "build",
            Box::new(|_| {
                Ok(JobOutput {
                    exit_code: Some(0),
                    ..JobOutput::default()
                })
            }),
        )
        .expect("start");
    let (seen_id, state, code) = rx.recv_timeout(Duration::from_secs(5)).expect("hook fires");
    assert_eq!(seen_id, id.as_str());
    assert_eq!(state.to_string(), "completed");
    assert_eq!(code, Some(0));

    // No second delivery.
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "hook must fire exactly once per job"
    );
}

#[test]
fn hook_may_call_back_into_the_registry() {
    // Proves the hook runs outside the registry lock: a hook that queries the
    // registry would deadlock if it were invoked under the state mutex.
    let reg = Arc::new(LocalJobRegistry::new());
    let reg2 = Arc::clone(&reg);
    let (tx, rx) = mpsc::channel();
    reg.on_job_done(Arc::new(move |summary| {
        let listed = reg2.list();
        let _ = tx.send((listed.len(), summary.label.clone()));
    }));

    reg.start("label-a", Box::new(|_| Ok(JobOutput::default())))
        .expect("start");
    let (count, label) = rx.recv_timeout(Duration::from_secs(5)).expect("hook fires");
    assert_eq!(count, 1, "hook could re-enter the registry");
    assert_eq!(label, "label-a");
}

#[test]
fn failed_job_reports_failed_state() {
    let reg = LocalJobRegistry::new();
    let (tx, rx) = mpsc::channel();
    reg.on_job_done(Arc::new(move |s| {
        let _ = tx.send((s.state, s.exit_code));
    }));
    reg.start("boom", Box::new(|_| Err("kaboom".into())))
        .expect("start");
    let (state, code) = rx.recv_timeout(Duration::from_secs(5)).expect("hook fires");
    assert_eq!(state.to_string(), "failed");
    assert_eq!(code, None);
}
