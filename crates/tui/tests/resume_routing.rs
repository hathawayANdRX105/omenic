//! resume_routing — route §3 T4 数据完整性红线：切会话后 prompt 必带**新**
//! `session_id` 与**新** `run_id`，旧 run 的事件按 `run_id` 过滤、不进新
//! 会话视图。对应 route §4 的 `prompt_routes_to_new_session_after_switch`
//! 与 `history_scroll_bounds_clamp`（回填历史的滚动边界）。
//!
//! `App` 是无 IO 纯状态机：出站走 [`PromptDelivery`] 三元组、事件准入走
//! `apply_run_event(run_id, ev)`——这正是事件循环消费的两个接缝，断言它们
//! 就钉住了「消息写进别的会话」那类事故的路由面。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use omenic_tui::app::{App, PromptDelivery};
use omenic_web_state::types::{ChatMessage, MessagePart};
use omenic_web_state::ui_state::AgentEvent;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 打字 + Enter（进队列，不出站）。
fn submit(app: &mut App, text: &str) {
    type_str(app, text);
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        omenic_tui::app::KeyAction::None
    );
}

fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        id: format!("m-{text}"),
        role: "user".to_string(),
        content: text.to_string(),
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![MessagePart::Text(text.to_string())],
        timestamp: String::new(),
        ts_epoch_ms: 0,
    }
}

/// 红线测试：切会话后出站 prompt 的 `session_id` 必须换成新会话、
/// `run_id` 必须是新号；旧 run_id 的事件帧被拒收，新 run_id 放行。
#[test]
fn prompt_routes_to_new_session_after_switch() {
    let mut app = App::new();
    app.start_session("s-1", Vec::new());
    assert!(!app.run_scoped(), "首会话仍走会话级事件流（T2 语义）");

    // 第一条 prompt 归 s-1。
    submit(&mut app, "first");
    let first: PromptDelivery = app.take_prompt().expect("非空 prompt 必须出站");
    assert_eq!(first.session_id, "s-1");
    assert!(first.run_id.starts_with("r-"), "run_id 须是 r-<epoch> 形态");
    let old_run = first.run_id.clone();
    assert_eq!(app.current_run(), Some(old_run.as_str()));
    app.note_turn_end();
    assert_eq!(app.current_run(), None, "turn 收尾后无在飞 run");

    // 切会话：新 id + `load_messages` 回填 + 事件准入切 run 作用域。
    app.switch_session("s-2", vec![user_msg("older history")]);
    assert_eq!(app.session_id(), "s-2");
    assert!(app.run_scoped(), "切会话后事件必须按 run_id 过滤");
    assert_eq!(app.current_run(), None, "旧 run 在切台瞬间失效");
    assert_eq!(app.messages().len(), 1, "选中后历史回填进新视图");
    assert_eq!(app.messages()[0].content, "older history");

    // 第二条 prompt：新 session_id + 新 run_id（红线断言的两个字段）。
    submit(&mut app, "second");
    let second: PromptDelivery = app.take_prompt().expect("切台后 prompt 必须出站");
    assert_eq!(
        second.session_id, "s-2",
        "红线：切会话后 prompt 带旧 session_id 会把消息写进别的会话"
    );
    assert_ne!(
        second.run_id, old_run,
        "红线：切会话后 prompt 必须换新 run_id"
    );
    assert_ne!(second.text, first.text, "两次出站各自携带自己的正文");

    // 旧 run 的事件按 run_id 过滤：同一帧，旧 run_id 拒收、新 run_id 放行。
    let delta = AgentEvent::AssistantText {
        delta: "poison".to_string(),
    };
    app.apply_run_event(&old_run, &delta);
    assert!(
        app.messages().iter().all(|m| !m.content.contains("poison")),
        "旧 run 的事件不许渲染进新会话视图"
    );
    app.apply_run_event(&second.run_id, &delta);
    assert_eq!(
        app.messages().last().map(|m| m.content.as_str()),
        Some("poison"),
        "当前 run 的事件必须进视图"
    );
}

/// 回填历史的滚动边界：越界夹紧、空历史恒 0、切台复位——不许 usize
/// 下溢 panic（route §4 `history_scroll_bounds_clamp`）。
#[test]
fn history_scroll_bounds_clamp() {
    let mut app = App::new();
    app.start_session("s-1", vec![user_msg("m1"), user_msg("m2"), user_msg("m3")]);
    assert_eq!(app.scroll(), 0, "回填后钉底");

    app.scroll_by(10_000);
    assert_eq!(app.scroll(), 2, "上滚边界 = 历史条数 - 1，夹紧不越界");
    app.scroll_by(-10_000);
    assert_eq!(app.scroll(), 0, "下滚回到底，不许 usize 下溢 panic");
    app.scroll_by(1);
    app.scroll_by(-1);
    app.scroll_by(1);
    assert_eq!(app.scroll(), 1, "步进滚动落在边界内");

    // 切台回填：滚动复位，边界随新历史重算。
    app.switch_session("s-2", vec![user_msg("only-1"), user_msg("only-2")]);
    assert_eq!(app.scroll(), 0, "切会话回填后滚动复位");
    app.scroll_by(50);
    assert_eq!(app.scroll(), 1, "新历史的边界随回填重算");

    // 空历史：恒 0，不 panic。
    app.switch_session("s-3", Vec::new());
    app.scroll_by(50);
    assert_eq!(app.scroll(), 0, "空历史滚动恒 0");
}
