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

    // Form validations
    let url_val = base_url();
    let url_error: Option<&'static str> = if url_val.trim().is_empty() {
        Some("Base URL 不能为空")
    } else if !url_val.starts_with("http://") && !url_val.starts_with("https://") {
        Some("URL 必须以 http:// 或 https:// 开头")
    } else if url_val.contains(' ') {
        Some("URL 不能包含空格")
    } else {
        None
    };

    let key_val = api_key();
    let key_error: Option<&'static str> = if key_val.trim().is_empty() {
        Some("API Key 不能为空")
    } else if key_val.trim().len() < 8 {
        Some("API Key 长度不足 8 字符")
    } else {
        None
    };

    let model_val = model();
    let model_error: Option<&'static str> = if model_val.trim().is_empty() {
        Some("默认模型不能为空")
    } else {
        None
    };

    let tokens_val = max_tokens();
    let tokens_error: Option<&'static str> = match tokens_val.trim().parse::<u32>() {
        Ok(v) if v < 16 => Some("Max Tokens 至少 16"),
        Ok(v) if v > 200_000 => Some("Max Tokens 超过上限 200,000"),
        Ok(_) => None,
        Err(_) => Some("必须为有效正整数"),
    };

    let dir_val = data_dir();
    let dir_error: Option<&'static str> = if dir_val.trim().is_empty() {
        Some("数据目录不能为空")
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
        div { class: "config-page",
            // Header Section
            div { class: "config-header",
                div { style: "display: flex; align-items: center; justify-content: space-between;",
                    div {
                        h1 { "模型与渠道配置" }
                        p { class: "config-subtitle", "管理 LLM 渠道端点、安全凭证与默认模型，配置实时持久化至 .oi/config.toml" }
                    }
                    div {
                        style: if is_form_valid {
                            "padding: 3px 10px; border-radius: 4px; background: #11261d; border: 1px solid #1a4231; color: var(--accent-green); font-size: 11.5px; font-weight: 500;"
                        } else {
                            "padding: 3px 10px; border-radius: 4px; background: #2b1416; border: 1px solid #4a2125; color: var(--accent-red); font-size: 11.5px; font-weight: 500;"
                        },
                        if is_form_valid { "校验通过" } else { "未通过校验" }
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
                            "连接成功：已探测到 {list.len()} 个可用模型"
                        }
                    },
                    Err(err) => rsx! {
                        div { class: "feedback-banner error",
                            "连接失败: {err}"
                        }
                    },
                }
            }

            // Form Card
            div { class: "config-card",
                div { class: "config-card-header",
                    div { class: "config-card-title", "LLM API 凭证与端点" }
                    span { style: "font-size: 11.5px; color: var(--text-muted); font-family: ui-monospace, monospace;", "OpenAI-compatible" }
                }

                div { class: "form-grid",
                    // Base URL Field
                    div { class: "form-group full",
                        div { style: "display: flex; justify-content: space-between;",
                            label { class: "form-label", "Base URL (API 基地址)" }
                            if let Some(err) = url_error {
                                span { style: "font-size: 11px; color: var(--accent-red);", "{err}" }
                            }
                        }
                        input {
                            class: "form-input",
                            value: "{base_url}",
                            oninput: move |e| base_url.set(e.value()),
                            placeholder: "http://127.0.0.1:3182",
                        }
                    }

                    // API Key Field
                    div { class: "form-group full",
                        div { style: "display: flex; justify-content: space-between; align-items: center;",
                            label { class: "form-label", "API Key / Bearer Token" }
                            button {
                                class: "action-pill-btn",
                                onclick: move |_| show_key.set(!show_key()),
                                if show_key() { "隐藏" } else { "显示" }
                            }
                        }
                        input {
                            class: "form-input",
                            style: "font-family: ui-monospace, monospace;",
                            r#type: if show_key() { "text" } else { "password" },
                            value: "{api_key}",
                            oninput: move |e| api_key.set(e.value()),
                            placeholder: "sk-...",
                        }
                        if let Some(err) = key_error {
                            span { style: "font-size: 11px; color: var(--accent-red);", "{err}" }
                        }
                    }

                    // Default Model
                    div { class: "form-group",
                        div { style: "display: flex; justify-content: space-between;",
                            label { class: "form-label", "默认模型 (Model ID)" }
                            if let Some(err) = model_error {
                                span { style: "font-size: 11px; color: var(--accent-red);", "{err}" }
                            }
                        }
                        input {
                            class: "form-input",
                            value: "{model}",
                            oninput: move |e| model.set(e.value()),
                            placeholder: "agnes-2.5-flash",
                        }
                    }

                    // Max Tokens
                    div { class: "form-group",
                        div { style: "display: flex; justify-content: space-between;",
                            label { class: "form-label", "Max Tokens" }
                            if let Some(err) = tokens_error {
                                span { style: "font-size: 11px; color: var(--accent-red);", "{err}" }
                            }
                        }
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
                        div { style: "display: flex; justify-content: space-between;",
                            label { class: "form-label", "数据目录 (Data Directory)" }
                            if let Some(err) = dir_error {
                                span { style: "font-size: 11px; color: var(--accent-red);", "{err}" }
                            }
                        }
                        input {
                            class: "form-input",
                            value: "{data_dir}",
                            oninput: move |e| data_dir.set(e.value()),
                            placeholder: "./.oi",
                        }
                    }
                }

                // Action Buttons Row
                div { class: "config-actions-row",
                    button {
                        class: "btn-primary-action",
                        style: if !is_form_valid { "opacity: 0.5; cursor: not-allowed;" } else { "" },
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
                                        save_status.set(Some(Ok("配置已写入 .oi/config.toml 并生效".into())));
                                        on_update_config.call(new_cfg);
                                    }
                                    Err(e) => {
                                        save_status.set(Some(Err(format!("保存失败: {}", e))));
                                    }
                                }
                            }
                        },
                        "保存配置"
                    }

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
                        if is_testing() { "正在测试..." } else { "测试连接" }
                    }
                }
            }

            // Online Models Card
            div { class: "config-card",
                div { class: "config-card-header",
                    div { class: "config-card-title", "在线可用模型 ({models.len()})" }
                    span { style: "font-size: 11.5px; color: var(--text-muted);", "点击选用" }
                }

                div { style: "display: flex; flex-direction: column; gap: 6px;",
                    for m in &models {
                        {
                            let m_str = m.clone();
                            let is_current = *model.read() == *m;
                            rsx! {
                                div {
                                    key: "{m}",
                                    style: "display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; background: var(--bg-base); border: 1px solid var(--border-subtle); border-radius: 5px;",
                                    div { style: "display: flex; align-items: center; gap: 8px;",
                                        span { style: "color: var(--accent-blue); font-family: ui-monospace, monospace; font-size: 13px;", "{m}" }
                                        if is_current {
                                            span { style: "background: #11261d; color: var(--accent-green); border: 1px solid #1a4231; padding: 1px 5px; border-radius: 3px; font-size: 10.5px;", "默认" }
                                        }
                                    }
                                    button {
                                        class: "action-pill-btn",
                                        disabled: is_current,
                                        onclick: move |_| {
                                            model.set(m_str.clone());
                                        },
                                        if is_current { "已选用" } else { "选用" }
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
