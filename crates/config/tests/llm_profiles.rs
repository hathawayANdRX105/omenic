//! 凭据档案：切档案 = 改一行 active_profile，而不是重写整份配置。
//!
//! Red when: 档案没被解析（daemon 仍读扁平字段），或 `api_key_env` 指向的
//! 环境变量缺失时静默用别的 key（模型会拿着错的凭据去请求）。

use std::sync::{Mutex, MutexGuard, OnceLock};

use config::Config;

/// `Config::load` reads `.oi/config.toml` relative to the *process* cwd, so
/// these tests must not run concurrently — one test's temp dir would be
/// another test's config. Serialized here rather than by `--test-threads=1`
/// so the rest of the suite keeps its parallelism.
fn cwd_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// `[llm]` 顶层字段全空、只有档案时，解析必须认出档案。
#[test]
fn active_profile_supplies_the_credential() {
    let toml = r#"
[llm]
active_profile = "work"

[[llm.profiles]]
name = "work"
base_url = "https://api.example.com"
model = "model-a"
api_key = "key-a"

[[llm.profiles]]
name = "local"
base_url = "http://127.0.0.1:1234"
model = "model-b"
api_key = "key-b"
"#;
    let cfg = parse(toml);
    assert_eq!(cfg.llm_profiles.len(), 2, "both profiles parsed");
    let active = cfg.active_llm().expect("active profile resolves");
    assert_eq!(active.model, "model-a");
    assert_eq!(active.api_key, "key-a");
    assert_eq!(active.base_url, "https://api.example.com");
}

/// 切档案只改 `active_profile`：模型与 key 一起换。
#[test]
fn switching_active_profile_switches_model_and_key() {
    let toml = r#"
[llm]
active_profile = "local"

[[llm.profiles]]
name = "work"
base_url = "https://api.example.com"
model = "model-a"
api_key = "key-a"

[[llm.profiles]]
name = "local"
base_url = "http://127.0.0.1:1234"
model = "model-b"
api_key = "key-b"
"#;
    let active = parse(toml).active_llm().expect("resolves");
    assert_eq!(active.model, "model-b");
    assert_eq!(active.api_key, "key-b");
}

/// `api_key_env` 优先：共享配置文件里不带明文 key。
#[test]
fn api_key_env_wins_over_inline_key() {
    // SAFETY: single-threaded test process for this variable.
    unsafe { std::env::set_var("OI_TEST_PROFILE_KEY", "from-env") };
    let toml = r#"
[llm]
active_profile = "envkey"

[[llm.profiles]]
name = "envkey"
base_url = "https://api.example.com"
model = "model-a"
api_key = "inline-key"
api_key_env = "OI_TEST_PROFILE_KEY"
"#;
    let active = parse(toml).active_llm().expect("resolves");
    assert_eq!(active.api_key, "from-env");
    unsafe { std::env::remove_var("OI_TEST_PROFILE_KEY") };
}

/// env 变量没设时退回 inline key，而不是变成"没凭据"或用别的档案。
#[test]
fn missing_env_falls_back_to_inline_key() {
    let toml = r#"
[llm]
active_profile = "envkey"

[[llm.profiles]]
name = "envkey"
base_url = "https://api.example.com"
model = "model-a"
api_key = "inline-key"
api_key_env = "OI_TEST_ABSENT_KEY"
"#;
    let active = parse(toml).active_llm().expect("resolves");
    assert_eq!(active.api_key, "inline-key");
}

/// 没设 active_profile 时行为不变：读扁平 `[llm]` 字段。
#[test]
fn no_active_profile_keeps_flat_fields() {
    let toml = r#"
[llm]
base_url = "https://flat.example.com"
model = "flat-model"
api_key = "flat-key"
max_tokens = 256

[[llm.profiles]]
name = "unused"
base_url = "https://unused.example.com"
model = "unused-model"
api_key = "unused-key"
"#;
    let active = parse(toml).active_llm().expect("resolves");
    assert_eq!(active.model, "flat-model");
    assert_eq!(active.max_tokens, Some(256));
}

/// active_profile 指向不存在的档案 = **加载失败**。悄悄回退到扁平字段会
/// 让这次运行用上另一个 provider——凭据打错字最不能出的就是这种错。
#[test]
fn unknown_active_profile_fails_validation() {
    let toml = r#"
[llm]
active_profile = "typo"
base_url = "https://flat.example.com"
model = "flat-model"
api_key = "flat-key"

[[llm.profiles]]
name = "work"
base_url = "https://api.example.com"
model = "model-a"
api_key = "key-a"
"#;
    let err = parse_err(toml);
    assert!(
        err.contains("no such profile"),
        "the typo must name itself: {err}"
    );
}

/// 校验通过后，`active_llm` 不会再因为"档案不存在"而返回 None——那是
/// validate 的职责（防御性第二道）。
#[test]
fn unknown_active_profile_is_unreachable_after_validation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let oi = dir.path().join(".oi");
    std::fs::create_dir_all(&oi).expect("create .oi");
    std::fs::write(
        oi.join("config.toml"),
        r#"
[llm]
active_profile = "typo"

[[llm.profiles]]
name = "work"
base_url = "https://api.example.com"
model = "model-a"
api_key = "key-a"
"#,
    )
    .expect("write config");
    let _guard = cwd_lock();
    let original = std::env::current_dir().expect("current_dir");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    let result = std::panic::catch_unwind(config::Config::load);
    let _ = std::env::set_current_dir(&original);
    assert!(result.is_ok(), "load must not panic");
    let cfg = result.expect("no panic").expect_err("typo must fail load");
    assert!(
        cfg.to_string().contains("no such profile"),
        "error should name the missing profile, got: {cfg}"
    );
}

fn parse(toml: &str) -> Config {
    // Same shape as the sibling `llm_fallbacks_parse` test: `Config::load`
    // reads `.oi/config.toml` relative to the process cwd, so the test
    // switches cwd into its own temp dir for the duration of the load.
    let dir = tempfile::tempdir().expect("tempdir");
    let oi = dir.path().join(".oi");
    std::fs::create_dir_all(&oi).expect("create .oi");
    std::fs::write(oi.join("config.toml"), toml).expect("write config");

    let _guard = cwd_lock();
    let original = std::env::current_dir().expect("current_dir");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    let result = std::panic::catch_unwind(config::Config::load);
    let _ = std::env::set_current_dir(&original);

    result
        .expect("Config::load panicked")
        .expect("Config::load failed")
}

fn parse_err(toml: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let oi = dir.path().join(".oi");
    std::fs::create_dir_all(&oi).expect("create .oi");
    std::fs::write(oi.join("config.toml"), toml).expect("write config");
    let _guard = cwd_lock();
    let original = std::env::current_dir().expect("current_dir");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    let result = std::panic::catch_unwind(config::Config::load);
    let _ = std::env::set_current_dir(&original);
    result
        .expect("Config::load panicked")
        .expect_err("expected a load error")
        .to_string()
}
