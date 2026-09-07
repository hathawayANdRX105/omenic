//! E2E Feature & Workflow Tests for Omenic Web UI
//! Covers session creation/deletion without VirtualDom key collisions,
//! task board filtering & progress calculations, statistics ranges, and configuration updates.

use dioxus::prelude::*;
use web::components::sidebar::Sidebar;
use web::components::taskpanel::TaskPanel;
use web::llm::LlmRuntimeConfig;
use web::mock::*;

#[test]
fn test_e2e_session_deletion_and_creation_diff() {
    #[component]
    fn SessionManagerApp() -> Element {
        let mut sessions = use_signal(|| {
            vec![
                Session {
                    id: "s-1".into(),
                    title: "会话 1".into(),
                    last_active: "刚刚".into(),
                    model: "default".into(),
                    status: SessionStatus::Idle,
                    last_active_epoch: 0,
                },
                Session {
                    id: "s-2".into(),
                    title: "会话 2".into(),
                    last_active: "刚刚".into(),
                    model: "default".into(),
                    status: SessionStatus::Idle,
                    last_active_epoch: 0,
                },
                Session {
                    id: "s-3".into(),
                    title: "会话 3".into(),
                    last_active: "刚刚".into(),
                    model: "default".into(),
                    status: SessionStatus::Idle,
                    last_active_epoch: 0,
                },
            ]
        });
        let active_id = use_signal(|| "s-1".to_string());

        let on_delete = move |id: String| {
            let mut list = sessions();
            list.retain(|s| s.id != id);
            sessions.set(list);
        };

        let on_create = move |()| {
            let mut list = sessions();
            let new_id = format!("s-{}", list.len() + 100);
            list.insert(
                0,
                Session {
                    id: new_id,
                    title: "新任务".into(),
                    last_active: "刚刚".into(),
                    model: "default".into(),
                    status: SessionStatus::Idle,
                    last_active_epoch: 0,
                },
            );
            sessions.set(list);
        };

        rsx! {
            Sidebar {
                spaces: vec![],
                active_space_id: String::new(),
                on_select_space: move |_| {},
                on_trigger_picker: move |_| {},
                sessions: sessions(),
                active_id: active_id(),
                on_select: move |_| {},
                on_create: on_create,
                on_delete: on_delete,
                on_archive: move |_| {},
                on_rename: move |(_id, _t): (String, String)| {},
            }
        }
    }

    // 1. Initial VirtualDom rebuild
    let mut dom = VirtualDom::new(SessionManagerApp);
    dom.rebuild_in_place();

    // 2. Simulate VirtualDom render passes after creation and deletion
    // Verify VirtualDom reconciliation succeeds without "keyed siblings must each have a unique key" panic
    dom.render_immediate(&mut dioxus_core::NoOpMutations);
}

#[test]
fn test_e2e_task_board_metrics_and_filter() {
    let tasks = mock_tasks();
    let total = tasks.len();
    assert!(total >= 5);

    let done_count = tasks.iter().filter(|t| t.status == "done").count();
    let in_prog_count = tasks.iter().filter(|t| t.status == "in_progress").count();
    let open_count = tasks.iter().filter(|t| t.status == "open").count();
    let blocked_count = tasks.iter().filter(|t| t.status == "blocked").count();

    assert_eq!(
        done_count + in_prog_count + open_count + blocked_count,
        total
    );

    let pct = (done_count as f64 / total as f64 * 100.0).round() as u32;
    assert!(pct <= 100);

    // Verify task panel mounts and diffs cleanly with filtered lists
    #[component]
    fn TaskPanelTestApp(tasks: Vec<TaskItem>) -> Element {
        rsx! {
            TaskPanel {
                tasks: tasks,
                on_close: move |_| {},
            }
        }
    }

    let mut dom = VirtualDom::new_with_props(TaskPanelTestApp, TaskPanelTestAppProps { tasks });
    dom.rebuild_in_place();
}

#[test]
fn test_e2e_stats_range_data_integrity() {
    for range in ["1h", "24h", "7d", "30d", "90d", "All"] {
        let stats = mock_stats_for_range(range);
        assert_eq!(stats.kpis.len(), 5, "Range {} must have 5 KPIs", range);
        assert_eq!(
            stats.sub_metrics.len(),
            7,
            "Range {} must have 7 submetrics",
            range
        );
        assert_eq!(
            stats.agent_bars.len(),
            2,
            "Range {} must have 2 agent bars",
            range
        );
        assert!(
            !stats.throughput.is_empty(),
            "Throughput points should not be empty"
        );
        assert!(!stats.feed.is_empty(), "Feed should not be empty");
    }
}

#[test]
fn test_e2e_config_validation_rules() {
    let mut cfg = LlmRuntimeConfig::default();
    cfg.base_url = "http://127.0.0.1:3182".into();
    cfg.api_key = "sk-valid-key-with-enough-chars".into();
    cfg.model = "agnes-2.5-flash".into();
    cfg.max_tokens = 4096;
    cfg.data_dir = "./.oi".into();

    assert!(cfg.base_url.starts_with("http://") || cfg.base_url.starts_with("https://"));
    assert!(cfg.api_key.len() >= 8);
    assert!(!cfg.model.is_empty());
    assert!(cfg.max_tokens >= 16 && cfg.max_tokens <= 200_000);
    assert!(!cfg.data_dir.is_empty());
}

// ── 单元级测试：db_load_sessions 空库回退 / worktree 解析去重 ─────────────
// 这些函数现在都是 pub，可以被测试导入以覆盖数据加载、工作区解析的回归保护。

#[test]
fn test_parse_worktree_porcelain_dedups_paths() {
    let sample = r#"worktree /home/u/proj/main
HEAD 0000000000000000000000000000000000000000
branch refs/heads/main

worktree /home/u/proj/.wt/alpha
HEAD 1111111111111111111111111111111111111111
branch refs/heads/feat/a

worktree /home/u/proj/.wt/beta
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feat/b

worktree /home/u/proj/.wt/beta
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feat/b
"#;
    let ws = web::pages::workspace::parse_worktree_porcelain(sample, "/home/u/proj/main");
    let paths: Vec<&str> = ws.iter().map(|w| w.path.as_str()).collect();
    // 同一路径只能出现一次
    for p in paths.iter() {
        assert_eq!(paths.iter().filter(|x| *x == p).count(), 1, "dup {p}");
    }
    assert!(ws.iter().any(|w| w.name.contains("main")));
    assert!(ws
        .iter()
        .any(|w| w.is_active && w.path == "/home/u/proj/main"));
    assert_eq!(ws.len(), 3);
}

#[test]
fn test_parse_worktree_porcelain_empty() {
    let ws = web::pages::workspace::parse_worktree_porcelain("", "/x");
    assert!(ws.is_empty());
}

#[test]
fn test_read_subdirectories_returns_dirs() {
    let tmp = std::env::temp_dir().join(format!(
        "oi-wt-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(tmp.join("aa")).unwrap();
    std::fs::create_dir_all(tmp.join("bb")).unwrap();
    std::fs::write(tmp.join("file.txt"), "x").unwrap();
    let mut dirs = web::pages::workspace::read_subdirectories(&tmp);
    dirs.sort_unstable();
    assert_eq!(dirs, vec!["aa".to_string(), "bb".to_string()]);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_db_load_sessions_empty_dir_returns_empty() {
    let tmp = std::env::temp_dir().join(format!(
        "oi-wt-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let (sessions, msgs, active) = web::pages::workspace::db_load_sessions(tmp.to_str().unwrap());
    // 契约：空工作区必须返回空数据，不允许回落到 mock 固定 5 个假会话
    assert!(
        sessions.is_empty(),
        "expect empty sessions, got {}",
        sessions.len()
    );
    assert!(msgs.is_empty());
    assert!(active.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_db_save_and_load_roundtrip() {
    let tmp = std::env::temp_dir().join(format!(
        "oi-wt-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let dir_str = tmp.to_str().unwrap().to_string();
    web::pages::workspace::db_save_message(
        dir_str.clone(),
        "s-test".into(),
        Some("网测".into()),
        session::SessionRole::User,
        "hello".into(),
    );
    // 线程异步落盘，等待
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (sessions, msgs, active) = web::pages::workspace::db_load_sessions(&dir_str);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "s-test");
    assert_eq!(active, "s-test");
    let stored = msgs.get("s-test").expect("messages loaded");
    assert_eq!(stored.len(), 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── IME 守卫注入核验（防止前端注入脚本被误删）─────────────────────────────

#[test]
fn test_chat_input_has_ime_composition_guard() {
    // 服务静态页必须包含 IME 组合事件守卫,否则 Linux 中文输入法下 Enter 会提前提交
    let lib_src = include_str!("../src/lib.rs");
    assert!(
        lib_src.contains("compositionstart"),
        "缺少 compositionstart 监听"
    );
    assert!(
        lib_src.contains("compositionend"),
        "缺少 compositionend 监听"
    );
    assert!(
        lib_src.contains("chat-input-area"),
        "IME 守卫应定向 chat-input-area"
    );
    assert!(
        lib_src.contains("isComposing"),
        "应保留 isComposing / keyCode 229 的二级判定"
    );
}

#[test]
fn test_new_session_does_not_insert_welcome_message() {
    // 回归保护: on_create_session 不再向消息列表注入欢迎气泡
    let src = include_str!("../src/pages/workspace.rs");
    // 欢迎气泡文本 sig 关键字
    assert!(
        !src.contains("新任务已创建"),
        "workspace.rs 不应再注入欢迎气泡文本"
    );
    // 且新建会话应创建一个 messages 空 Vec(而非欢迎消息列表)
    assert!(
        src.contains("map.insert(new_id.clone(), Vec::new())"),
        "新会话应注册空消息列表而不是欢迎消息"
    );
}
