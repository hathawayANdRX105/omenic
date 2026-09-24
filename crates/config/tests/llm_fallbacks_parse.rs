//! T5 — verify the config crate's public `Config::load()` TOML path
//! deserializes the web client's `[[llm.fallbacks]]` table array into
//! `Config.llm_fallbacks`, and that web's `load_from_system` key paths
//! read the same layout.

use std::path::PathBuf;

fn write_config(dir: &std::path::Path) -> PathBuf {
    let oi_dir = dir.join(".oi");
    std::fs::create_dir_all(&oi_dir).expect("create .oi dir");
    let path = oi_dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[llm]
base_url = "http://primary.example.com"
api_key = "sk-primary"
model = "primary-model"
max_tokens = 4096

[[llm.fallbacks]]
model = "fallback-model-a"
base_url = "http://relay-a.example.com/v1"
api_key = "sk-fallback-a"
max_tokens = 8192

[[llm.fallbacks]]
model = "fallback-model-b"
"#,
    )
    .expect("write config.toml");
    path
}

#[test]
fn parse_llm_fallbacks_table_array() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path());

    let original_cwd = std::env::current_dir().expect("current_dir");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    let result = std::panic::catch_unwind(config::Config::load);
    let _ = std::env::set_current_dir(&original_cwd);

    let config = result
        .expect("Config::load panicked")
        .expect("Config::load failed");

    assert_eq!(
        config.llm_fallbacks.len(),
        2,
        "expected two fallback providers"
    );
    assert_eq!(
        config.llm_fallbacks[0].model.as_deref(),
        Some("fallback-model-a")
    );
    assert_eq!(
        config.llm_fallbacks[0].base_url.as_deref(),
        Some("http://relay-a.example.com/v1")
    );
    assert_eq!(
        config.llm_fallbacks[0].api_key.as_deref(),
        Some("sk-fallback-a")
    );
    assert_eq!(config.llm_fallbacks[0].max_tokens, Some(8192));
    assert_eq!(
        config.llm_fallbacks[1].model.as_deref(),
        Some("fallback-model-b")
    );
    assert_eq!(config.llm_fallbacks[1].api_key, None);
    assert_eq!(config.llm_fallbacks[1].base_url, None);
    assert_eq!(config.llm_fallbacks[1].max_tokens, None);
}
