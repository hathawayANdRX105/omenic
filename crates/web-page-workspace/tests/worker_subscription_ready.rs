//! Worker 订阅就绪门回归测试。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use web_page_workspace::ReadinessGate;

#[test]
fn action_runs_only_after_subscription_is_ready() {
    let gate = ReadinessGate::new();
    let called = Arc::new(AtomicBool::new(false));
    let called_in_thread = Arc::clone(&called);
    let gate_in_thread = gate.clone();

    let waiter = std::thread::spawn(move || {
        gate_in_thread.run_when_ready(Duration::from_secs(1), || {
            called_in_thread.store(true, Ordering::SeqCst);
        })
    });

    std::thread::sleep(Duration::from_millis(20));
    assert!(
        !called.load(Ordering::SeqCst),
        "prompt must not run before subscribe"
    );

    gate.mark_ready();
    assert!(waiter.join().unwrap().is_ok());
    assert!(called.load(Ordering::SeqCst), "prompt runs after subscribe");
}

#[test]
fn disconnect_releases_an_existing_waiter_without_running_action() {
    let gate = ReadinessGate::new();
    let gate_in_thread = gate.clone();
    let started = Instant::now();

    let waiter = std::thread::spawn(move || {
        gate_in_thread.run_when_ready(Duration::from_secs(2), || {
            panic!("disconnected prompt must not run")
        })
    });

    std::thread::sleep(Duration::from_millis(20));
    gate.mark_not_ready();
    assert!(waiter.join().unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn timeout_does_not_run_action() {
    let gate = ReadinessGate::new();
    let called = AtomicBool::new(false);

    assert!(
        gate.run_when_ready(Duration::from_millis(10), || {
            called.store(true, Ordering::SeqCst);
        })
        .is_err()
    );
    assert!(!called.load(Ordering::SeqCst));
}
