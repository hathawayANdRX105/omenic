//! `Block::Image` → OpenAI multimodal message conversion.
//!
//! Pure-function tests over [`llm::openai::context_to_openai_messages`] — no
//! network.

use llm::openai::context_to_openai_messages;
use llm::{Block, Content, Context, Message, Role};
use serde_json::json;

fn ctx(messages: Vec<Message>) -> Context {
    Context {
        system_prompt: None,
        messages,
    }
}

#[test]
fn user_image_block_becomes_image_url_part() {
    let messages = context_to_openai_messages(&ctx(vec![Message {
        role: Role::User,
        content: Content::Blocks(vec![
            Block::Text {
                text: "看图".to_string(),
            },
            Block::Image {
                media_type: "image/png".to_string(),
                data: "AAA".to_string(),
            },
        ]),
    }]));
    assert_eq!(messages.len(), 1, "one user message in, one out");
    let content = &messages[0]["content"];
    assert!(
        content.is_array(),
        "user message with an image must use the multimodal content-array form"
    );
    // Text part first, then the image_url part carrying the full data URL.
    assert_eq!(content[0]["type"], json!("text"));
    assert_eq!(content[0]["text"], json!("看图"));
    assert_eq!(content[1]["type"], json!("image_url"));
    assert_eq!(
        content[1]["image_url"]["url"],
        json!("data:image/png;base64,AAA")
    );
}

#[test]
fn user_text_message_stays_a_string() {
    let messages = context_to_openai_messages(&ctx(vec![Message {
        role: Role::User,
        content: Content::Text("just text".to_string()),
    }]));
    assert_eq!(
        &messages[0],
        &json!({ "role": "user", "content": "just text" }),
        "plain user text must stay string content, not a parts array"
    );
}

#[test]
fn user_blocks_without_images_stay_a_string() {
    let messages = context_to_openai_messages(&ctx(vec![Message {
        role: Role::User,
        content: Content::Blocks(vec![
            Block::Text {
                text: "a".to_string(),
            },
            Block::Text {
                text: "b".to_string(),
            },
        ]),
    }]));
    assert_eq!(
        &messages[0],
        &json!({ "role": "user", "content": "a\nb" }),
        "image-less block messages keep string content with \\n-joined text"
    );
}

#[test]
fn assistant_image_block_is_dropped() {
    let messages = context_to_openai_messages(&ctx(vec![Message {
        role: Role::Assistant,
        content: Content::Blocks(vec![
            Block::Text {
                text: "看".to_string(),
            },
            Block::Image {
                media_type: "image/png".to_string(),
                data: "AAA".to_string(),
            },
        ]),
    }]));
    // Must not panic, and no image_url part may leak onto assistant turns.
    assert_eq!(messages.len(), 1);
    let s = messages[0].to_string();
    assert!(
        !s.contains("image_url"),
        "assistant turns carry no images: {s}"
    );
    assert_eq!(
        &messages[0],
        &json!({ "role": "assistant", "content": "看" }),
        "assistant text stays a string when images are dropped"
    );
}
