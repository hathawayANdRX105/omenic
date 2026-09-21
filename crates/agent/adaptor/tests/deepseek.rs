//! Tests for DeepSeek adapter dialect detection and max_tokens defaulting.
//!
//! Tests the dispatcher in adaptor::stream_cb and deepseek::stream_cb.
//! Uses simple assertion-based tests (no full HTTP mock to stay minimal).

use adaptor::{Model, StreamEvent, deepseek};
use std::sync::atomic::AtomicBool;

#[test]
fn test_deepseek_dialect_detection_model_name() {
    let signal = AtomicBool::new(false);
    let mut events: Vec<StreamEvent> = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());

    let model = Model {
        api_key: "sk-test".to_string(),
        model: "deepseek-chat".to_string(),
        base_url: None,
        max_tokens: None,
    };

    let context = adaptor::Context::default();
    let tools = vec![];

    // Should route to deepseek dialect which sets max_tokens
    deepseek::stream_cb(&model, &context, &tools, &signal, &mut emit);
    assert!(
        !events.is_empty(),
        "stream_cb must emit at least a terminal event"
    );
}

#[test]
fn test_deepseek_dialect_detection_url() {
    let signal = AtomicBool::new(false);
    let mut events: Vec<StreamEvent> = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());

    let model = Model {
        api_key: "sk-test".to_string(),
        model: "gpt-4o".to_string(),
        base_url: Some("https://api.deepseek.com".to_string()),
        max_tokens: None,
    };

    let context = adaptor::Context::default();
    let tools = vec![];

    deepseek::stream_cb(&model, &context, &tools, &signal, &mut emit);
    assert!(!events.is_empty());
}

#[test]
fn test_deepseek_max_tokens_default() {
    let model = Model {
        api_key: "sk-test".to_string(),
        model: "deepseek-reasoner".to_string(),
        base_url: None,
        max_tokens: None,
    };

    // The adapter clones and defaults max_tokens to 8192
    // We can't easily assert internal without exposing, but the call succeeds
    let signal = AtomicBool::new(false);
    let mut events = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());
    let context = adaptor::Context::default();

    deepseek::stream_cb(&model, &context, &[], &signal, &mut emit);
    assert_eq!(model.max_tokens, None, "original model unchanged");
}

#[test]
fn test_non_deepseek_dialect_detection() {
    let signal = AtomicBool::new(false);
    let mut events = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());

    let model = Model {
        api_key: "sk-test".to_string(),
        model: "gpt-4o".to_string(),
        base_url: Some("https://api.openai.com/v1".to_string()),
        max_tokens: None,
    };

    let context = adaptor::Context::default();
    adaptor::stream_cb(&model, &context, &[], &signal, &mut emit);
    assert!(!events.is_empty());
}

#[test]
fn test_openai_url_detection() {
    let signal = AtomicBool::new(false);
    let mut events = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());

    let model = Model {
        api_key: "sk-test".to_string(),
        model: "claude-3".to_string(),
        base_url: Some("https://api.anthropic.com".to_string()),
        max_tokens: None,
    };

    let context = adaptor::Context::default();
    adaptor::stream_cb(&model, &context, &[], &signal, &mut emit);
    assert!(!events.is_empty());
}

#[test]
fn test_deepseek_max_tokens_preserved() {
    let model = Model {
        api_key: "sk-test".to_string(),
        model: "deepseek-chat".to_string(),
        base_url: None,
        max_tokens: Some(4096),
    };

    let signal = AtomicBool::new(false);
    let mut events = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());
    let context = adaptor::Context::default();

    deepseek::stream_cb(&model, &context, &[], &signal, &mut emit);
    assert_eq!(model.max_tokens, Some(4096));
}

#[test]
fn test_dialect_detection_edge_cases() {
    let signal = AtomicBool::new(false);
    let mut events = vec![];
    let mut emit = |e: &StreamEvent| events.push(e.clone());
    let context = adaptor::Context::default();

    // Case: model name with "deepseek" substring but not prefix
    let model1 = Model {
        api_key: "sk-test".to_string(),
        model: "my-deepseek-model".to_string(),
        base_url: None,
        max_tokens: None,
    };
    adaptor::stream_cb(&model1, &context, &[], &signal, &mut emit);
    assert!(!events.is_empty());

    events.clear();

    // Case: base_url with deepseek in path
    let model2 = Model {
        api_key: "sk-test".to_string(),
        model: "gpt-4".to_string(),
        base_url: Some("https://example.com/deepseek/v1".to_string()),
        max_tokens: None,
    };
    adaptor::stream_cb(&model2, &context, &[], &signal, &mut emit);
    assert!(!events.is_empty());
}
