//! Tests for guard components. Each test contains assertions for behavior, boundaries, and invariants per project rules.
//! No implementation details are asserted.

use omenic_harness_guard::{
    GuardService, RepeatConfig, RepeatToolReminder, TimeoutConfig, TimeoutPolicy,
};
use parking_lot::RwLock;
use serde_json::json;
use std::sync::Arc;

mod repeat_tests {
    use super::*;

    #[test]
    fn test_repeat_thresholds() {
        let config = RepeatConfig {
            thresholds: vec![3, 5, 8],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 500,
        };
        let reminder = RepeatToolReminder::new(config).unwrap();

        // Calls 1-2: no reminder
        assert!(
            reminder
                .observe("agent1", "tool1", &json!({"a": 1}))
                .is_none()
        );
        assert!(
            reminder
                .observe("agent1", "tool1", &json!({"a": 1}))
                .is_none()
        );

        // Call 3: first threshold (short message)
        let r1 = reminder.observe("agent1", "tool1", &json!({"a": 1}));
        assert!(r1.is_some());
        let msg1 = r1.unwrap();
        assert!(msg1.contains("You are repeating the exact same tool call"));

        // Call 4: no reminder
        assert!(
            reminder
                .observe("agent1", "tool1", &json!({"a": 1}))
                .is_none()
        );

        // Call 5: second threshold (detailed message)
        let r2 = reminder.observe("agent1", "tool1", &json!({"a": 1}));
        assert!(r2.is_some());
        let msg2 = r2.unwrap();
        assert!(msg2.contains("Repeated tool call detected:"));
        assert!(msg2.contains("tool: tool1"));
        assert!(msg2.contains("consecutive_calls: 5"));

        // Different arguments reset the chain: counts restart at 1, so the
        // first two calls of the new chain stay silent and the third
        // crosses threshold 3 again — exact-threshold semantics, same as
        // the reference (`if !THRESHOLDS.contains(&count) → None`).
        assert!(
            reminder
                .observe("agent1", "tool1", &json!({"b": 2}))
                .is_none()
        );
        assert!(
            reminder
                .observe("agent1", "tool1", &json!({"b": 2}))
                .is_none()
        );
        let r3 = reminder.observe("agent1", "tool1", &json!({"b": 2}));
        assert!(
            r3.is_some(),
            "a fresh chain crossing threshold 3 reminds again"
        );
        assert!(r3.unwrap().contains("consecutive_calls: 3"));

        // Call 8: third threshold
        let r3 = reminder.observe("agent1", "tool1", &json!({"b": 2}));
        assert!(r3.is_some());
        let msg3 = r3.unwrap();
        assert!(msg3.contains("consecutive_calls: 8"));
    }

    #[test]
    fn test_canonical_args_order_independence() {
        let config = RepeatConfig {
            thresholds: vec![2],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 500,
        };
        let reminder = RepeatToolReminder::new(config).unwrap();

        let args1 = json!({"a": 1, "b": 2});
        let args2 = json!({"b": 2, "a": 1});

        assert!(reminder.observe("agent1", "tool", &args1).is_none());
        let r = reminder.observe("agent1", "tool", &args2);
        assert!(r.is_some());
    }

    #[test]
    fn test_exclude_transparent() {
        let config = RepeatConfig {
            thresholds: vec![2],
            include: vec![],
            exclude: vec!["todo_write".to_string()],
            arguments_preview_chars: 500,
        };
        let reminder = RepeatToolReminder::new(config).unwrap();

        // Excluded tool is transparent - does not increment or reset chain
        reminder.observe("agent1", "todo_write", &json!({"x": 1}));
        assert!(
            reminder
                .observe("agent1", "grep", &json!({"x": 1}))
                .is_none()
        ); // count = 1
        let r = reminder.observe("agent1", "grep", &json!({"x": 1}));
        assert!(r.is_some()); // count = 2, triggers
    }

    #[test]
    fn test_include_pattern() {
        let config = RepeatConfig {
            thresholds: vec![2],
            include: vec!["mcp_*".to_string()],
            exclude: vec![],
            arguments_preview_chars: 500,
        };
        let reminder = RepeatToolReminder::new(config).unwrap();

        // Non-matching tool ignored
        assert!(
            reminder
                .observe("agent1", "bash", &json!({"x": 1}))
                .is_none()
        );
        assert!(
            reminder
                .observe("agent1", "bash", &json!({"x": 1}))
                .is_none()
        );

        // Matching mcp_* tool tracked
        assert!(
            reminder
                .observe("agent1", "mcp_fs", &json!({"x": 1}))
                .is_none()
        );
        let r = reminder.observe("agent1", "mcp_fs", &json!({"x": 1}));
        assert!(r.is_some());
    }

    #[test]
    fn test_argument_preview_truncation() {
        let config = RepeatConfig {
            // [2,3]: count 2 gets the gentle message, count 3 the detailed
            // template — only the detailed one carries the args preview.
            thresholds: vec![2, 3],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 10,
        };
        let reminder = RepeatToolReminder::new(config).unwrap();

        let long_args = json!({"big": "a".repeat(100)});
        let _ = reminder.observe("agent1", "tool", &long_args);
        let _ = reminder.observe("agent1", "tool", &long_args);
        let r = reminder.observe("agent1", "tool", &long_args);
        assert!(r.is_some());
        let msg = r.unwrap();
        assert!(msg.contains("… (+"));
        assert!(msg.contains("more chars)"));
    }

    #[test]
    fn test_config_validation_fail_loud() {
        // Empty thresholds
        let config = RepeatConfig {
            thresholds: vec![],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 1,
        };
        assert!(RepeatToolReminder::new(config).is_err());

        // Threshold below 2
        let config = RepeatConfig {
            thresholds: vec![1],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 1,
        };
        assert!(RepeatToolReminder::new(config).is_err());

        // Duplicates
        let config = RepeatConfig {
            thresholds: vec![2, 2],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 1,
        };
        assert!(RepeatToolReminder::new(config).is_err());

        // Not ascending
        let config = RepeatConfig {
            thresholds: vec![5, 3],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 1,
        };
        assert!(RepeatToolReminder::new(config).is_err());

        // Invalid preview chars
        let config = RepeatConfig {
            thresholds: vec![2],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 0,
        };
        assert!(RepeatToolReminder::new(config).is_err());
    }

    #[test]
    fn test_per_agent_isolation() {
        let config = RepeatConfig {
            thresholds: vec![2],
            include: vec![],
            exclude: vec![],
            arguments_preview_chars: 500,
        };
        let reminder = RepeatToolReminder::new(config).unwrap();

        // Agent1 and Agent2 have independent chains
        reminder.observe("agent1", "tool", &json!({"x": 1}));
        reminder.observe("agent1", "tool", &json!({"x": 1})); // triggers for agent1

        assert!(
            reminder
                .observe("agent2", "tool", &json!({"x": 1}))
                .is_none()
        );
        let r = reminder.observe("agent2", "tool", &json!({"x": 1}));
        assert!(r.is_some()); // triggers for agent2 independently
    }
}

mod timeout_tests {
    use super::*;

    #[test]
    fn test_timeout_deadline_for_matching_pattern() {
        let config = TimeoutConfig {
            rules: vec![("web_fetch".to_string(), 5), ("*.py".to_string(), 10)]
                .into_iter()
                .collect(),
        };
        let policy = TimeoutPolicy::new(config).unwrap();

        assert_eq!(
            policy.deadline_for("web_fetch"),
            Some(std::time::Duration::from_secs(5))
        );
        assert_eq!(
            policy.deadline_for("script.py"),
            Some(std::time::Duration::from_secs(10))
        );
        assert_eq!(policy.deadline_for("unknown"), None);
    }

    #[test]
    fn test_timeout_glob_wildcard() {
        let config = TimeoutConfig {
            rules: vec![("*.py".to_string(), 3), ("*".to_string(), 1)]
                .into_iter()
                .collect(),
        };
        let policy = TimeoutPolicy::new(config).unwrap();

        assert_eq!(
            policy.deadline_for("script.py"),
            Some(std::time::Duration::from_secs(3))
        );
        assert_eq!(
            policy.deadline_for("any_tool"),
            Some(std::time::Duration::from_secs(1))
        );
    }

    #[test]
    fn test_timeout_config_validation_fail_loud() {
        let config = TimeoutConfig {
            rules: vec![("web_fetch".to_string(), 0)].into_iter().collect(),
        };
        assert!(TimeoutPolicy::new(config).is_err());

        let config = TimeoutConfig {
            rules: vec![("web_fetch".to_string(), 5)].into_iter().collect(),
        };
        assert!(TimeoutPolicy::new(config).is_ok());
    }

    #[test]
    fn test_timeout_rules_snapshot() {
        let config = TimeoutConfig {
            rules: vec![("tool1".to_string(), 10), ("tool2".to_string(), 20)]
                .into_iter()
                .collect(),
        };
        let policy = TimeoutPolicy::new(config).unwrap();

        let rules = policy.rules();
        assert_eq!(rules.get("tool1"), Some(&10));
        assert_eq!(rules.get("tool2"), Some(&20));
    }
}

#[test]
fn test_guard_service_integration() {
    let repeat_config = RepeatConfig {
        thresholds: vec![2],
        include: vec![],
        exclude: vec![],
        arguments_preview_chars: 500,
    };
    let repeat = RepeatToolReminder::new(repeat_config).unwrap();

    let timeout_config = TimeoutConfig {
        rules: vec![("test_tool".to_string(), 3)].into_iter().collect(),
    };
    let timeout = TimeoutPolicy::new(timeout_config).unwrap();

    let service = GuardService {
        reminder: repeat,
        timeout,
        snapshot: Arc::new(RwLock::new(vec![])),
    };

    assert!(
        service
            .check_repeat("agent1", "test", &json!({"a": 1}))
            .is_none()
    );
    assert!(
        service
            .check_repeat("agent1", "test", &json!({"a": 1}))
            .is_some()
    );

    assert_eq!(
        service.deadline_for("test_tool"),
        Some(std::time::Duration::from_secs(3))
    );
    assert_eq!(service.deadline_for("other_tool"), None);

    let snapshot = service.snapshot();
    assert!(snapshot.is_empty());
}
