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
        div { class: "flex-1 overflow-y-auto flex flex-col gap-6 px-12 pt-9 pb-14 max-w-[1000px] w-full mx-auto",
            // Header Section
            div { class: "flex items-center justify-between pb-5 border-b border-subtle",
                div { class: "flex items-center justify-between w-full",
                    div {
                        h1 { "模型与渠道配置" }
                        p { class: "text-[13px] text-muted mt-1 m-0", "管理 LLM 渠道端点、安全凭证与默认模型，配置实时持久化至 .oi/config.toml" }
                    }
                    div {
                        class: if is_form_valid {
                            "px-2.5 py-0.5 rounded text-[11.5px] font-medium bg-[#11261d] border border-[#1a4231] text-success"
                        } else {
                            "px-2.5 py-0.5 rounded text-[11.5px] font-medium bg-[#2b1416] border border-[#4a2125] text-danger"
                        },
                        if is_form_valid { "校验通过" } else { "未通过校验" }
                    }
                }
            }

            // Save feedback banner
            if let Some(res) = save_status.read().as_ref() {
                match res {
                    Ok(msg) => rsx! {
                        div { class: "px-4 py-2.5 rounded-lg text-[13px] flex items-center gap-2 bg-emerald-500/10 text-emerald-300 border border-emerald-500/25", "{msg}" }
                    },
                    Err(err) => rsx! {
                        div { class: "px-4 py-2.5 rounded-lg text-[13px] flex items-center gap-2 bg-red-500/10 text-red-300 border border-red-500/25", "{err}" }
                    },
                }
            }

            // Test connection feedback banner
            if let Some(res) = test_status.read().as_ref() {
                match res {
                    Ok(list) => rsx! {
                        div { class: "px-4 py-2.5 rounded-lg text-[13px] flex items-center gap-2 bg-emerald-500/10 text-emerald-300 border border-emerald-500/25",
                            "连接成功：已探测到 {list.len()} 个可用模型"
                        }
                    },
                    Err(err) => rsx! {
                        div { class: "px-4 py-2.5 rounded-lg text-[13px] flex items-center gap-2 bg-red-500/10 text-red-300 border border-red-500/25",
                            "连接失败: {err}"
                        }
                    },
                }
            }

            // Form Card
            div { class: "bg-surface border border-subtle rounded-xl px-7 py-6 flex flex-col gap-5 shadow-[0_4px_20px_rgba(0,0,0,0.2)]",
                div { class: "flex items-center justify-between pb-3.5 border-b border-[rgba(255,255,255,0.05)]",
                    div { class: "text-[15px] font-semibold text-white", "LLM API 凭证与端点" }
                    span { class: "text-[11.5px] text-muted font-mono", "OpenAI-compatible" }
                }

                div { class: "grid grid-cols-2 gap-4.5 gap-x-5",
                    // Base URL Field
                    div { class: "flex flex-col gap-1.5 col-span-2",
                        div { class: "flex justify-between",
                            label { class: "text-xs font-medium text-[#c9cddb]", "Base URL (API 基地址)" }
                            if let Some(err) = url_error {
                                span { class: "text-[11px] text-danger", "{err}" }
                            }
                        }
                        input {
                            class: "w-full px-3.5 py-2.5 bg-[#0f1017] border border-subtle rounded-md text-white text-[13px] outline-none transition-all focus:border-accent focus:shadow-[0_0_0_3px_rgba(162,138,199,0.2)] placeholder:text-muted",
                            value: "{base_url}",
                            oninput: move |e| base_url.set(e.value()),
                            placeholder: "http://127.0.0.1:3182",
                        }
                    }

                    // API Key Field
                    div { class: "flex flex-col gap-1.5 col-span-2",
                        div { class: "flex justify-between items-center",
                            label { class: "text-xs font-medium text-[#c9cddb]", "API Key / Bearer Token" }
                            button {
                                class: "px-3 py-1 bg-[rgba(255,255,255,0.06)] border border-subtle rounded text-secondary text-[11px] cursor-pointer hover:text-white hover:bg-[rgba(255,255,255,0.12)] transition-colors",
                                onclick: move |_| show_key.set(!show_key()),
                                if show_key() { "隐藏" } else { "显示" }
                            }
                        }
                        input {
                            class: "w-full px-3.5 py-2.5 bg-[#0f1017] border border-subtle rounded-md text-white text-[13px] font-mono outline-none transition-all focus:border-accent focus:shadow-[0_0_0_3px_rgba(162,138,199,0.2)] placeholder:text-muted",
                            r#type: if show_key() { "text" } else { "password" },
                            value: "{api_key}",
                            oninput: move |e| api_key.set(e.value()),
                            placeholder: "sk-...",
                        }
                        if let Some(err) = key_error {
                            span { class: "text-[11px] text-danger", "{err}" }
                        }
                    }

                    // Default Model
                    div { class: "flex flex-col gap-1.5",
                        div { class: "flex justify-between",
                            label { class: "text-xs font-medium text-[#c9cddb]", "默认模型 (Model ID)" }
                            if let Some(err) = model_error {
                                span { class: "text-[11px] text-danger", "{err}" }
                            }
                        }
                        input {
                            class: "w-full px-3.5 py-2.5 bg-[#0f1017] border border-subtle rounded-md text-white text-[13px] outline-none transition-all focus:border-accent focus:shadow-[0_0_0_3px_rgba(162,138,199,0.2)] placeholder:text-muted",
                            value: "{model}",
                            oninput: move |e| model.set(e.value()),
                            placeholder: "agnes-2.5-flash",
                        }
                    }

                    // Max Tokens
                    div { class: "flex flex-col gap-1.5",
                        div { class: "flex justify-between",
                            label { class: "text-xs font-medium text-[#c9cddb]", "Max Tokens" }
                            if let Some(err) = tokens_error {
                                span { class: "text-[11px] text-danger", "{err}" }
                            }
                        }
                        input {
                            class: "w-full px-3.5 py-2.5 bg-[#0f1017] border border-subtle rounded-md text-white text-[13px] outline-none transition-all focus:border-accent focus:shadow-[0_0_0_3px_rgba(162,138,199,0.2)] placeholder:text-muted",
                            r#type: "number",
                            value: "{max_tokens}",
                            oninput: move |e| max_tokens.set(e.value()),
                            placeholder: "4096",
                        }
                    }

                    // Data Directory
                    div { class: "flex flex-col gap-1.5 col-span-2",
                        div { class: "flex justify-between",
                            label { class: "text-xs font-medium text-[#c9cddb]", "数据目录 (Data Directory)" }
                            if let Some(err) = dir_error {
                                span { class: "text-[11px] text-danger", "{err}" }
                            }
                        }
                        input {
                            class: "w-full px-3.5 py-2.5 bg-[#0f1017] border border-subtle rounded-md text-white text-[13px] outline-none transition-all focus:border-accent focus:shadow-[0_0_0_3px_rgba(162,138,199,0.2)] placeholder:text-muted",
                            value: "{data_dir}",
                            oninput: move |e| data_dir.set(e.value()),
                            placeholder: "./.oi",
                        }
                    }
                }

                // Action Buttons Row
                div { class: "flex items-center gap-3 pt-1.5",
                    button {
                        class: if !is_form_valid { "px-5 py-2 bg-accent text-[#0b0c10] text-[13px] font-semibold rounded-md transition-colors opacity-50 cursor-not-allowed" } else { "px-5 py-2 bg-accent text-[#0b0c10] text-[13px] font-semibold rounded-md hover:bg-accent-hover transition-colors cursor-pointer" },
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
                        class: "px-5 py-2 bg-[rgba(255,255,255,0.05)] text-primary text-[13px] font-medium rounded-md border border-subtle hover:bg-[rgba(255,255,255,0.1)] transition-colors cursor-pointer",
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
            div { class: "bg-surface border border-subtle rounded-xl px-7 py-6 flex flex-col gap-5 shadow-[0_4px_20px_rgba(0,0,0,0.2)]",
                div { class: "flex items-center justify-between pb-3.5 border-b border-[rgba(255,255,255,0.05)]",
                    div { class: "text-[15px] font-semibold text-white", "在线可用模型 ({models.len()})" }
                    span { class: "text-[11.5px] text-muted", "点击选用" }
                }

                div { class: "grid grid-cols-2 gap-2.5",
                    for m in &models {
                        {
                            let m_str = m.clone();
                            let is_current = *model.read() == *m;
                            let card_class = if is_current { "p-3 rounded-lg border border-accent bg-surface-elevated cursor-pointer transition-colors" } else { "p-3 rounded-lg border border-subtle bg-base cursor-pointer transition-colors hover:border-hover" };
                            rsx! {
                                div {
                                    key: "{m}",
                                    class: "{card_class}",
                                    div { class: "flex items-center gap-2",
                                        span { class: "text-[#c4b5fd] font-mono text-[13px] font-medium", "{m}" }
                                        if is_current {
                                            span { class: "bg-emerald-500/10 text-success border border-emerald-500/30 px-1.5 py-0.5 rounded text-[11px] font-semibold", "默认" }
                                        }
                                    }
                                    button {
                                        class: "px-3 py-1 bg-[rgba(255,255,255,0.06)] border border-subtle rounded text-secondary text-[11px] cursor-pointer hover:text-white hover:bg-[rgba(255,255,255,0.12)] transition-colors",
                                        disabled: is_current,
                                        onclick: move |_| {
                                            model.set(m_str.clone());
                                        },
                                        if is_current { "当前默认" } else { "选用" }
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
