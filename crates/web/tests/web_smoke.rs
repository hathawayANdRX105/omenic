use web::mock::*;

#[test]
fn test_mock_sessions_serialize() {
    let sessions = mock_sessions();
    assert!(!sessions.is_empty());
    let json = serde_json::to_string(&sessions).unwrap();
    let de: Vec<Session> = serde_json::from_str(&json).unwrap();
    assert_eq!(sessions.len(), de.len());
    assert_eq!(sessions[0].id, de[0].id);
}

#[test]
fn test_mock_messages_serialize() {
    let messages = mock_messages();
    assert!(!messages.is_empty());
    let json = serde_json::to_string(&messages).unwrap();
    let de: Vec<ChatMessage> = serde_json::from_str(&json).unwrap();
    assert_eq!(messages.len(), de.len());
}

#[test]
fn test_mock_statusline_serialize() {
    let statusline = mock_statusline();
    assert_eq!(statusline.model, "qwen3-32b");
    let json = serde_json::to_string(&statusline).unwrap();
    let de: StatusLine = serde_json::from_str(&json).unwrap();
    assert_eq!(de.git_branch, "feat/web-dioxus");
}

#[test]
fn test_mock_tasks_serialize() {
    let tasks = mock_tasks();
    assert!(!tasks.is_empty());
    let json = serde_json::to_string(&tasks).unwrap();
    let de: Vec<TaskItem> = serde_json::from_str(&json).unwrap();
    assert_eq!(tasks.len(), de.len());
}

#[test]
fn test_mock_stats_serialize() {
    let stats = mock_stats();
    assert_eq!(stats.kpis.len(), 5);
    assert_eq!(stats.sub_metrics.len(), 7);
    assert_eq!(stats.agent_bars.len(), 2);
    assert!(!stats.throughput.is_empty());
    assert!(!stats.feed.is_empty());

    let json = serde_json::to_string(&stats).unwrap();
    let de: StatsData = serde_json::from_str(&json).unwrap();
    assert_eq!(de.kpis.len(), 5);
}

#[test]
fn test_mock_config_serialize() {
    let cfg = mock_config();
    assert_eq!(cfg.providers.len(), 2);
    assert_eq!(cfg.default_model, "qwen3-32b");
    let json = serde_json::to_string(&cfg).unwrap();
    let de: ConfigData = serde_json::from_str(&json).unwrap();
    assert_eq!(de.default_model, "qwen3-32b");
}
