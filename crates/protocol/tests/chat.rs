//! serde round-trip for the `chat` DTO mirror, including `Block::Image`.

use protocol::chat::{Block, Content, Message, Role};

#[test]
fn message_with_image_block_round_trips() {
    let msg = Message {
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
    };
    let s = serde_json::to_string(&msg).unwrap();
    assert!(
        s.contains("\"type\":\"image\""),
        "image block must serialize with tag \"image\", got: {s}"
    );
    let back: Message = serde_json::from_str(&s).unwrap();
    assert_eq!(
        msg, back,
        "Image block must survive a JSON round-trip losslessly"
    );
}
