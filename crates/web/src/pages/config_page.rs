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

    // ── Real-time UI Form Validations (UI 校验功能) ─────────────────────────
    let url_val = base_url();
    let url_error: Option<&'static str> = if url_val.trim().is_empty() {
        Some("Base URL 不能为空")
    } else if !url_val.starts_with("http://") && !url_val.starts_with("https://") {
        Some("URL 协议非法：必须以 http:// 或 https:// 开头")
    } else if url_val.contains(' ') {
        Some("URL 不能包含空格字符")
    } else {
        None
    };

    let key_val = api_key();
    let key_error: Option<&'static str> = if key_val.trim().is_empty() {
        Some("API Key / Token 不能为空")
    } else if key_val.trim().len() < 8 {
        Some("API Key 长度不足 8 个字符，请确认格式完整")
    } else {
        None
    };

    let model_val = model();
    let model_error: Option<&'static str> = if model_val.trim().is_empty() {
        Some("默认模型标识 (Model ID) 不能为空")
    } else {
        None
    };

    let tokens_val = max_tokens();
    let tokens_error: Option<&'static str> = match tokens_val.trim().parse::<u32>() {
        Ok(v) if v < 16 => Some("Max Tokens 过小，建议至少 16"),
        Ok(v) if v > 200_000 => Some("Max Tokens 超过上限 200,000"),
        Ok(_) => None,
        Err(_) => Some("必须为有效正整数数值"),
    };

    let dir_val = data_dir();
    let dir_error: Option<&'static str> = if dir_val.trim().is_empty() {
        Some("数据目录路径不能为空")
    } else {
        None
    };

    let is_form_valid = url_error.is_none()
        && key_error.is_none()
        && model_error.is_none()
        && tokens_error.is_none()
        && dir_error.is_none();

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
        div {
            class: "config-page",
            style: "padding: 32px 40px; max-width: 960px; margin: 0 auto; overflow-y: auto; height: calc(100vh - 50px); color: #f1f5f9;",

            // Header Section
            div {
                class: "config-header",
                style: "margin-bottom: 28px;",
                div { style: "display: flex; align-items: center; justify-content: space-between;",
                    div {
                        h1 { style: "font-size: 24px; font-weight: 700; color: #f8fafc; margin-bottom: 6px;", "模型与渠道配置" }
                        p { style: "font-size: 13.5px; color: #94a3b8;", "管理 LLM API 渠道端点、安全凭证与默认模型，配置实时校验并持久化至 .oi/config.toml" }
                    }
                    div {
                        style: if is_form_valid {
                            "padding: 4px 12px; border-radius: 6px; background: #143525; border: 1px solid #1e5238; color: #4ade80; font-size: 12px; font-weight: 600;"
                        } else {
                            "padding: 4px 12px; border-radius: 6px; background: #3b181a; border: 1px solid #572528; color: #f87171; font-size: 12px; font-weight: 600;"
                        },
                        if is_form_valid { "● 表单校验通过" } else { "⚠️ 存在未通过的校验项" }
                    }
                }
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
            div {
                class: "config-card",
                style: "background: #17181f; border: 1px solid #282a36; border-radius: 12px; padding: 24px; margin-bottom: 24px; box-shadow: 0 4px 20px rgba(0,0,0,0.3);",

                div {
                    class: "config-card-header",
                    style: "display: flex; align-items: center; justify-content: space-between; margin-bottom: 20px; padding-bottom: 14px; border-bottom: 1px solid #22242e;",
                    div { style: "font-size: 16px; font-weight: 600; color: #f8fafc;", "LLM 核心凭证与 API 端点" }
                    span { style: "font-size: 12px; color: #4ade80; background: #13271d; border: 1px solid #1c3b2c; padding: 2px 8px; border-radius: 4px;", "● 支持 OpenAI 兼容规范 (如 new-api, 9router, vLLM)" }
                }

                div {
                    class: "form-grid",
                    style: "display: grid; grid-template-columns: 1fr 1fr; gap: 20px;",

                    // Base URL Field
                    div {
                        class: "form-group full",
                        style: "grid-column: 1 / -1; display: flex; flex-direction: column; gap: 6px;",
                        div { style: "display: flex; justify-content: space-between;",
                            label { style: "font-size: 13px; font-weight: 600; color: #cbd5e1;", "Base URL (API 基地址，自动兼容 /v1)" }
                            if let Some(err) = url_error {
                                span { style: "font-size: 12px; color: #f87171; font-weight: 500;", "⚠️ {err}" }
                            }
                        }
                        input {
                            class: if url_error.is_some() { "form-input error" } else { "form-input" },
                            style: if url_error.is_some() {
                                "padding: 10px 14px; background: #1a1518 !important; border: 1px solid #ef4444 !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none;"
                            } else {
                                "padding: 10px 14px; background: #121317 !important; border: 1px solid #2b2e3c !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none;"
                            },
                            value: "{base_url}",
                            oninput: move |e| base_url.set(e.value()),
                            placeholder: "http://127.0.0.1:3182",
                        }
                        span { style: "font-size: 11.5px; color: #64748b;", "如本地部署的 new-api 默认为 http://127.0.0.1:3182，远程服务如 https://api.openai.com" }
                    }

                    // API Key Field
                    div {
                        class: "form-group full",
                        style: "grid-column: 1 / -1; display: flex; flex-direction: column; gap: 6px;",
                        div { style: "display: flex; justify-content: space-between; align-items: center;",
                            div { style: "display: flex; align-items: center; gap: 8px;",
                                label { style: "font-size: 13px; font-weight: 600; color: #cbd5e1;", "API Key / Bearer Token" }
                                if let Some(err) = key_error {
                                    span { style: "font-size: 12px; color: #f87171; font-weight: 500;", "⚠️ {err}" }
                                }
                            }
                            button {
                                style: "background: #232530; border: 1px solid #333646; border-radius: 6px; color: #94a3b8; font-size: 12px; padding: 3px 10px; cursor: pointer;",
                                onclick: move |_| show_key.set(!show_key()),
                                if show_key() { "🔒 隐藏密钥" } else { "👁️ 显示明文" }
                            }
                        }
                        input {
                            class: if key_error.is_some() { "form-input error" } else { "form-input" },
                            style: if key_error.is_some() {
                                "padding: 10px 14px; background: #1a1518 !important; border: 1px solid #ef4444 !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none; font-family: monospace;"
                            } else {
                                "padding: 10px 14px; background: #121317 !important; border: 1px solid #2b2e3c !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none; font-family: monospace;"
                            },
                            r#type: if show_key() { "text" } else { "password" },
                            value: "{api_key}",
                            oninput: move |e| api_key.set(e.value()),
                            placeholder: "sk-...",
                        }
                        span { style: "font-size: 11.5px; color: #64748b;", "用于鉴权的 Bearer 令牌，存入本地 .oi/config.toml，不向任何第三方泄露" }
                    }

                    // Default Model
                    div {
                        class: "form-group",
                        style: "display: flex; flex-direction: column; gap: 6px;",
                        div { style: "display: flex; justify-content: space-between;",
                            label { style: "font-size: 13px; font-weight: 600; color: #cbd5e1;", "默认模型 (Model ID)" }
                            if let Some(err) = model_error {
                                span { style: "font-size: 12px; color: #f87171;", "⚠️ {err}" }
                            }
                        }
                        input {
                            class: "form-input",
                            style: "padding: 10px 14px; background: #121317 !important; border: 1px solid #2b2e3c !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none;",
                            value: "{model}",
                            oninput: move |e| model.set(e.value()),
                            placeholder: "agnes-2.5-flash",
                        }
                    }

                    // Max Tokens
                    div {
                        class: "form-group",
                        style: "display: flex; flex-direction: column; gap: 6px;",
                        div { style: "display: flex; justify-content: space-between;",
                            label { style: "font-size: 13px; font-weight: 600; color: #cbd5e1;", "单次推理 Max Tokens" }
                            if let Some(err) = tokens_error {
                                span { style: "font-size: 12px; color: #f87171;", "⚠️ {err}" }
                            }
                        }
                        input {
                            class: "form-input",
                            style: "padding: 10px 14px; background: #121317 !important; border: 1px solid #2b2e3c !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none;",
                            r#type: "number",
                            value: "{max_tokens}",
                            oninput: move |e| max_tokens.set(e.value()),
                            placeholder: "4096",
                        }
                    }

                    // Data Directory
                    div {
                        class: "form-group full",
                        style: "grid-column: 1 / -1; display: flex; flex-direction: column; gap: 6px;",
                        div { style: "display: flex; justify-content: space-between;",
                            label { style: "font-size: 13px; font-weight: 600; color: #cbd5e1;", "omenic 本地数据目录 (Data Directory)" }
                            if let Some(err) = dir_error {
                                span { style: "font-size: 12px; color: #f87171;", "⚠️ {err}" }
                            }
                        }
                        input {
                            class: "form-input",
                            style: "padding: 10px 14px; background: #121317 !important; border: 1px solid #2b2e3c !important; border-radius: 8px; color: #f1f5f9 !important; font-size: 14px; outline: none;",
                            value: "{data_dir}",
                            oninput: move |e| data_dir.set(e.value()),
                            placeholder: "./.oi",
                        }
                    }
                }

                // Action Buttons Row
                div {
                    class: "config-actions-row",
                    style: "display: flex; align-items: center; gap: 16px; margin-top: 24px; padding-top: 16px; border-top: 1px solid #22242e;",

                    // Save Button
                    button {
                        class: "btn-primary-action",
                        style: if is_form_valid {
                            "padding: 10px 22px; background: #38bdf8; color: #0f172a; border: none; border-radius: 8px; font-size: 14px; font-weight: 600; cursor: pointer; transition: all 0.15s ease;"
                        } else {
                            "padding: 10px 22px; background: #262934; color: #64748b; border: none; border-radius: 8px; font-size: 14px; font-weight: 600; cursor: not-allowed;"
                        },
                        disabled: !is_form_valid,
                        onclick: move |_| {
                            if is_form_valid {
                                let new_cfg = LlmRuntimeConfig {
                                    base_url: base_url().trim().to_string(),
                                    api_key: api_key().trim().to_string(),
                                    model: model().trim().to_string(),
                                    max_tokens: max_tokens().parse::<u32>().unwrap_or(4096),
                                    data_dir: data_dir().trim().to_string(),
                                };
                                match new_cfg.save_to_file() {
                                    Ok(()) => {
                                        save_status.set(Some(Ok("✓ 配置已成功持久化保存至 .oi/config.toml 并实时生效！".into())));
                                        on_update_config.call(new_cfg);
                                    }
                                    Err(e) => {
                                        save_status.set(Some(Err(format!("保存失败: {}", e))));
                                    }
                                }
                            }
                        },
                        "💾 保存配置并应用"
                    }

                    // Test Connection Button
                    button {
                        class: "btn-secondary-action",
                        style: "padding: 10px 20px; background: #232530; border: 1px solid #333748; color: #f1f5f9; border-radius: 8px; font-size: 14px; font-weight: 500; cursor: pointer; transition: all 0.15s ease;",
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
                        if is_testing() { "⚡ 正在连接端点..." } else { "⚡ 测试连接与拉取可用模型" }
                    }
                }
            }

            // Online Models Card
            div {
                class: "config-card",
                style: "background: #17181f; border: 1px solid #282a36; border-radius: 12px; padding: 24px; box-shadow: 0 4px 20px rgba(0,0,0,0.3);",

                div {
                    class: "config-card-header",
                    style: "display: flex; align-items: center; justify-content: space-between; margin-bottom: 18px; padding-bottom: 14px; border-bottom: 1px solid #22242e;",
                    div { style: "font-size: 16px; font-weight: 600; color: #f8fafc;", "在线可用模型列表 ({models.len()})" }
                    span { style: "font-size: 12px; color: #94a3b8;", "点击「设为当前模型」即可一键切换" }
                }

                div { style: "display: flex; flex-direction: column; gap: 8px;",
                    for m in &models {
                        {
                            let m_str = m.clone();
                            let is_current = *model.read() == *m;
                            rsx! {
                                div {
                                    key: "{m}",
                                    style: "display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; background: #121318; border: 1px solid #232532; border-radius: 8px; transition: border-color 0.15s ease;",
                                    div { style: "display: flex; align-items: center; gap: 12px;",
                                        span { style: "color: #38bdf8; font-weight: 600; font-family: ui-monospace, SFMono-Regular, monospace; font-size: 14px;", "{m}" }
                                        if is_current {
                                            span { style: "background: #143525; color: #4ade80; border: 1px solid #1e5238; padding: 2px 8px; border-radius: 4px; font-size: 11.5px; font-weight: 600;", "✓ 当前默认" }
                                        }
                                    }
                                    button {
                                        style: if is_current {
                                            "background: #1e212b; color: #64748b; border: 1px solid #282b38; padding: 5px 14px; border-radius: 6px; font-size: 12.5px; cursor: default;"
                                        } else {
                                            "background: #232530; color: #f1f5f9; border: 1px solid #333648; padding: 5px 14px; border-radius: 6px; font-size: 12.5px; cursor: pointer; transition: all 0.15s ease;"
                                        },
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
