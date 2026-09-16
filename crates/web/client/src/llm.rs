//! Real LLM integration and configuration persistence for omenic web.

use omenic_web_state::types::ChatMessage;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRuntimeConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub data_dir: String,
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
        }
    }

    /// Persist to `.oi/config.toml`.
    ///
    /// **增量写回**：若文件已存在，用 [`toml_edit::DocumentMut`] 只更新本结构体
    /// 管理的键——根表的 `omp_path` / `data_dir` / `model` 与 `[llm]` 段下的
    /// `base_url` / `api_key` / `model` / `max_tokens`——**其余内容原样保留**：
    /// `[mcp]` / `[memory]` / `[daemon]` 等未管理段、注释、空行与排版。
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
                    self.write_managed_keys(&mut doc);
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
    fn write_full_config(&self, target_path: &Path, omp_path: Option<&str>) -> Result<(), String> {
        let toml_content = format!(
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

        std::fs::write(target_path, toml_content)
            .map_err(|e| format!("写入配置文件 {} 失败: {}", target_path.display(), e))
    }

    /// 把本结构体管理的键写进已解析的文档；文档里的其它键、段、注释、排版一律不动。
    fn write_managed_keys(&self, doc: &mut toml_edit::DocumentMut) {
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
        let llm = root
            .get_mut("llm")
            .and_then(toml_edit::Item::as_table_mut)
            .expect("[llm] 段刚被确保存在");
        set_item(llm, "base_url", toml_edit::value(self.base_url.as_str()));
        set_item(llm, "api_key", toml_edit::value(self.api_key.as_str()));
        set_item(llm, "model", toml_edit::value(self.model.as_str()));
        // TOML 整数是 i64；u32 → i64 无损。
        set_item(llm, "max_tokens", toml_edit::value(self.max_tokens as i64));
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

/// 从一份**不可解析**的原始配置里抢救 `omp_path` 的值（若有）。
///
/// 文件能正常解析时走增量路径，`write_managed_keys` 已经「缺失才补」地保住了
/// 自定义路径；只有解析失败回退全量写时才需要这里——用行扫描而非 toml 解析，
/// 因为调用前提就是这个文件解析不了。格式必须是 `omp_path = "value"`。
fn preserve_omp_path(unparsed: &str) -> Option<String> {
    for line in unparsed.lines() {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("omp_path")?;
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('=')?;
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
