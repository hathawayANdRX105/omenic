use omenic_harness_core::Message;
use omenic_harness_prompt::PromptInput;

#[test]
fn prompt_input_fields() {
    let input = PromptInput {
        system: "You are a helpful assistant.".to_string(),
        user: "Hello".to_string(),
        history: Vec::new(),
    };
    assert_eq!(input.system, "You are a helpful assistant.");
    assert_eq!(input.user, "Hello");
    assert!(input.history.is_empty());
}

#[test]
fn prompt_input_with_history() {
    let input = PromptInput {
        system: "sys".to_string(),
        user: "user".to_string(),
        history: vec![Message {
            role: "assistant".to_string(),
            content: "prev".to_string(),
        }],
    };
    assert_eq!(input.history.len(), 1);
    assert_eq!(input.history[0].role, "assistant");
}

#[test]
fn prompt_template_creation() {
    use omenic_harness_prompt::PromptTemplate;
    let tpl = PromptTemplate::new("test", "body {{system}}");
    assert_eq!(tpl.name, "test");
    assert_eq!(tpl.body, "body {{system}}");
}
