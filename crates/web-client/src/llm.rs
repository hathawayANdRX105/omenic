//! Real LLM integration and configuration persistence for omenic web.

use serde::{Deserialize, Serialize};
use std::path::Path;
use web_state::types::ChatMessage;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRuntimeConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub data_dir: String,
    /// 设置页「MCP 服务器」表单的行数据。空 vec 表示表单没有服务器，
    /// 保存时完全不碰 `[mcp]`（不创建、不改写）。
    pub mcp_servers: Vec<McpServerForm>,
    /// 设置页「Fallback Provider」表单的行数据（`[[llm.fallbacks]]`，
    /// 主 provider 失败后按序切换）。空 vec 表示表单没有行，保存时
    /// 完全不碰 `[llm].fallbacks`（不创建、不改写、不删除既有行）。
    pub llm_fallbacks: Vec<LlmFallbackForm>,
}

/// 设置页单个 fallback LLM provider 的表单行（web 侧 DTO，不依赖 config
/// crate）：与 `crates/infra/config` 的 `LlmFallbackConfig` 一一对应，但
/// 统一成文本框友好的 `String`——空串表示「未设置」（保存时该键被清掉，
/// 等价 config crate 的 `Option::None`）。
///
/// `model` 是必填的匹配键：保存按它定位既有 `[[llm.fallbacks]]` 表
/// （同 [`McpServerForm`] 按 `name` 匹配的先例），model 为空的行保存中止。
/// 空 `base_url` / `api_key` = 继承主 provider；空 `max_tokens` = 该
/// fallback 请求不带 max_tokens。
///
/// `Default` 是「添加 Fallback Provider」按钮的空白行：全空串。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmFallbackForm {
    /// fallback provider 的 base_url（API 基地址）；空串 = 继承主 provider
    pub base_url: String,
    /// fallback provider 的 api_key；空串 = 继承主 provider
    pub api_key: String,
    /// fallback provider 的 model（必填：空串的表单行不会保存）
    pub model: String,
    /// fallback provider 的 max_tokens 十进制文本；空串 = 该 provider 不带 max_tokens
    pub max_tokens: String,
}

/// 设置页单个 MCP 服务器的表单行（web 侧 DTO，不依赖 config crate）：
/// 与 `crates/infra/config` 的 `McpServerConfig` 一一对应，但统一成文本框
/// 友好的 `String`——空串表示「未设置」，`args` 是逗号分隔文本。
///
/// `env` / `reconnect` 不进表单（高级键，直接编辑 TOML）：保存路径按
/// `name` 匹配已有服务器，这两个键（及任何未知键）逐字保留。
///
/// `Default` 是「添加服务器」按钮的空白行：全空串 + false。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpServerForm {
    /// 短句柄；保存时按它匹配 `[[mcp.servers]]` 里的既有表。
    pub name: String,
    /// 可执行文件；空串 = 未设置（HTTP 服务器改用 `url`）。
    pub command: String,
    /// streamable-HTTP 端点；空串 = 未设置。
    pub url: String,
    /// 参数列表的逗号分隔文本（UI 形态，保存时切回 `Vec<String>`）。
    /// 局限：参数本身含逗号无法在此文本框表达。
    pub args: String,
    /// 子进程工作目录；空串 = 未设置（继承进程 cwd）。
    pub cwd: String,
    /// 单次 tool call 超时毫秒数的十进制文本；空串 = 用 crate 默认。
    pub tool_call_timeout_ms: String,
    /// 启动失败是否中止整个 MCP bring-up（false 与 serde 默认等价）。
    pub fail_on_startup_error: bool,
}

impl Default for LlmRuntimeConfig {
    fn default() -> Self {
        Self::load_from_system()
    }
}

impl LlmRuntimeConfig {
    /// Load settings from `.oi/config.toml`, environment variables, or fallback to local new-api.
    pub fn load_from_system() -> Self {
        let mut base_url = "http://127.0.0.1:3182".to_string();
        let mut api_key = "sk-config-not-set".to_string();
        let mut model = "agnes-2.5-flash".to_string();
        let mut max_tokens = 4096;
        let mut data_dir = "./.oi".to_string();
        let mut mcp_servers = Vec::new();
        let mut llm_fallbacks = Vec::new();

        // 1. Try reading config.toml
        for path in ["./.oi/config.toml", "../.oi/config.toml", "omenic.toml"] {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(value) = content.parse::<toml::Value>() {
                    if let Some(d) = value.get("data_dir").and_then(|v: &toml::Value| v.as_str()) {
                        data_dir = d.to_string();
                    }
                    if let Some(m) = value.get("model").and_then(|v: &toml::Value| v.as_str()) {
                        model = m.to_string();
                    }
                    if let Some(llm) = value.get("llm") {
                        if let Some(b) = llm.get("base_url").and_then(|v: &toml::Value| v.as_str())
                        {
                            base_url = b.to_string();
                        }
                        if let Some(k) = llm.get("api_key").and_then(|v: &toml::Value| v.as_str()) {
                            api_key = k.to_string();
                        }
                        if let Some(m) = llm.get("model").and_then(|v: &toml::Value| v.as_str()) {
                            model = m.to_string();
                        }
                        if let Some(t) = llm
                            .get("max_tokens")
                            .and_then(|v: &toml::Value| v.as_integer())
                        {
                            max_tokens = t as u32;
                        }
                    }

                    // [mcp] → 表单行。没有该段（或 servers 不是表数组）时
                    // 保持空 vec，表单显示「未配置」。
                    if let Some(servers) = value
                        .get("mcp")
                        .and_then(|m| m.get("servers"))
                        .and_then(toml::Value::as_array)
                    {
                        for server in servers {
                            if let Some(form) = mcp_server_form_from_value(server) {
                                mcp_servers.push(form);
                            }
                        }
                    }

                    // [[llm.fallbacks]] → 表单行（主 provider 失败后按序切换
                    // 的 fallback provider）。没有该段时保持空 vec。
                    if let Some(fallbacks) = value
                        .get("llm")
                        .and_then(|l| l.get("fallbacks"))
                        .and_then(toml::Value::as_array)
                    {
                        for fallback in fallbacks {
                            if let Some(form) = llm_fallback_form_from_value(fallback) {
                                llm_fallbacks.push(form);
                            }
                        }
                    }
                }
                break;
            }
        }

        // 2. Env overrides
        if let Ok(v) = std::env::var("NEWAPI_RELAY_TOKEN") {
            api_key = v;
        } else if let Ok(v) = std::env::var("OMENIC_LLM_API_KEY") {
            api_key = v;
        }

        if let Ok(v) = std::env::var("OMENIC_LLM_BASE_URL") {
            base_url = v;
        }
        if let Ok(v) = std::env::var("OMENIC_LLM_MODEL") {
            model = v;
        }

        Self {
            base_url,
            api_key,
            model,
            max_tokens,
            data_dir,
            mcp_servers,
            llm_fallbacks,
        }
    }

    /// Persist to `.oi/config.toml`.
    ///
    /// **增量写回**：若文件已存在，用 [`toml_edit::DocumentMut`] 只更新本结构体
    /// 管理的键——根表的 `omp_path` / `data_dir` / `model`、`[llm]` 段下的
    /// `base_url` / `api_key` / `model` / `max_tokens`，以及按 `name` 逐台
    /// 编辑的 `[[mcp.servers]]`（见 [`Self::mcp_servers`]）——**其余内容
    /// 原样保留**：`[memory]` / `[daemon]` 等未管理段、MCP 服务器上的
    /// `env` / `reconnect` / 未知键、注释、空行与排版。表单状态里没有的
    /// MCP 服务器一律不动（表单是追加/按名编辑，不做整体重写）。
    ///
    /// 旧实现用 `format!()` 整文件重写，只写上述 6 个键，会把配置页一次保存
    /// 变成对 `[mcp]` / `[memory]` / `[daemon]` 的静默抹除。
    pub fn save_to_file(&self) -> Result<(), String> {
        let dir = Path::new(&self.data_dir);
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
        }

        let target_path = dir.join("config.toml");

        // 文件已存在：增量更新，未管理区域逐字节保留。
        if let Ok(content) = std::fs::read_to_string(&target_path) {
            match content.parse::<toml_edit::DocumentMut>() {
                Ok(mut doc) => {
                    self.write_managed_keys(&mut doc)?;
                    return std::fs::write(&target_path, doc.to_string()).map_err(|e| {
                        format!("写入配置文件 {} 失败: {}", target_path.display(), e)
                    });
                }
                // 解析失败时绝不能让用户配置凭空消失：先备份原文，再回退全量写。
                Err(e) => {
                    let backup_path = target_path.with_extension("toml.bak");
                    // 备份本身失败时必须中止：继续全量写会覆盖掉唯一一份原文，
                    // 而它连备份都没有——正是本修复要消除的静默丢失。
                    if let Err(backup_err) = std::fs::write(&backup_path, &content) {
                        return Err(format!(
                            "配置文件 {} 解析失败({})，且备份原始内容到 {} 也失败({})；已中止写入以避免丢失配置",
                            target_path.display(),
                            e,
                            backup_path.display(),
                            backup_err
                        ));
                    }
                    eprintln!(
                        "warn: 配置文件 {} 解析失败({})，已备份为 {} 后回退全量写",
                        target_path.display(),
                        e,
                        backup_path.display()
                    );
                    // 全量写也只补默认 omp_path——原文里若已有自定义路径，写死
                    // "omp" 会抹掉它（与 write_managed_keys 的缺失才补同语义）。
                    return self
                        .write_full_config(&target_path, preserve_omp_path(&content).as_deref());
                }
            }
        }

        self.write_full_config(&target_path, None)
    }

    /// 全量写一份只含管理键的新配置（文件不存在或原文件不可解析时使用）。
    /// `omp_path` 由调用方决定：`None` 用默认 `"omp"`，`Some(v)` 沿用原值。
    /// 空表单不写 `[mcp]` 段（与增量路径一致）；非空时复用
    /// `write_mcp_servers` 落地，否则首次保存（文件本不存在）或解析失败回退
    /// 到全量写时整张 `[[mcp.servers]]` 会被静默丢掉。
    fn write_full_config(&self, target_path: &Path, omp_path: Option<&str>) -> Result<(), String> {
        let mut toml_content = format!(
            "# omenic configuration\n\
             omp_path = \"{}\"\n\
             data_dir = \"{}\"\n\
             model = \"{}\"\n\n\
             [llm]\n\
             base_url = \"{}\"\n\
             api_key = \"{}\"\n\
             model = \"{}\"\n\
             max_tokens = {}\n",
            omp_path.unwrap_or("omp"),
            self.data_dir,
            self.model,
            self.base_url,
            self.api_key,
            self.model,
            self.max_tokens
        );

        for fallback in &self.llm_fallbacks {
            let model = fallback.model.trim();
            if model.is_empty() {
                return Err("[llm].fallbacks 存在缺少 model 的 provider，已中止保存".to_string());
            }
            toml_content.push_str(&format!("\n[[llm.fallbacks]]\nmodel = \"{}\"\n", model));
            let base_url = fallback.base_url.trim();
            if !base_url.is_empty() {
                toml_content.push_str(&format!("base_url = \"{}\"\n", base_url));
            }
            let api_key = fallback.api_key.trim();
            if !api_key.is_empty() {
                toml_content.push_str(&format!("api_key = \"{}\"\n", api_key));
            }
            let max_tokens = fallback.max_tokens.trim();
            if !max_tokens.is_empty() {
                let tokens: u32 = max_tokens.parse().map_err(|_| {
                    format!(
                        "[llm].fallbacks 的 max_tokens {:?} 不是有效的正整数",
                        max_tokens
                    )
                })?;
                toml_content.push_str(&format!("max_tokens = {}\n", tokens));
            }
        }

        // 没有服务器时保持纯字符串写（与历史输出逐字节一致）；非空服务器表
        // 必须走 `write_mcp_servers`：它是增量路径写 args/env/timeout/url
        // 的唯一实现，全量写自己拼一份会静默丢掉整张 [[mcp.servers]]。
        if self.mcp_servers.is_empty() {
            return std::fs::write(target_path, toml_content)
                .map_err(|e| format!("写入配置文件 {} 失败: {}", target_path.display(), e));
        }
        let mut doc = toml_content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("生成的配置文本无法解析: {}", e))?;
        write_mcp_servers(doc.as_table_mut(), &self.mcp_servers)?;
        std::fs::write(target_path, doc.to_string())
            .map_err(|e| format!("写入配置文件 {} 失败: {}", target_path.display(), e))
    }

    /// 把本结构体管理的键写进已解析的文档；文档里的其它键、段、注释、排版一律不动。
    fn write_managed_keys(&self, doc: &mut toml_edit::DocumentMut) -> Result<(), String> {
        let root = doc.as_table_mut();
        // `omp_path` 没有对应字段，只在缺失时补上默认值——直接写死 "omp" 会抹掉
        // 用户在别处配好的自定义路径，正是本修复要消除的那类静默覆盖。
        if !root.contains_key("omp_path") {
            root.insert("omp_path", toml_edit::value("omp"));
        }
        set_item(root, "data_dir", toml_edit::value(self.data_dir.as_str()));
        set_item(root, "model", toml_edit::value(self.model.as_str()));

        if !root.contains_key("llm") {
            root.insert("llm", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        // 只在「llm 存在但不是表」（如 `llm = "x"`）时失败：上面刚确保过键存在，
        // 到这里还取不到表说明用户配置本身畸形。panic 会让保存路径崩溃，返回
        // 错误让调用方决定更安全。
        let llm = root
            .get_mut("llm")
            .and_then(toml_edit::Item::as_table_mut)
            .ok_or_else(|| "[llm] 段已存在但不是表，无法增量更新".to_string())?;
        set_item(llm, "base_url", toml_edit::value(self.base_url.as_str()));
        set_item(llm, "api_key", toml_edit::value(self.api_key.as_str()));
        set_item(llm, "model", toml_edit::value(self.model.as_str()));
        // TOML 整数是 i64；u32 → i64 无损。
        set_item(llm, "max_tokens", toml_edit::value(self.max_tokens as i64));

        write_llm_fallbacks(llm, &self.llm_fallbacks)?;
        write_mcp_servers(root, &self.mcp_servers)
    }

    /// Test connection by querying /v1/models
    pub fn test_connection(&self) -> Result<Vec<String>, String> {
        let clean_base = self.base_url.trim_end_matches('/');
        let url = if clean_base.ends_with("/v1") {
            format!("{}/models", clean_base)
        } else {
            format!("{}/v1/models", clean_base)
        };

        let resp = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(8))
            .call()
            .map_err(|e| format!("连接失败: {}", e))?;

        let json: serde_json::Value = resp
            .into_json()
            .map_err(|e| format!("解析返回 JSON 失败: {}", e))?;

        let mut models = Vec::new();
        if let Some(list) = json.get("data").and_then(|v| v.as_array()) {
            for item in list {
                if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                    models.push(id.to_string());
                }
            }
        }

        if models.is_empty() {
            models.push(self.model.clone());
        }

        Ok(models)
    }

    /// Call real chat completions endpoint
    pub fn chat(&self, messages: &[ChatMessage]) -> Result<String, String> {
        let clean_base = self.base_url.trim_end_matches('/');
        let url = if clean_base.ends_with("/v1") {
            format!("{}/chat/completions", clean_base)
        } else {
            format!("{}/v1/chat/completions", clean_base)
        };

        // Convert messages to openai format
        let payload_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content
                })
            })
            .collect();

        let body = serde_json::json!({
            "model": self.model,
            "messages": payload_messages,
            "max_tokens": self.max_tokens,
            "temperature": 0.7
        });

        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(45))
            .send_json(body)
            .map_err(|e| match e {
                ureq::Error::Status(code, resp) => {
                    let text = resp.into_string().unwrap_or_default();
                    format!("API 状态码错误 {}: {}", code, text)
                }
                ureq::Error::Transport(t) => format!("网络传输错误: {}", t),
            })?;

        let json: serde_json::Value = resp
            .into_json()
            .map_err(|e| format!("解析回复 JSON 失败: {}", e))?;

        if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
            Ok(content.to_string())
        } else {
            Err("回复中未包含有效的 message.content".to_string())
        }
    }
}

/// 写入一个管理键：已存在就原地替换 value，不存在才 insert。
///
/// 不能无条件用 `Table::insert`——它对已存在的键会调 `Key::fmt()`，把 key 的
/// decor 清掉，而 toml_edit 把「行前注释」（如 `# provider endpoint`）挂在
/// key 的 decor 上，那样注释就跟着没了。只换 value 时 key 与其 decor 原地不动，
/// 注释/排版得以保留。
fn set_item(table: &mut toml_edit::Table, key: &str, item: toml_edit::Item) {
    if let Some(slot) = table.get_mut(key) {
        *slot = item;
    } else {
        table.insert(key, item);
    }
}

/// 把表单的 fallback provider 行写进 `[llm].fallbacks` 表数组（`[[llm.fallbacks]]`，
/// G8-B 语义的 MCP 先例照搬，匹配键换成 `model`）：
///
/// - 空表单**完全不碰** `[llm].fallbacks`——不创建该数组，既有的行、注释、
///   排版原样保留；
/// - 表单里的每一行按 `model` 匹配既有表：命中就只替换管理键
///   （base_url/api_key/model/max_tokens），未知键及其注释逐字保留；
///   未命中才追加新表；
/// - 文件里有、表单里没有的 fallback 一律不动——表单是追加/按 model 编辑，
///   绝不整体重写（删卡不等于删配置）。
fn write_llm_fallbacks(
    llm: &mut toml_edit::Table,
    forms: &[LlmFallbackForm],
) -> Result<(), String> {
    if forms.is_empty() {
        return Ok(());
    }

    if !llm.contains_key("fallbacks") {
        llm.insert(
            "fallbacks",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    let fallbacks = llm
        .get_mut("fallbacks")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            "[llm].fallbacks 已存在但不是 [[llm.fallbacks]] 表数组，无法增量更新".to_string()
        })?;

    for form in forms {
        let model = form.model.trim();
        if model.is_empty() {
            // model 是匹配键：没有它既无法定位旧表也无法命名新表，
            // 与 [mcp] 的「缺少 name 中止保存」同语义。
            return Err("[llm].fallbacks 存在缺少 model 的 provider，已中止保存".to_string());
        }
        let existing = fallbacks
            .iter()
            .position(|t| t.get("model").and_then(toml_edit::Item::as_str) == Some(model));
        match existing {
            Some(idx) => {
                // 匹配键本身不重写（保住 model 上的注释与排版），只动管理键。
                let table = fallbacks
                    .get_mut(idx)
                    .expect("position 刚返回的索引必然存在");
                update_fallback_managed_keys(table, form)?;
            }
            None => {
                let mut table = toml_edit::Table::new();
                table.insert("model", toml_edit::value(model));
                update_fallback_managed_keys(&mut table, form)?;
                fallbacks.push(table);
            }
        }
    }
    Ok(())
}

/// 把一个 fallback 表单行的管理键写进单个 `[[llm.fallbacks]]` 表：
/// 空串字段清键（等价 config crate 的 `Option::None`，不留下 `= ""`），
/// 非空写值；`max_tokens` 解析 u32 后写 i64（TOML 整数是 i64，u32 → i64 无损）。
fn update_fallback_managed_keys(
    table: &mut toml_edit::Table,
    form: &LlmFallbackForm,
) -> Result<(), String> {
    set_or_clear_str(table, "base_url", &form.base_url);
    set_or_clear_str(table, "api_key", &form.api_key);

    let tokens_text = form.max_tokens.trim();
    if tokens_text.is_empty() {
        table.remove("max_tokens");
    } else {
        let tokens: u32 = tokens_text.parse().map_err(|_| {
            format!(
                "[llm].fallbacks provider {:?} 的 max_tokens 不是有效的正整数: {:?}",
                form.model.trim(),
                tokens_text
            )
        })?;
        set_item(table, "max_tokens", toml_edit::value(tokens as i64));
    }
    Ok(())
}

/// 把表单的 MCP 服务器行写进文档的 `[[mcp.servers]]` 表数组（G8-B 语义的
/// MCP 延伸）：
///
/// - 空表单**完全不碰** `[mcp]`——不创建该段，已有的服务器、注释、排版
///   原样保留（与没有 MCP 表单的旧版保存行为一致）；
/// - 表单里的每一行按 `name` 匹配既有表：命中就只替换管理键
///   （command/url/args/cwd/tool_call_timeout_ms/fail_on_startup_error），
///   `env` / `reconnect` / 未知键及其注释逐字保留；未命中才追加新表；
/// - 文件里有、表单里没有的服务器一律不动——表单是追加/按名编辑，
///   绝不整体重写。
fn write_mcp_servers(root: &mut toml_edit::Table, forms: &[McpServerForm]) -> Result<(), String> {
    if forms.is_empty() {
        return Ok(());
    }

    if !root.contains_key("mcp") {
        root.insert("mcp", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    // 与 [llm] 同样的畸形防御：段存在但不是表时返回错误而不是 panic。
    let mcp = root
        .get_mut("mcp")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| "[mcp] 段已存在但不是表，无法增量更新".to_string())?;
    if !mcp.contains_key("servers") {
        mcp.insert(
            "servers",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    let servers = mcp
        .get_mut("servers")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            "[mcp].servers 已存在但不是 [[mcp.servers]] 表数组，无法增量更新".to_string()
        })?;

    for form in forms {
        let name = form.name.trim();
        if name.is_empty() {
            // name 是匹配键：没有它既无法定位旧表也无法命名新表，
            // 与其静默追加一台无名服务器，不如让调用方修好表单再存。
            return Err("[mcp] 存在缺少 name 的服务器，已中止保存".to_string());
        }
        let existing = servers
            .iter()
            .position(|t| t.get("name").and_then(toml_edit::Item::as_str) == Some(name));
        match existing {
            Some(idx) => {
                // 匹配键本身不重写（保住 name 上的注释与排版），只动管理键。
                let table = servers.get_mut(idx).expect("position 刚返回的索引必然存在");
                update_server_managed_keys(table, form)?;
            }
            None => {
                let mut table = toml_edit::Table::new();
                table.insert("name", toml_edit::value(name));
                update_server_managed_keys(&mut table, form)?;
                servers.push(table);
            }
        }
    }
    Ok(())
}

/// 把一个服务器表单行的管理键写进单个 `[[mcp.servers]]` 表。
///
/// 空串字段表示「未设置」：原地**清键**而不是写空串/零值（与 config crate
/// 的 `Option` / `#[serde(default)]` 语义一致，也避免留下 `command = ""`
/// 这类会让 spawn 失败的值）；`fail_on_startup_error` 为 false 时同样清键
/// （false 就是 serde 默认）。`env` / `reconnect` / 未知键根本不触碰。
fn update_server_managed_keys(
    table: &mut toml_edit::Table,
    form: &McpServerForm,
) -> Result<(), String> {
    set_or_clear_str(table, "command", &form.command);
    set_or_clear_str(table, "url", &form.url);

    let args = split_args_text(&form.args);
    if args.is_empty() {
        table.remove("args");
    } else {
        let mut arr = toml_edit::Array::new();
        for arg in args {
            arr.push(arg);
        }
        set_item(table, "args", toml_edit::value(arr));
    }

    set_or_clear_str(table, "cwd", &form.cwd);

    let timeout_text = form.tool_call_timeout_ms.trim();
    if timeout_text.is_empty() {
        table.remove("tool_call_timeout_ms");
    } else {
        let ms: u64 = timeout_text.parse().map_err(|_| {
            format!(
                "[mcp] 服务器 {:?} 的 tool_call_timeout_ms 不是有效的毫秒数: {:?}",
                form.name.trim(),
                timeout_text
            )
        })?;
        let ms = i64::try_from(ms).map_err(|_| {
            format!(
                "[mcp] 服务器 {:?} 的 tool_call_timeout_ms 超出可表示范围",
                form.name.trim()
            )
        })?;
        set_item(table, "tool_call_timeout_ms", toml_edit::value(ms));
    }

    if form.fail_on_startup_error {
        set_item(table, "fail_on_startup_error", toml_edit::value(true));
    } else {
        table.remove("fail_on_startup_error");
    }
    Ok(())
}

/// 写一个「空串 = 未设置」的字符串管理键：非空就原地替换 value（保住键的
/// 注释与排版），空串则把整个键清掉。
fn set_or_clear_str(table: &mut toml_edit::Table, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        table.remove(key);
    } else {
        set_item(table, key, toml_edit::value(value));
    }
}

/// 逗号分隔文本 → 参数列表：按逗号切分、逐项去首尾空白、丢弃空项。
/// 局限见 [`McpServerForm::args`]：参数本身含逗号时直接编辑 TOML。
fn split_args_text(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 把一个 `[[mcp.servers]]` 的 toml 值转成表单行；缺 `name` 的条目返回
/// `None`（不进表单）。无名条目因此也永远不会被保存路径改写——保存按
/// `name` 匹配，表单外的一切原样保留。
fn mcp_server_form_from_value(server: &toml::Value) -> Option<McpServerForm> {
    let name = server.get("name")?.as_str()?;
    let args = server
        .get("args")
        .and_then(toml::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Some(McpServerForm {
        name: name.to_string(),
        command: toml_str_field(server, "command"),
        url: toml_str_field(server, "url"),
        args,
        cwd: toml_str_field(server, "cwd"),
        tool_call_timeout_ms: server
            .get("tool_call_timeout_ms")
            .and_then(toml::Value::as_integer)
            .map(|n| n.to_string())
            .unwrap_or_default(),
        fail_on_startup_error: server
            .get("fail_on_startup_error")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
    })
}

/// 取一个字符串字段的值，缺失或类型不符时给空串（表单语义：未设置）。
fn toml_str_field(server: &toml::Value, key: &str) -> String {
    server
        .get(key)
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// 把一个 `[[llm.fallbacks]]` 的 toml 值转成表单行；缺 `model` 的条目返回
/// `None`（不进表单——model 是保存路径的匹配键，同 mcp 的 name）。
fn llm_fallback_form_from_value(fallback: &toml::Value) -> Option<LlmFallbackForm> {
    let model = fallback.get("model")?.as_str()?;
    Some(LlmFallbackForm {
        base_url: toml_str_field(fallback, "base_url"),
        api_key: toml_str_field(fallback, "api_key"),
        model: model.to_string(),
        max_tokens: fallback
            .get("max_tokens")
            .and_then(toml::Value::as_integer)
            .map(|n| n.to_string())
            .unwrap_or_default(),
    })
}

/// 从一份**不可解析**的原始配置里抢救 `omp_path` 的值（若有）。
///
/// 文件能正常解析时走增量路径，`write_managed_keys` 已经「缺失才补」地保住了
/// 自定义路径；只有解析失败回退全量写时才需要这里——用行扫描而非 toml 解析，
/// 因为调用前提就是这个文件解析不了。格式必须是 `omp_path = "value"`。
fn preserve_omp_path(unparsed: &str) -> Option<String> {
    for line in unparsed.lines() {
        let trimmed = line.trim();
        // 必须 continue 而不是 ?：真实配置首行通常是注释，`?` 会让第一行
        // 不匹配就把整个函数返回成 None，第 2 行的 omp_path 于是被丢弃——
        // 正是这个函数要避免的丢失。
        let rest = match trimmed.strip_prefix("omp_path") {
            Some(r) => r,
            None => continue,
        };
        let rest = rest.trim_start();
        let rest = match rest.strip_prefix('=') {
            Some(r) => r,
            None => continue,
        };
        let rest = rest.trim();
        let value = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"'));
        if let Some(v) = value {
            return Some(v.to_string());
        }
        // 无引号形式也接受，取到行尾（去注释与空白）。
        return Some(rest.split('#').next().unwrap_or(rest).trim().to_string());
    }
    None
}
