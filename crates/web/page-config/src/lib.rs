//! 设置弹窗（dsh SettingsRoot 复刻）：800px r24 面板 + 188px 内导航。
//! 「模型与渠道」承载 LLM 配置表单；「关于」放版本与项目信息。

use dioxus::prelude::*;
use omenic_web_client::llm::LlmRuntimeConfig;
use omenic_web_components::icons::{Gear, X};
use omenic_web_components::ui::{Button, ButtonSize, ButtonVariant, IconButton, Modal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Models,
    About,
}

#[component]
pub fn SettingsModal(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
    on_close: EventHandler<()>,
) -> Element {
    let mut section = use_signal(|| Section::Models);

    let nav_cell = |active: bool| {
        if active {
            "h-10 px-3 rounded-xl flex items-center gap-2.5 text-[14px] leading-[22px] bg-[#434546] text-label cursor-pointer transition-colors border-none w-full"
        } else {
            "h-10 px-3 rounded-xl flex items-center gap-2.5 text-[14px] leading-[22px] text-label-2 hover:bg-ihover hover:text-label cursor-pointer transition-colors border-none w-full bg-transparent"
        }
    };

    rsx! {
        Modal { width_class: "w-[800px]", on_close: on_close,
            div { class: "flex h-[min(760px,80vh)]",
                // 左导航栏（188px）
                nav { class: "w-[188px] shrink-0 bg-sidebar p-3 flex flex-col gap-1",
                    button {
                        r#type: "button",
                        class: "{nav_cell(section() == Section::Models)}",
                        onclick: move |_| section.set(Section::Models),
                        Gear { size: 16, class: "text-label-3" }
                        span { "模型与渠道" }
                    }
                    button {
                        r#type: "button",
                        class: "{nav_cell(section() == Section::About)}",
                        onclick: move |_| section.set(Section::About),
                        span { class: "w-4 text-center text-[14px] text-label-3", "i" }
                        span { "关于" }
                    }
                }
                // 右内容列
                div { class: "flex-1 min-w-0 flex flex-col",
                    div { class: "h-[54px] px-6 flex items-center justify-between border-b border-b1 shrink-0",
                        span { class: "text-[14px] leading-5 font-medium text-label",
                            if section() == Section::Models { "模型与渠道" } else { "关于 omenic" }
                        }
                        IconButton { title: "关闭", onclick: move |_| on_close.call(()), X { size: 16 } }
                    }
                    div { class: "flex-1 overflow-y-auto",
                        if section() == Section::Models {
                            ConfigForm { config: config.clone(), on_update_config: on_update_config }
                        } else {
                            AboutPane {}
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ConfigForm(
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
            "deepseek-v4-flash".to_string(),
            "qwen3-32b".to_string(),
            "agnes-2.5-flash".to_string(),
        ]
    });

    let input_class = "w-full h-9 rounded-[10px] bg-layer-2 border border-b2 px-3 text-[14px] leading-[22px] text-label outline-none transition-colors focus:border-brand placeholder:text-caption";
    let label_class = "text-[13px] leading-5 font-medium text-label-2";
    let err_class = "text-[12px] leading-4 text-danger";

    rsx! {
        div { class: "px-6 py-6 flex flex-col gap-4",
            // 保存反馈
            if let Some(res) = save_status.read().as_ref() {
                match res {
                    Ok(msg) => rsx! {
                        div { class: "px-4 py-2.5 rounded-[10px] text-[13px] leading-5 bg-chip-success text-success-2", "{msg}" }
                    },
                    Err(err) => rsx! {
                        div { class: "px-4 py-2.5 rounded-[10px] text-[13px] leading-5 bg-chip-danger text-danger", "{err}" }
                    },
                }
            }
            // 连接测试反馈
            if let Some(res) = test_status.read().as_ref() {
                match res {
                    Ok(list) => rsx! {
                        div { class: "px-4 py-2.5 rounded-[10px] text-[13px] leading-5 bg-chip-success text-success-2",
                            "连接成功：已探测到 {list.len()} 个可用模型"
                        }
                    },
                    Err(err) => rsx! {
                        div { class: "px-4 py-2.5 rounded-[10px] text-[13px] leading-5 bg-chip-danger text-danger",
                            "连接失败: {err}"
                        }
                    },
                }
            }

            // 凭证与端点卡
            div { class: "bg-layer-1 border border-b1 rounded-2xl px-6 py-5 flex flex-col gap-4",
                div { class: "flex items-center justify-between pb-3 border-b border-b1",
                    div { class: "text-[15px] leading-[22px] font-medium text-label", "LLM API 凭证与端点" }
                    span { class: "text-[11px] leading-4 text-caption font-mono", "OpenAI-compatible" }
                }

                div { class: "grid grid-cols-2 gap-4 gap-x-5",
                    div { class: "flex flex-col gap-1.5 col-span-2",
                        div { class: "flex justify-between",
                            label { class: "{label_class}", "Base URL (API 基地址)" }
                            if let Some(err) = url_error {
                                span { class: "{err_class}", "{err}" }
                            }
                        }
                        input {
                            class: "{input_class}",
                            value: "{base_url}",
                            oninput: move |e| base_url.set(e.value()),
                            placeholder: "http://127.0.0.1:3182",
                        }
                    }

                    div { class: "flex flex-col gap-1.5 col-span-2",
                        div { class: "flex justify-between items-center",
                            label { class: "{label_class}", "API Key / Bearer Token" }
                            Button {
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::Sm,
                                onclick: move |_| show_key.set(!show_key()),
                                if show_key() { "隐藏" } else { "显示" }
                            }
                        }
                        input {
                            class: "{input_class} font-mono",
                            r#type: if show_key() { "text" } else { "password" },
                            value: "{api_key}",
                            oninput: move |e| api_key.set(e.value()),
                            placeholder: "sk-...",
                        }
                        if let Some(err) = key_error {
                            span { class: "{err_class}", "{err}" }
                        }
                    }

                    div { class: "flex flex-col gap-1.5",
                        div { class: "flex justify-between",
                            label { class: "{label_class}", "默认模型 (Model ID)" }
                            if let Some(err) = model_error {
                                span { class: "{err_class}", "{err}" }
                            }
                        }
                        input {
                            class: "{input_class}",
                            value: "{model}",
                            oninput: move |e| model.set(e.value()),
                            placeholder: "deepseek-v4-flash",
                        }
                    }

                    div { class: "flex flex-col gap-1.5",
                        div { class: "flex justify-between",
                            label { class: "{label_class}", "Max Tokens" }
                            if let Some(err) = tokens_error {
                                span { class: "{err_class}", "{err}" }
                            }
                        }
                        input {
                            class: "{input_class}",
                            r#type: "number",
                            value: "{max_tokens}",
                            oninput: move |e| max_tokens.set(e.value()),
                            placeholder: "4096",
                        }
                    }

                    div { class: "flex flex-col gap-1.5 col-span-2",
                        div { class: "flex justify-between",
                            label { class: "{label_class}", "数据目录 (Data Directory)" }
                            if let Some(err) = dir_error {
                                span { class: "{err_class}", "{err}" }
                            }
                        }
                        input {
                            class: "{input_class} font-mono",
                            value: "{data_dir}",
                            oninput: move |e| data_dir.set(e.value()),
                            placeholder: "./.oi",
                        }
                    }
                }

                div { class: "flex items-center gap-2.5 pt-1",
                    Button {
                        variant: ButtonVariant::Primary,
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

                    Button {
                        variant: ButtonVariant::Outline,
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

            // 在线模型卡
            div { class: "bg-layer-1 border border-b1 rounded-2xl px-6 py-5 flex flex-col gap-4",
                div { class: "flex items-center justify-between pb-3 border-b border-b1",
                    div { class: "text-[15px] leading-[22px] font-medium text-label", "在线可用模型 ({models.len()})" }
                    span { class: "text-[11px] leading-4 text-caption", "点击选用" }
                }

                div { class: "grid grid-cols-2 gap-2.5",
                    for m in &models {
                        {
                            let m_str = m.clone();
                            let is_current = *model.read() == *m;
                            let card_class = if is_current {
                                "p-3 rounded-xl border border-brand bg-layer-2 cursor-pointer transition-colors"
                            } else {
                                "p-3 rounded-xl border border-b1 bg-layer-2 cursor-pointer transition-colors hover:border-b2"
                            };
                            rsx! {
                                div { key: "{m}", class: "{card_class}",
                                    div { class: "flex items-center gap-2",
                                        span { class: "text-brand-300 font-mono text-[13px] font-medium", "{m}" }
                                        if is_current {
                                            span { class: "bg-chip-success text-success-2 px-1.5 py-px rounded-md text-[11px] font-semibold", "默认" }
                                        }
                                    }
                                    div { class: "mt-2",
                                        Button {
                                            variant: ButtonVariant::Ghost,
                                            size: ButtonSize::Sm,
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
}

#[component]
fn AboutPane() -> Element {
    let version = env!("CARGO_PKG_VERSION");
    rsx! {
        div { class: "px-6 py-6 flex flex-col gap-4 max-w-[520px]",
            div { class: "flex items-center gap-2",
                span { class: "text-[18px] leading-6 font-semibold tracking-[0.04em] text-label", "omenic" }
                span { class: "text-[12px] leading-5 text-label-3 font-mono px-2 py-0.5 rounded-full bg-layer-1 border border-b1", "v{version}" }
            }
            p { class: "text-[14px] leading-[22px] text-label-2",
                "agent harness 的 Rust 复刻（参考 DeepSeek Harness）：agent 循环、会话持久化、事件流与插件面。web UI 为 Dioxus LiveView，设计语言复刻 dsh web。"
            }
            div { class: "flex flex-col gap-1.5 text-[13px] leading-5 text-label-3",
                span { "路线与进度见仓库根 ROADMAP.md（C1–C8 / R1–R5 / G1–G5）。" }
                span { "当前 web 数据为 mock 数据源，G4 事件流定稿后切换 daemon 实时数据。" }
            }
        }
    }
}
