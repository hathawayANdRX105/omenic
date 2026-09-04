//! Real LLM integration and configuration persistence for omenic web.

use crate::mock::ChatMessage;
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
        let mut api_key = "sk-sj93dHD9Wgn4jblgLVOfKFopInxHSvfB4Y9L1AbHbV6CMObg".to_string();
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

    /// Persist to `.oi/config.toml`
    pub fn save_to_file(&self) -> Result<(), String> {
        let dir = Path::new(&self.data_dir);
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
        }

        let toml_content = format!(
            "# omenic configuration\n\
             omp_path = \"omp\"\n\
             data_dir = \"{}\"\n\
             model = \"{}\"\n\n\
             [llm]\n\
             base_url = \"{}\"\n\
             api_key = \"{}\"\n\
             model = \"{}\"\n\
             max_tokens = {}\n",
            self.data_dir, self.model, self.base_url, self.api_key, self.model, self.max_tokens
        );

        let target_path = dir.join("config.toml");
        std::fs::write(&target_path, toml_content)
            .map_err(|e| format!("写入配置文件 {} 失败: {}", target_path.display(), e))
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
