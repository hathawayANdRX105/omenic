//! daemon 用档案里的凭据建 orbit 模型；没有 active 档案时行为与从前一致。
//!
//! Red when: 档案被解析了但 daemon 仍读扁平字段（切档案不起作用），或档案
//! 缺 key 时静默用上别的 provider。

use daemon::{Daemon, DaemonConfig};

fn config_from(toml: &str) -> (tempfile::TempDir, DaemonConfig) {
    let dir = tempfile::tempdir().expect("temp dir");
    let oi = dir.path().join(".oi");
    std::fs::create_dir_all(&oi).expect("create .oi");
    std::fs::write(oi.join("config.toml"), toml).expect("write config");
    let original = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    let result = std::panic::catch_unwind(config::Config::load);
    let _ = std::env::set_current_dir(&original);
    let cfg = result
        .expect("Config::load panicked")
        .expect("Config::load failed");
    (dir, DaemonConfig::from_config(&cfg).expect("daemon config"))
}

#[test]
fn active_profile_reaches_the_orbit_model() {
    let (_dir, cfg) = config_from(
        r#"
[llm]
active_profile = "work"

[[llm.profiles]]
name = "work"
base_url = "https://api.example.com"
model = "model-a"
api_key = "key-a"
max_tokens = 4096
"#,
    );
    let model = cfg.orbit_model.expect("orbit model from profile");
    assert_eq!(model.model, "model-a");
    assert_eq!(model.api_key, "key-a");
    assert_eq!(
        model.base_url.as_deref(),
        Some("https://api.example.com/v1"),
        "the `/v1` suffix is still appended at the call site"
    );
    assert_eq!(model.max_tokens, Some(4096));
}

#[test]
fn no_profile_keeps_the_flat_fields_path() {
    let (_dir, cfg) = config_from(
        r#"
[llm]
base_url = "https://flat.example.com"
model = "flat-model"
api_key = "flat-key"
"#,
    );
    let model = cfg.orbit_model.expect("orbit model from flat fields");
    assert_eq!(model.model, "flat-model");
    assert_eq!(model.api_key, "flat-key");
}

#[test]
fn profile_without_a_resolvable_key_yields_no_orbit_model() {
    // A profile whose `api_key_env` names an unset variable and which has no
    // inline key must not silently fall back to another provider's key.
    let (_dir, cfg) = config_from(
        r#"
[llm]
active_profile = "envonly"
base_url = "https://flat.example.com"
model = "flat-model"
api_key = "flat-key"

[[llm.profiles]]
name = "envonly"
base_url = "https://api.example.com"
model = "model-a"
api_key_env = "OI_ABSENT_PROFILE_KEY"
"#,
    );
    assert!(
        cfg.orbit_model.is_none(),
        "no key anywhere for the active profile = no orbit model"
    );
}

#[test]
fn daemon_starts_with_a_profile_only_config() {
    let (_dir, cfg) = config_from(
        r#"
[llm]
active_profile = "local"

[[llm.profiles]]
name = "local"
base_url = "http://127.0.0.1:9"
model = "local-model"
api_key = "local-key"
"#,
    );
    let mut cfg = cfg;
    cfg.socket_path = Some(std::path::PathBuf::from(
        cfg.data_dir.join("profile-daemon.sock"),
    ));
    let _daemon = Daemon::start(cfg).expect("daemon starts on a profile-only config");
}
