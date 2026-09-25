//! `LlmRuntimeConfig` 的 TOML 往返测试（ROADMAP 5.8 的遗留债：配置页
//! 读写代码早已接线，`crates/web/client/tests/` 却零覆盖）。
//!
//! 覆盖 `save_to_file` → `load_from_system` 的往返、`[llm]` 段对顶层键的
//! 覆盖、`[mcp]` 段按 name 的增量编辑（env/reconnect/未知键/注释保留）、
//! 以及无配置文件时的兜底默认值。
//!
//! **为什么全部串行**：`load_from_system` 读的是相对路径
//! （`./.oi/config.toml` 等）并叠加进程级环境变量覆盖，都是进程全局状态。
//! cargo 默认多线程跑测试，若并行改 CWD / env 会互相打架，所以这里用一把
//! 全局锁把它们排成队，并在每个用例里显式清掉相关 env（开发机上可能真的
//! 设了 `OMENIC_LLM_*`，不清会让断言随环境飘）。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use web_client::llm::{LlmFallbackForm, LlmRuntimeConfig, McpServerForm};

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
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
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
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
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
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    };
    cfg.save_to_file().expect("保存到不存在的目录应自动建目录");

    assert!(nested.join("config.toml").is_file());
}

/// `[llm]` 段里的 model 覆盖顶层 model；max_tokens 从整数正确取回。
#[test]
fn llm_section_overrides_top_level_model() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    std::fs::write(
        sb.path().join(".oi/config.toml"),
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
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    // 只给 base_url，其余全缺
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "[llm]\nbase_url = \"http://only-url\"\n",
    )
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
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
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

/// 核心回归：`save_to_file` 只更新自己管理的键，`[mcp]` / `[memory]` / `[daemon]`
/// 等未管理段必须原样保留（旧实现用 `format!()` 整文件重写，会静默抹掉它们）。
#[test]
fn save_preserves_unmanaged_sections() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    // 一份「完整」配置：除管理键外还含三个未管理段，均取自
    // `crates/infra/config` 的真实 schema（`TomlConfig` 的 mcp/memory/daemon）。
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "# omenic configuration\n\
         omp_path = \"omp\"\n\
         data_dir = \"./.oi\"\n\
         model = \"agnes-2.5-flash\"\n\
         \n\
         [llm]\n\
         base_url = \"http://127.0.0.1:3182\"\n\
         api_key = \"sk-original\"\n\
         model = \"agnes-2.5-flash\"\n\
         max_tokens = 4096\n\
         \n\
         [memory]\n\
         enabled = true\n\
         dir = \"./.oi/memory\"\n\
         \n\
         [daemon]\n\
         cwd = \"/workspace\"\n\
         max_turns = 32\n\
         \n\
         [mcp]\n\
         \n\
         [[mcp.servers]]\n\
         name = \"fs\"\n\
         command = \"npx\"\n\
         args = [\"-y\", \"@modelcontextprotocol/server-filesystem\"]\n",
    )
    .expect("写配置失败");

    LlmRuntimeConfig {
        base_url: "http://127.0.0.1:9999/v1".to_string(),
        api_key: "sk-updated".to_string(),
        model: "agnes-3.0-pro".to_string(),
        max_tokens: 8192,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
    .save_to_file()
    .expect("保存配置失败");

    // 用 toml_edit 解析后逐段断言，而不是只看字符串包含——这样「段在但键被抹」
    // 也能被抓到。
    let doc = std::fs::read_to_string(sb.path().join(".oi/config.toml"))
        .expect("读回配置失败")
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后的配置必须是合法 TOML");

    // 管理键确实被更新
    assert_eq!(doc["data_dir"].as_str(), Some("./.oi"));
    assert_eq!(doc["model"].as_str(), Some("agnes-3.0-pro"));
    assert_eq!(
        doc["llm"]["base_url"].as_str(),
        Some("http://127.0.0.1:9999/v1")
    );
    assert_eq!(doc["llm"]["api_key"].as_str(), Some("sk-updated"));
    assert_eq!(doc["llm"]["model"].as_str(), Some("agnes-3.0-pro"));
    assert_eq!(doc["llm"]["max_tokens"].as_integer(), Some(8192));

    // 未管理段逐键保留
    assert_eq!(doc["memory"]["enabled"].as_bool(), Some(true));
    assert_eq!(doc["memory"]["dir"].as_str(), Some("./.oi/memory"));
    assert_eq!(doc["daemon"]["cwd"].as_str(), Some("/workspace"));
    assert_eq!(doc["daemon"]["max_turns"].as_integer(), Some(32));

    // [[mcp.servers]] 是表数组，不是内联数组
    let servers = doc["mcp"]["servers"]
        .as_array_of_tables()
        .expect("[mcp].servers 应保留为表数组");
    assert_eq!(servers.len(), 1, "[[mcp.servers]] 条目数应保留");
    let server = servers.get(0).expect("表数组首项应存在");
    assert_eq!(server["name"].as_str(), Some("fs"));
    assert_eq!(server["command"].as_str(), Some("npx"));
    assert_eq!(
        server["args"].as_array().map(|a| a.len()),
        Some(2),
        "args 数组内容应保留"
    );
}

/// 向后兼容：旧配置只有根表键、没有 `[llm]` 段，保存后 `[llm]` 段被正确创建且四键齐全。
#[test]
fn save_adds_missing_llm_section() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "# legacy config\nomp_path = \"omp\"\ndata_dir = \"./.oi\"\nmodel = \"legacy-model\"\n",
    )
    .expect("写配置失败");

    LlmRuntimeConfig {
        base_url: "http://127.0.0.1:3182".to_string(),
        api_key: "sk-new".to_string(),
        model: "agnes-2.5-flash".to_string(),
        max_tokens: 2048,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
    .save_to_file()
    .expect("保存配置失败");

    let doc = std::fs::read_to_string(sb.path().join(".oi/config.toml"))
        .expect("读回配置失败")
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后应为合法 TOML");

    let llm = doc
        .get("llm")
        .and_then(toml_edit::Item::as_table)
        .expect("save_to_file 应为缺段的配置补上 [llm]");
    assert_eq!(llm["base_url"].as_str(), Some("http://127.0.0.1:3182"));
    assert_eq!(llm["api_key"].as_str(), Some("sk-new"));
    assert_eq!(llm["model"].as_str(), Some("agnes-2.5-flash"));
    assert_eq!(llm["max_tokens"].as_integer(), Some(2048));

    // 原有根键保留，model 被管理键覆盖为结构体的值
    assert_eq!(doc["omp_path"].as_str(), Some("omp"));
    assert_eq!(doc["data_dir"].as_str(), Some("./.oi"));
    assert_eq!(doc["model"].as_str(), Some("agnes-2.5-flash"));
}

/// 注释由 toml_edit 原样保留（根表注释与段内注释都在）。
#[test]
fn save_preserves_comments() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "# my note\n\
         data_dir = \"./.oi\"\n\
         model = \"m\"\n\
         \n\
         [llm]\n\
         # provider endpoint\n\
         base_url = \"http://x\"\n\
         api_key = \"sk\"\n\
         model = \"m\"\n\
         max_tokens = 100\n",
    )
    .expect("写配置失败");

    LlmRuntimeConfig {
        base_url: "http://x".to_string(),
        api_key: "sk".to_string(),
        model: "m".to_string(),
        max_tokens: 100,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
    .save_to_file()
    .expect("保存配置失败");

    let saved = std::fs::read_to_string(sb.path().join(".oi/config.toml")).expect("读回配置失败");
    assert!(saved.contains("# my note"), "根表注释应保留: {saved}");
    assert!(
        saved.contains("# provider endpoint"),
        "段内注释应保留: {saved}"
    );
}

/// 解析失败回退全量写时，`omp_path` 若不在第一行也必须被抢救回来。
///
/// 真实配置几乎都以注释开头，而 `preserve_omp_path` 曾在行循环里用 `?`：
/// 第一行不匹配就从整个函数返回 None，第二行的 `omp_path` 于是被丢弃，
/// 回退写把用户的自定义路径静默换成默认 "omp"。
#[test]
fn fallback_preserves_omp_path_not_on_first_line() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    // 重复的根键 model 使整份文档无法解析，强制走备份 + 全量写回退路径。
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "# omenic configuration\n\
         omp_path = \"custom-omp\"\n\
         data_dir = \"./.oi\"\n\
         model = \"m\"\n\
         model = \"duplicate-key-breaks-parsing\"\n",
    )
    .expect("写配置失败");

    LlmRuntimeConfig {
        base_url: "http://x".to_string(),
        api_key: "sk".to_string(),
        model: "m".to_string(),
        max_tokens: 100,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
    .save_to_file()
    .expect("保存配置失败");

    let saved = std::fs::read_to_string(sb.path().join(".oi/config.toml")).expect("读回配置失败");
    assert!(
        saved.contains("custom-omp"),
        "回退全量写必须抢救第二行的自定义 omp_path: {saved}"
    );
    // 原文已备份，解析失败不丢配置。
    assert!(
        sb.path().join(".oi/config.toml.bak").exists(),
        "解析失败时应备份原文"
    );
}

/// `[llm]` 键存在但不是表（如 `llm = "x"`）时，增量保存必须返回错误而不是
/// panic——保存路径崩溃比拒绝写入更难诊断。
#[test]
fn save_errors_instead_of_panicking_when_llm_is_not_a_table() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "data_dir = \"./.oi\"\n\
         model = \"m\"\n\
         llm = \"not-a-table\"\n",
    )
    .expect("写配置失败");

    let res = LlmRuntimeConfig {
        base_url: "http://x".to_string(),
        api_key: "sk".to_string(),
        model: "m".to_string(),
        max_tokens: 100,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
    .save_to_file();

    let err = res.expect_err("畸形 [llm] 应返回错误");
    assert!(
        err.contains("[llm]"),
        "错误信息应指出是 [llm] 段的问题: {err}"
    );
}

/// MCP 表单按 name 编辑既有服务器：改 `command` 后保存，`env` / `reconnect`
/// 与两处注释必须逐字保留（G8-B「只动管理键」语义延伸到 [[mcp.servers]]）。
#[test]
fn mcp_edit_preserves_env_reconnect_and_comments() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "data_dir = \"./.oi\"\n\
         model = \"m\"\n\
         \n\
         [mcp]\n\
         \n\
         # filesystem server, spawn per session\n\
         [[mcp.servers]]\n\
         name = \"fs\"\n\
         # npx downloads on first run\n\
         command = \"npx\"\n\
         args = [\"-y\", \"@modelcontextprotocol/server-filesystem\"]\n\
         env = { RUST_LOG = \"debug\", FS_ROOT = \"/tmp\" }\n\
         \n\
         [mcp.servers.reconnect]\n\
         initial_delay_ms = 500\n\
         max_delay_ms = 30000\n\
         max_attempts = 10\n",
    )
    .expect("写配置失败");

    let mut loaded = LlmRuntimeConfig::load_from_system();

    assert_eq!(loaded.mcp_servers.len(), 1, "应加载出 1 台 MCP 服务器");
    assert_eq!(loaded.mcp_servers[0].name, "fs");
    assert_eq!(
        loaded.mcp_servers[0].args, "-y, @modelcontextprotocol/server-filesystem",
        "args 应以逗号分隔文本进表单"
    );

    // 模拟用户在表单里改 command 后保存
    loaded.mcp_servers[0].command = "npx-dlx".to_string();
    loaded.save_to_file().expect("保存配置失败");

    let saved = std::fs::read_to_string(sb.path().join(".oi/config.toml")).expect("读回配置失败");
    let doc = saved
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后的配置必须是合法 TOML");

    let server = doc["mcp"]["servers"]
        .as_array_of_tables()
        .and_then(|a| a.get(0))
        .expect("[[mcp.servers]] 首项应存在");
    // 管理键确实被更新
    assert_eq!(
        server["command"].as_str(),
        Some("npx-dlx"),
        "command 应被更新"
    );
    assert_eq!(
        server["args"].as_array().map(|a| a.len()),
        Some(2),
        "args 数组应保留"
    );

    // 未管理键逐字保留
    let env = server["env"].as_inline_table().expect("env 应保留为内联表");
    assert_eq!(env.get("RUST_LOG").and_then(|v| v.as_str()), Some("debug"));
    assert_eq!(env.get("FS_ROOT").and_then(|v| v.as_str()), Some("/tmp"));
    let reconnect = server["reconnect"]
        .as_table()
        .expect("reconnect 子表应保留");
    assert_eq!(reconnect["initial_delay_ms"].as_integer(), Some(500));
    assert_eq!(reconnect["max_delay_ms"].as_integer(), Some(30000));
    assert_eq!(reconnect["max_attempts"].as_integer(), Some(10));

    // 注释逐字保留：段头注释（表 decor）与键前注释（key decor，set_item 不动它）
    assert!(
        saved.contains("# filesystem server, spawn per session"),
        "[[mcp.servers]] 头部注释应保留: {saved}"
    );
    assert!(
        saved.contains("# npx downloads on first run"),
        "managed 键前的行注释应保留: {saved}"
    );
}

/// 文件原本没有 `[mcp]`：空表单保存不创建该段；表单添加服务器后保存，
/// `[[mcp.servers]]` 才出现，且未填的键（command/args/cwd）不写出。
#[test]
fn mcp_section_written_only_when_form_has_servers() {
    let sb = Sandbox::new();

    let base = LlmRuntimeConfig {
        base_url: "http://127.0.0.1:3182".to_string(),
        api_key: "sk-b".to_string(),
        model: "m".to_string(),
        max_tokens: 128,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    };
    base.save_to_file().expect("首次保存失败");

    let cfg_path = sb.path().join(".oi/config.toml");
    let after_empty = std::fs::read_to_string(&cfg_path).expect("读回配置失败");
    assert!(
        !after_empty.contains("[mcp]"),
        "空表单不得写 [mcp] 段: {after_empty}"
    );

    let mut with_server = LlmRuntimeConfig::load_from_system();
    with_server.mcp_servers.push(McpServerForm {
        name: "fetch".to_string(),
        command: String::new(),
        url: "http://127.0.0.1:9100/mcp".to_string(),
        args: String::new(),
        cwd: String::new(),
        tool_call_timeout_ms: "1500".to_string(),
        fail_on_startup_error: true,
    });
    with_server.save_to_file().expect("带服务器保存失败");

    let doc = std::fs::read_to_string(&cfg_path)
        .expect("读回配置失败")
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后的配置必须是合法 TOML");

    let servers = doc["mcp"]["servers"]
        .as_array_of_tables()
        .expect("应写出 [[mcp.servers]] 表数组");
    assert_eq!(servers.len(), 1);
    let server = servers.get(0).expect("表数组首项应存在");
    assert_eq!(server["name"].as_str(), Some("fetch"));
    // url 型服务器：command 未填 → 不写该键（而不是留一个空串值）
    assert!(server.get("command").is_none(), "空 command 不应写出");
    assert_eq!(server["url"].as_str(), Some("http://127.0.0.1:9100/mcp"));
    assert_eq!(server["tool_call_timeout_ms"].as_integer(), Some(1500));
    assert_eq!(server["fail_on_startup_error"].as_bool(), Some(true));
    assert!(server.get("args").is_none(), "空 args 不应写出");
    assert!(server.get("cwd").is_none(), "空 cwd 不应写出");
}

/// 首次保存（文件本不存在 → 走全量写）也必须带上 `[[mcp.servers]]`。
///
/// 全量写原先只拼 `[llm]` 与 `[[llm.fallbacks]]` 就落盘，而 `mcp_servers`
/// 非空时增量路径才会写表：于是新机器上的第一次保存会静默丢掉整张服务器
/// 表，直到第二次保存（文件已存在、走增量）才重新出现。
#[test]
fn full_write_persists_mcp_servers_on_first_save() {
    let sb = Sandbox::new();

    let cfg = LlmRuntimeConfig {
        base_url: "http://127.0.0.1:3183".to_string(),
        api_key: "sk-c".to_string(),
        model: "m".to_string(),
        max_tokens: 128,
        data_dir: "./.oi".to_string(),
        mcp_servers: vec![McpServerForm {
            name: "fetch".to_string(),
            command: String::new(),
            url: "http://127.0.0.1:9100/mcp".to_string(),
            args: String::new(),
            cwd: String::new(),
            tool_call_timeout_ms: "1500".to_string(),
            fail_on_startup_error: true,
        }],
        llm_fallbacks: Vec::new(),
    };
    // 目标文件不存在 → write_full_config 分支。
    assert!(
        !sb.path().join(".oi/config.toml").exists(),
        "本用例的前提是配置文件尚不存在"
    );
    cfg.save_to_file().expect("首次保存失败");

    let saved = std::fs::read_to_string(sb.path().join(".oi/config.toml")).expect("读回配置失败");
    let doc = saved
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后的配置必须是合法 TOML");
    let servers = doc["mcp"]["servers"]
        .as_array_of_tables()
        .expect("全量写也必须落地 [[mcp.servers]]");
    assert_eq!(servers.len(), 1, "服务器不得在首次保存时丢失");
    let server = servers.get(0).expect("表数组首项应存在");
    assert_eq!(server["name"].as_str(), Some("fetch"));
    assert_eq!(server["url"].as_str(), Some("http://127.0.0.1:9100/mcp"));
    assert_eq!(server["tool_call_timeout_ms"].as_integer(), Some(1500));
}

/// 表单状态里没有的服务器保存后原样保留——包括其 `env` 与未知键：
/// 表单是「按 name 追加/编辑」，绝不整体重写 `[mcp]`（删卡不等于删配置）。
#[test]
fn mcp_servers_missing_from_form_are_left_untouched() {
    let sb = Sandbox::new();

    std::fs::create_dir_all(sb.path().join(".oi")).expect("建 .oi 失败");
    std::fs::write(
        sb.path().join(".oi/config.toml"),
        "data_dir = \"./.oi\"\n\
         model = \"m\"\n\
         \n\
         [mcp]\n\
         \n\
         [[mcp.servers]]\n\
         name = \"fs\"\n\
         command = \"npx\"\n\
         args = [\"-y\", \"fs-server\"]\n\
         custom_future_key = \"keep-me\"\n\
         \n\
         [[mcp.servers]]\n\
         name = \"other\"\n\
         url = \"http://127.0.0.1:9200/mcp\"\n\
         env = { TOKEN = \"t\" }\n",
    )
    .expect("写配置失败");

    // 加载两台后把 other 从表单状态里删掉（模拟用户删卡），再编辑 fs
    let mut loaded = LlmRuntimeConfig::load_from_system();
    assert_eq!(loaded.mcp_servers.len(), 2, "应加载出 2 台 MCP 服务器");
    loaded.mcp_servers.retain(|s| s.name == "fs");
    loaded.mcp_servers[0].command = "npx-updated".to_string();
    loaded.save_to_file().expect("保存配置失败");

    let doc = std::fs::read_to_string(sb.path().join(".oi/config.toml"))
        .expect("读回配置失败")
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后的配置必须是合法 TOML");

    let servers = doc["mcp"]["servers"]
        .as_array_of_tables()
        .expect("[[mcp.servers]] 应保留为表数组");
    assert_eq!(
        servers.len(),
        2,
        "表单外的服务器不得被删除（按 name 编辑，不整体重写）"
    );

    // fs 原地更新，未知键保留
    let fs = servers.get(0).expect("表数组首项应存在");
    assert_eq!(fs["command"].as_str(), Some("npx-updated"));
    assert_eq!(
        fs["args"].as_array().map(|a| a.len()),
        Some(2),
        "args 数组应保留"
    );
    assert_eq!(
        fs["custom_future_key"].as_str(),
        Some("keep-me"),
        "服务器上的未知键必须逐字保留"
    );

    // other 完全不在表单状态里 → 逐键原样
    let other = servers.get(1).expect("表数组第二项应存在");
    assert_eq!(other["name"].as_str(), Some("other"));
    assert_eq!(other["url"].as_str(), Some("http://127.0.0.1:9200/mcp"));
    assert_eq!(
        other["env"]["TOKEN"].as_str(),
        Some("t"),
        "表单外服务器的 env 应原样保留"
    );
}

/// `[llm].fallbacks` 段 roundtrip：`save_to_file` 写 2 行（全字段 + 只 model）
/// → `load_from_system` 读回，行序不丢、空串字段没写成 `= ""`、max_tokens
/// 数值型保留（断言写法参照同文件 mcp 段先例）。
#[test]
fn llm_fallbacks_roundtrip_preserves_rows_and_clears_empty_fields() {
    let sb = Sandbox::new();

    let cfg = LlmRuntimeConfig {
        base_url: "http://primary.example.com".to_string(),
        api_key: "sk-primary".to_string(),
        model: "primary-model".to_string(),
        max_tokens: 4096,
        data_dir: "./.oi".to_string(),
        mcp_servers: Vec::new(),
        llm_fallbacks: vec![
            LlmFallbackForm {
                base_url: "http://relay-a.example.com/v1".to_string(),
                api_key: "sk-fallback-a".to_string(),
                model: "fallback-model-a".to_string(),
                max_tokens: "8192".to_string(),
            },
            // 只填 model 的行：空串字段必须不写出（而不是 `= ""`）
            LlmFallbackForm {
                base_url: String::new(),
                api_key: String::new(),
                model: "fallback-model-b".to_string(),
                max_tokens: String::new(),
            },
        ],
    };
    cfg.save_to_file().expect("保存配置失败");

    let doc = std::fs::read_to_string(sb.path().join(".oi/config.toml"))
        .expect("读回配置失败")
        .parse::<toml_edit::DocumentMut>()
        .expect("保存后的配置必须是合法 TOML");

    let raw = std::fs::read_to_string(sb.path().join(".oi/config.toml")).expect("读回失败");
    let fallbacks = doc["llm"]["fallbacks"]
        .as_array_of_tables()
        .expect("应写出 [[llm.fallbacks]] 表数组");
    assert_eq!(fallbacks.len(), 2, "两行 fallback 必须按序保留");

    // 全字段行
    let a = fallbacks.get(0).expect("表数组首项应存在");
    assert_eq!(a["model"].as_str(), Some("fallback-model-a"));
    assert_eq!(
        a["base_url"].as_str(),
        Some("http://relay-a.example.com/v1")
    );
    assert_eq!(a["api_key"].as_str(), Some("sk-fallback-a"));
    assert_eq!(
        a["max_tokens"].as_integer(),
        Some(8192),
        "max_tokens 必须以数值型保留"
    );

    // 只 model 行：空串字段键被清掉，不是空串值
    let b = fallbacks.get(1).expect("表数组第二项应存在");
    assert_eq!(b["model"].as_str(), Some("fallback-model-b"));
    assert!(b.get("base_url").is_none(), "空 base_url 不应写出");
    assert!(b.get("api_key").is_none(), "空 api_key 不应写出");
    assert!(b.get("max_tokens").is_none(), "空 max_tokens 不应写出");
    assert!(!raw.contains("base_url = \"\""), "不得出现空串值: {raw}");
    assert!(!raw.contains("api_key = \"\""), "不得出现空串值: {raw}");
    assert!(!raw.contains("max_tokens = \"\""), "不得出现空串值: {raw}");

    // load_from_system 读回：2 行按序不丢、空字段是空串、数值转回文本
    let loaded = LlmRuntimeConfig::load_from_system();
    assert_eq!(loaded.llm_fallbacks.len(), 2, "load 应读回两行");
    assert_eq!(loaded.llm_fallbacks[0], cfg.llm_fallbacks[0]);
    assert_eq!(
        loaded.llm_fallbacks[1],
        LlmFallbackForm {
            base_url: String::new(),
            api_key: String::new(),
            model: "fallback-model-b".to_string(),
            max_tokens: String::new(),
        },
        "只 model 行读回后其余字段应为空串"
    );
}
