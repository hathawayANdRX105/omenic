//! 一个档案配错了，选它之前看不见；选中之后才发现。
//!
//! Red when: 档案缺 key / 字段为空仍被报成可用（启动日志不说，运行时才炸），
//! 或 `api_key_env` 已设却被误判成缺 key。

use std::sync::{LazyLock, Mutex, MutexGuard};

use config::{Config, LlmProfileConfig, ProfileStatus};

/// `Config::load` 读进程 cwd 下的 `.oi/config.toml`，所以这些测试不能并行。
fn cwd_lock() -> MutexGuard<'static, ()> {
    static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn profile(name: &str, base_url: &str, model: &str) -> LlmProfileConfig {
    LlmProfileConfig {
        name: name.into(),
        base_url: base_url.into(),
        model: model.into(),
        api_key: Some("key".into()),
        api_key_env: None,
        max_tokens: None,
    }
}

#[test]
fn a_fully_specified_profile_is_ready() {
    let p = profile("work", "https://api.example.com", "m");
    assert_eq!(p.status(), ProfileStatus::Ready);
    assert!(p.status().is_ready());
}

/// 缺 key：档案不能发请求，选中它只会得到"无 orbit 模型"。
#[test]
fn a_profile_without_any_key_is_missing_key() {
    let mut p = profile("nokey", "https://api.example.com", "m");
    p.api_key = None;
    assert_eq!(p.status(), ProfileStatus::MissingKey);
    assert!(!p.status().is_ready());
}

/// 空白字段按缺失算（` validate` 只 trim 来测试值，空白能过那里）。
#[test]
fn blank_base_url_or_model_is_incomplete() {
    let mut blank_url = profile("a", "   ", "m");
    assert_eq!(blank_url.status(), ProfileStatus::Incomplete);
    blank_url.base_url = "https://api.example.com".into();
    blank_url.model = "  ".into();
    assert_eq!(blank_url.status(), ProfileStatus::Incomplete);
}

/// `api_key_env` 指向的变量没设、又没有 inline key —— 这是共享配置最常见的
/// 状态（key 在用户 shell 里，daemon 环境下没继承），必须单独报出来。
#[test]
fn unset_api_key_env_with_no_inline_key_is_missing_key() {
    let mut p = profile("env", "https://api.example.com", "m");
    p.api_key = None;
    p.api_key_env = Some("OI_TEST_ABSENT_PROFILE_KEY".into());
    assert_eq!(p.status(), ProfileStatus::MissingKey);
}

/// 变量设了就是 ready，不该因为"没有 inline key"被误报。
#[test]
fn a_set_api_key_env_counts_as_ready() {
    // SAFETY: single-threaded test process for this variable.
    unsafe { std::env::set_var("OI_TEST_PROFILE_KEY", "from-env") };
    let mut p = profile("env", "https://api.example.com", "m");
    p.api_key = None;
    p.api_key_env = Some("OI_TEST_PROFILE_KEY".into());
    assert_eq!(p.status(), ProfileStatus::Ready);
    unsafe { std::env::remove_var("OI_TEST_PROFILE_KEY") };
}

/// 多档案：每个各自报自己的状态，一个坏的不影响好的那个被选中。
#[test]
fn every_profile_reports_its_own_status_in_config_order() {
    let toml = r#"
[llm]
active_profile = "work"

[[llm.profiles]]
name = "work"
base_url = "https://api.example.com"
model = "m"
api_key = "k"

[[llm.profiles]]
name = "broken"
base_url = "https://broken.example.com"
model = "m"

[[llm.profiles]]
name = "blank"
base_url = ""
model = "m"
api_key = "k"
"#;
    let cfg = parse(toml);
    assert_eq!(
        cfg.profile_statuses(),
        vec![
            ("work", ProfileStatus::Ready),
            ("broken", ProfileStatus::MissingKey),
            ("blank", ProfileStatus::Incomplete),
        ]
    );
    // 坏的档案存在不该影响 active 的那个解析。
    let active = cfg.active_llm().expect("active resolves");
    assert_eq!(active.model, "m");
}

fn parse(toml: &str) -> Config {
    let dir = tempfile::tempdir().expect("tempdir");
    let oi = dir.path().join(".oi");
    std::fs::create_dir_all(&oi).expect("create .oi");
    std::fs::write(oi.join("config.toml"), toml).expect("write config");
    let _guard = cwd_lock();
    let original = std::env::current_dir().expect("current_dir");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    let result = std::panic::catch_unwind(Config::load);
    let _ = std::env::set_current_dir(&original);
    result
        .expect("Config::load panicked")
        .expect("Config::load failed")
}
