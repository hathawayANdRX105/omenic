use crate::llm::LlmRuntimeConfig;
use dioxus::prelude::*;

#[component]
pub fn ConfigPage(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    let mut base_url = use_signal(|| config.base_url.clone());
    let mut api_key = use_signal(|| config.api_key.clone());
    let mut model = use_signal(|| config.model.clone());
    let mut max_tokens = use_signal(|| config.max_tokens.to_string());
    let mut data_dir = use_signal(|| config.data_dir.clone());

    let mut show_key = use_signal(|| false);
    let mut test_status = use_signal(|| None::<Result<Vec<String>, String>>);
    let mut save_status = use_signal(|| None::<Result<String, String>>);
    let mut is_testing = use_signal(|| false);

    let models = test_status().and_then(|r| r.ok()).unwrap_or_else(|| {
        vec![
            "agnes-2.5-flash".into(),
            "deepseek-v4-flash".into(),
            "claude-opus-4-7".into(),
            "qwen3-32b".into(),
            "kimi-k3".into(),
        ]
    });

    rsx! {
        div { class: "config-page",
            div { class: "config-header",
                h1 { "模型与渠道配置" }
                p { class: "config-subtitle", "真实管理 LLM API 渠道凭证、端点与运行时默认模型，配置实时持久化至 .oi/config.toml" }
            }

            // Save feedback banner
            if let Some(res) = save_status.read().as_ref() {
                match res {
                    Ok(msg) => rsx! {
                        div { class: "feedback-banner success", "{msg}" }
                    },
                    Err(err) => rsx! {
                        div { class: "feedback-banner error", "{err}" }
                    },
                }
            }

            // Test connection feedback banner
            if let Some(res) = test_status.read().as_ref() {
                match res {
                    Ok(list) => rsx! {
                        div { class: "feedback-banner success",
                            "✓ 连接成功！已从 API 探测到 {list.len()} 个在线可用模型"
                        }
                    },
                    Err(err) => rsx! {
                        div { class: "feedback-banner error",
                            "✗ 无法连接到端点: {err}"
                        }
                    },
                }
            }

            // Form Card
            div { class: "config-card",
                div { class: "config-card-header",
                    div { class: "config-card-title", "LLM 核心凭证与 API 端点" }
                    span { style: "font-size: 12px; color: #4ade80;", "● 支持 OpenAI 兼容规范 (如 new-api, 9router, vLLM)" }
                }

                div { class: "form-grid",
                    // Base URL
                    div { class: "form-group full",
                        label { class: "form-label", "Base URL (API 基地址，自动兼容 /v1)" }
                        input {
                            class: "form-input",
                            value: "{base_url}",
                            oninput: move |e| base_url.set(e.value()),
                            placeholder: "http://127.0.0.1:3182",
                        }
                    }

                    // API Key
                    div { class: "form-group full",
                        div { style: "display: flex; justify-content: space-between; align-items: center;",
                            label { class: "form-label", "API Key / Token" }
                            button {
                                class: "action-pill-btn",
                                style: "padding: 2px 8px;",
                                onclick: move |_| show_key.set(!show_key()),
                                if show_key() { "🔒 隐藏密钥" } else { "👁️ 显示明文" }
                            }
                        }
                        input {
                            class: "form-input",
                            r#type: if show_key() { "text" } else { "password" },
                            value: "{api_key}",
                            oninput: move |e| api_key.set(e.value()),
                            placeholder: "sk-...",
                        }
                    }

                    // Default Model
                    div { class: "form-group",
                        label { class: "form-label", "默认模型 (Model ID)" }
                        input {
                            class: "form-input",
                            value: "{model}",
                            oninput: move |e| model.set(e.value()),
                            placeholder: "agnes-2.5-flash",
                        }
                    }

                    // Max Tokens
                    div { class: "form-group",
                        label { class: "form-label", "单次推理 Max Tokens" }
                        input {
                            class: "form-input",
                            r#type: "number",
                            value: "{max_tokens}",
                            oninput: move |e| max_tokens.set(e.value()),
                            placeholder: "4096",
                        }
                    }

                    // Data Directory
                    div { class: "form-group full",
                        label { class: "form-label", "omenic 本地数据目录 (Data Directory)" }
                        input {
                            class: "form-input",
                            value: "{data_dir}",
                            oninput: move |e| data_dir.set(e.value()),
                            placeholder: "./.oi",
                        }
                    }
                }

                // Action Buttons
                div { class: "config-actions-row",
                    // Save Button
                    button {
                        class: "btn-primary-action",
                        onclick: move |_| {
                            let new_cfg = LlmRuntimeConfig {
                                base_url: base_url().trim().to_string(),
                                api_key: api_key().trim().to_string(),
                                model: model().trim().to_string(),
                                max_tokens: max_tokens().parse::<u32>().unwrap_or(4096),
                                data_dir: data_dir().trim().to_string(),
                            };
                            match new_cfg.save_to_file() {
                                Ok(()) => {
                                    save_status.set(Some(Ok("✓ 配置已成功写入配置文件并实时生效！".into())));
                                    on_update_config.call(new_cfg);
                                }
                                Err(e) => {
                                    save_status.set(Some(Err(format!("保存失败: {}", e))));
                                }
                            }
                        },
                        "💾 保存配置并应用"
                    }

                    // Test Connection Button
                    button {
                        class: "btn-secondary-action",
                        disabled: is_testing(),
                        onclick: move |_| {
                            is_testing.set(true);
                            let probe_cfg = LlmRuntimeConfig {
                                base_url: base_url().trim().to_string(),
                                api_key: api_key().trim().to_string(),
                                model: model().trim().to_string(),
                                max_tokens: max_tokens().parse::<u32>().unwrap_or(4096),
                                data_dir: data_dir().trim().to_string(),
                            };
                            let res = probe_cfg.test_connection();
                            test_status.set(Some(res));
                            is_testing.set(false);
                        },
                        if is_testing() { "⚡ 正在连接..." } else { "⚡ 测试连接与拉取模型" }
                    }
                }
            }

            // Online Models Card
            div { class: "config-card",
                div { class: "config-card-header",
                    div { class: "config-card-title", "在线可用模型列表 ({models.len()})" }
                    span { style: "font-size: 12px; color: #94a3b8;", "点击「设为当前模型」即可快速切换" }
                }

                div { style: "display: flex; flex-direction: column; gap: 8px;",
                    for m in &models {
                        {
                            let m_str = m.clone();
                            let is_current = *model.read() == *m;
                            rsx! {
                                div {
                                    key: "{m}",
                                    style: "display: flex; align-items: center; justify-content: space-between; padding: 10px 14px; background: #121318; border: 1px solid #232532; border-radius: 6px;",
                                    div { style: "display: flex; align-items: center; gap: 10px;",
                                        span { style: "color: #38bdf8; font-weight: 600; font-family: monospace; font-size: 13.5px;", "{m}" }
                                        if is_current {
                                            span { style: "background: #143525; color: #4ade80; border: 1px solid #1e5238; padding: 1px 6px; border-radius: 3px; font-size: 11px;", "当前默认" }
                                        }
                                    }
                                    button {
                                        class: if is_current { "action-pill-btn active" } else { "action-pill-btn" },
                                        disabled: is_current,
                                        onclick: move |_| {
                                            model.set(m_str.clone());
                                        },
                                        if is_current { "已选用" } else { "设为当前模型" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
