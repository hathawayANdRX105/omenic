//! Deterministic tests for the worker subscription readiness gate.
//! Proves blocked-before-ready, release-after-ready, reset requires another
//! notification, and timeout behavior.

use omenic_web_page_workspace::ReadinessGate;
use std::time::Duration; // from lib.rs

#[test]
fn blocked_before_ready() {
    let gate = ReadinessGate::new();
    // Should not be ready initially
    let result = gate.wait_ready(Duration::from_millis(50));
    assert!(result.is_err()); // blocked until timeout
}

#[test]
fn release_after_ready() {
    let gate = ReadinessGate::new();
    gate.mark_ready();
    let result = gate.wait_ready(Duration::from_millis(50));
    assert!(result.is_ok());
}

#[test]
fn reset_requires_another_notification() {
    let gate = ReadinessGate::new();
    gate.mark_ready();
    let result1 = gate.wait_ready(Duration::from_millis(10));
    assert!(result1.is_ok());

    gate.mark_not_ready();
    let result2 = gate.wait_ready(Duration::from_millis(10));
    assert!(result2.is_err());

    gate.mark_ready();
    let result3 = gate.wait_ready(Duration::from_millis(10));
    assert!(result3.is_ok());
}

#[test]
fn timeout_false() {
    let gate = ReadinessGate::new();
    let result = gate.wait_ready(Duration::from_millis(5));
    assert!(result.is_err());
}

#[test]
fn test_worker_subscription_ready_integration() {
    // Integration test for full lifecycle
    let gate = ReadinessGate::new();
    // Simulate worker_event_loop ready
    gate.mark_ready();
    // Simulate prompt thread wait
    assert!(gate.wait_ready(Duration::from_millis(100)).is_ok());
    // Simulate disconnect
    gate.mark_not_ready();
    assert!(gate.wait_ready(Duration::from_millis(5)).is_err());
}
