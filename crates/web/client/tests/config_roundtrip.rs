//! `LlmRuntimeConfig` 的 TOML 往返测试（ROADMAP 5.8 的遗留债：配置页
//! 读写代码早已接线，`crates/web/client/tests/` 却零覆盖）。
//!
//! 覆盖 `save_to_file` → `load_from_system` 的往返、`[llm]` 段对顶层键的
//! 覆盖、以及无配置文件时的兜底默认值。
//!
//! **为什么全部串行**：`load_from_system` 读的是相对路径
//! （`./.oi/config.toml` 等）并叠加进程级环境变量覆盖，都是进程全局状态。
//! cargo 默认多线程跑测试，若并行改 CWD / env 会互相打架，所以这里用一把
//! 全局锁把它们排成队，并在每个用例里显式清掉相关 env（开发机上可能真的
//! 设了 `OMENIC_LLM_*`，不清会让断言随环境飘）。

use omenic_web_client::llm::LlmRuntimeConfig;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// `load_from_system` 认的环境变量覆盖，测试前统一清空。
/// 必须与 `load_from_system` 里的 env var 名单保持同步：漏掉新增的覆盖
/// 会让本测试在该变量被污染时仍静默通过（覆盖丢失而不报错）。
const ENV_OVERRIDES: [&str; 4] = [
    "NEWAPI_RELAY_TOKEN",
    "OMENIC_LLM_API_KEY",
    "OMENIC_LLM_BASE_URL",
    "OMENIC_LLM_MODEL",
];

fn serial_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// 进入一个隔离的临时工作目录，并清掉 env 覆盖。
///
/// 返回的 guard 在 drop 时恢复原 CWD；`_lock` 保证同一时刻只有一个用例在
/// 动这些全局状态（前一个用例 panic 导致的锁中毒不该连累后续用例，故
/// 主动 `into_inner`）。
///
/// **CWD 刻意再下沉一层**（`<tempdir>/work`）：`load_from_system` 的候选
/// 路径里有 `../.oi/config.toml`，若直接站在 tempdir 根上，`..` 会落到
/// 共享的系统临时目录（`/tmp`），别人留下的 `.oi/config.toml` 就会被读进
/// 来，用例随机变红。下沉一层后 `./` 与 `../` 都还在本用例的 tempdir 内。
struct Sandbox {
    _lock: MutexGuard<'static, ()>,
    original_cwd: PathBuf,
    work: PathBuf,
    _dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let lock = serial_lock().lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("读取当前目录失败");
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).expect("创建工作子目录失败");
        std::env::set_current_dir(&work).expect("切换到临时目录失败");
        // SAFETY: 全局锁保证此刻没有其它测试线程在读写这些变量。
        unsafe {
            for key in ENV_OVERRIDES {
                std::env::remove_var(key);
            }
        }
        Sandbox {
            _lock: lock,
            original_cwd,
            work,
            _dir: dir,
        }
    }

    /// 当前工作目录（相对路径都以此为基准）。
    fn path(&self) -> &Path {
        &self.work
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original_cwd);
    }
}

/// 写盘 → 读回，五个字段逐一比对。
#[test]
fn save_then_load_preserves_every_field() {
    let sb = Sandbox::new();

    let saved = LlmRuntimeConfig {
        base_url: "http://127.0.0.1:9999/v1".to_string(),
        api_key: "sk-roundtrip-fixture".to_string(),
        model: "agnes-3.0-pro".to_string(),
        max_tokens: 8192,
        // 相对路径：save 写 ./.oi/config.toml，load 的第一个候选正是它
        data_dir: "./.oi".to_string(),
    };

    saved.save_to_file().expect("保存配置失败");
    assert!(
        sb.path().join(".oi/config.toml").is_file(),
        "save_to_file 应在 data_dir 下写出 config.toml"
    );

    let loaded = LlmRuntimeConfig::load_from_system();

    assert_eq!(loaded.base_url, saved.base_url);
    assert_eq!(loaded.api_key, saved.api_key);
    assert_eq!(loaded.model, saved.model);
    assert_eq!(loaded.max_tokens, saved.max_tokens);
    assert_eq!(loaded.data_dir, saved.data_dir);
    assert_eq!(loaded, saved, "整体往返必须等值");
}

/// 往返是幂等的：读回来的配置再存一次，文件内容逐字节相同。
#[test]
fn resaving_a_loaded_config_is_byte_identical() {
    let sb = Sandbox::new();

    let original = LlmRuntimeConfig {
        base_url: "https://relay.example.com".to_string(),
        api_key: "sk-idempotent".to_string(),
        model: "agnes-2.5-flash".to_string(),
        max_tokens: 4096,
        data_dir: "./.oi".to_string(),
    };
    original.save_to_file().expect("首次保存失败");

    let cfg_path = sb.path().join(".oi/config.toml");
    let first = std::fs::read_to_string(&cfg_path).expect("读取首次写入内容失败");

    let loaded = LlmRuntimeConfig::load_from_system();
    loaded.save_to_file().expect("二次保存失败");
    let second = std::fs::read_to_string(&cfg_path).expect("读取二次写入内容失败");

    assert_eq!(first, second, "load → save 不得改变文件内容");
}

/// `save_to_file` 会按需创建 data_dir（配置页首次保存时 `.oi` 往往不存在）。
#[test]
fn save_creates_a_missing_data_dir() {
    let sb = Sandbox::new();

    let nested = sb.path().join("deep/nested/.oi");
    assert!(!nested.exists(), "前置条件：目标目录尚不存在");

    let cfg = LlmRuntimeConfig {
        base_url: "http://localhost:3182".to_string(),
        api_key: "sk-mkdir".to_string(),
        model: "m".to_string(),
        max_tokens: 128,
        data_dir: nested.to_string_lossy().to_string(),
    };
    cfg.save_to_file().expect("保存到不存在的目录应自动建目录");

    assert!(nested.join("config.toml").is_file());
}

/// `[llm]` 段里的 model 覆盖顶层 model；max_tokens 从整数正确取回。
#[test]
fn llm_section_overrides_top_level_model() {
    let _sb = Sandbox::new();

    std::fs::create_dir_all(".oi").expect("建 .oi 失败");
    std::fs::write(
        ".oi/config.toml",
        "data_dir = \"./.oi\"\n\
         model = \"top-level-model\"\n\
         \n\
         [llm]\n\
         base_url = \"http://example.invalid\"\n\
         api_key = \"sk-section\"\n\
         model = \"llm-section-model\"\n\
         max_tokens = 2048\n",
    )
    .expect("写配置失败");

    let loaded = LlmRuntimeConfig::load_from_system();

    assert_eq!(
        loaded.model, "llm-section-model",
        "[llm].model 应覆盖顶层 model"
    );
    assert_eq!(loaded.base_url, "http://example.invalid");
    assert_eq!(loaded.api_key, "sk-section");
    assert_eq!(loaded.max_tokens, 2048);
    assert_eq!(loaded.data_dir, "./.oi");
}

/// 配置文件缺字段时，缺的那部分保持内置默认，不被清空。
#[test]
fn missing_keys_fall_back_to_builtin_defaults() {
    let _sb = Sandbox::new();

    std::fs::create_dir_all(".oi").expect("建 .oi 失败");
    // 只给 base_url，其余全缺
    std::fs::write(".oi/config.toml", "[llm]\nbase_url = \"http://only-url\"\n")
        .expect("写配置失败");

    let loaded = LlmRuntimeConfig::load_from_system();

    assert_eq!(loaded.base_url, "http://only-url");
    // 既有设计：未配置时 api_key 是占位串（不是空串），配置页据此提示用户去填
    assert_eq!(loaded.api_key, "sk-config-not-set");
    assert_eq!(loaded.model, "agnes-2.5-flash");
    assert_eq!(loaded.max_tokens, 4096);
    assert_eq!(loaded.data_dir, "./.oi");
}

/// 完全没有配置文件时 `load_from_system` 不 panic，返回全套内置默认值。
#[test]
fn load_without_any_config_file_yields_defaults() {
    let _sb = Sandbox::new();

    let loaded = LlmRuntimeConfig::load_from_system();

    assert_eq!(loaded.base_url, "http://127.0.0.1:3182");
    assert_eq!(loaded.api_key, "sk-config-not-set");
    assert_eq!(loaded.model, "agnes-2.5-flash");
    assert_eq!(loaded.max_tokens, 4096);
    assert_eq!(loaded.data_dir, "./.oi");
    // `Default` 就是 `load_from_system`，两者必须一致
    assert_eq!(LlmRuntimeConfig::default(), loaded);
}

/// 环境变量优先于文件；且 `NEWAPI_RELAY_TOKEN` 优先于 `OMENIC_LLM_API_KEY`
/// （`load_from_system` 里是 if / else if，前者赢）。
#[test]
fn env_overrides_take_precedence_over_the_file() {
    let _sb = Sandbox::new();

    LlmRuntimeConfig {
        base_url: "http://from-file".to_string(),
        api_key: "sk-from-file".to_string(),
        model: "model-from-file".to_string(),
        max_tokens: 512,
        data_dir: "./.oi".to_string(),
    }
    .save_to_file()
    .expect("保存失败");

    // SAFETY: Sandbox 持有全局锁，此刻无其它测试线程读写这些变量。
    unsafe {
        std::env::set_var("NEWAPI_RELAY_TOKEN", "sk-from-relay-env");
        std::env::set_var("OMENIC_LLM_API_KEY", "sk-should-lose");
        std::env::set_var("OMENIC_LLM_BASE_URL", "http://from-env");
        std::env::set_var("OMENIC_LLM_MODEL", "model-from-env");
    }

    let loaded = LlmRuntimeConfig::load_from_system();

    assert_eq!(loaded.base_url, "http://from-env");
    assert_eq!(loaded.model, "model-from-env");
    assert_eq!(
        loaded.api_key, "sk-from-relay-env",
        "NEWAPI_RELAY_TOKEN 应赢过 OMENIC_LLM_API_KEY"
    );
    // max_tokens 没有 env 覆盖通道，仍来自文件
    assert_eq!(loaded.max_tokens, 512);

    // SAFETY: 同上；离开前清掉，避免污染同进程后续用例。
    unsafe {
        for key in ENV_OVERRIDES {
            std::env::remove_var(key);
        }
    }
}
