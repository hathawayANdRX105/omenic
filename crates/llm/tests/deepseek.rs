//! DeepSeek dialect: detection heuristic and max_tokens defaulting.
//!
//! Pure-function tests — no network, no stub server.

use llm::{Model, deepseek};

fn model(name: &str, base_url: Option<&str>) -> Model {
    Model {
        api_key: "sk-test".to_string(),
        model: name.to_string(),
        base_url: base_url.map(str::to_string),
        max_tokens: None,
    }
}

#[test]
fn deepseek_model_names_take_the_dialect() {
    assert!(deepseek::is_deepseek_model(&model("deepseek-chat", None)));
    assert!(deepseek::is_deepseek_model(&model(
        "deepseek-reasoner",
        Some("https://example.com/v1")
    )));
}

#[test]
fn deepseek_base_url_takes_the_dialect() {
    assert!(deepseek::is_deepseek_model(&model(
        "some-other-model",
        Some("https://api.deepseek.com")
    )));
    assert!(deepseek::is_deepseek_model(&model(
        "some-other-model",
        Some("https://proxy.internal/deepseek/v1")
    )));
}

#[test]
fn plain_openai_models_stay_on_the_openai_dialect() {
    assert!(!deepseek::is_deepseek_model(&model(
        "gpt-4o",
        Some("https://api.openai.com/v1")
    )));
    assert!(!deepseek::is_deepseek_model(&model("gpt-4o", None)));
}

#[test]
fn max_tokens_defaults_when_absent() {
    assert_eq!(
        deepseek::effective_max_tokens(&model("deepseek-chat", None)),
        deepseek::DEEPSEEK_DEFAULT_MAX_TOKENS
    );
}

#[test]
fn max_tokens_default_does_not_override_caller_value() {
    let mut m = model("deepseek-chat", None);
    m.max_tokens = Some(2048);
    assert_eq!(deepseek::effective_max_tokens(&m), 2048);
}
