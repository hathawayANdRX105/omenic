use prompt::PromptInput;
use protocol::Message;

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
    use prompt::PromptTemplate;
    let tpl = PromptTemplate::new("test", "body {{system}}");
    assert_eq!(tpl.name, "test");
    assert_eq!(tpl.body, "body {{system}}");
}

#[test]
fn render_substitutes_all_three_slots() {
    use prompt::{PromptInput, render_with_template};
    use protocol::Message;
    let input = PromptInput {
        system: "be terse".to_string(),
        user: "hi".to_string(),
        history: vec![
            Message {
                role: "user".to_string(),
                content: "earlier".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "noted".to_string(),
            },
        ],
    };
    let out = render_with_template("S:{{system}}|H:{{history}}|U:{{user}}", &input);
    assert!(out.starts_with("S:be terse|H:"), "got: {out}");
    assert!(out.ends_with("|U:hi"), "got: {out}");
    // {{history}} is OpenAI-format messages JSON.
    let json = &out["S:be terse|H:".len()..out.len() - "|U:hi".len()];
    let parsed: Vec<serde_json::Value> = serde_json::from_str(json).expect("valid JSON");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0]["role"], "user");
    assert_eq!(parsed[0]["content"], "earlier");
    assert_eq!(parsed[1]["role"], "assistant");
    assert_eq!(parsed[1]["content"], "noted");
}

#[test]
fn render_leaves_template_without_placeholders_untouched() {
    use prompt::{PromptInput, render_with_template};
    let out = render_with_template("static text", &PromptInput::default());
    assert_eq!(out, "static text");
    // Absent slots render as empty, not literal braces.
    let out = render_with_template("[{{user}}]", &PromptInput::default());
    assert_eq!(out, "[]");
}
