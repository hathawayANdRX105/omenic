//! 设置弹窗（dsh SettingsRoot 复刻）：800px r24 面板 + 188px 内导航。
//! 「模型与渠道」承载 LLM 配置表单；「MCP 服务器」承载 [[mcp.servers]]
//! 的列表编辑；「关于」放版本与项目信息。

use crate::components::icons::{Gear, Terminal, Trash, X};
use crate::components::ui::{Button, ButtonSize, ButtonVariant, IconButton, Modal};
use dioxus::prelude::*;
use web_client::llm::{LlmFallbackForm, LlmRuntimeConfig, McpServerForm};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Models,
    Mcp,
    About,
}

/// Max Tokens 输入校验：空白修剪后须为 16–200,000 的 u32，合法返回 `None`。
pub fn validate_max_tokens(value: &str) -> Option<&'static str> {
    match value.trim().parse::<u32>() {
        Ok(v) if v < 16 => Some("Max Tokens 至少 16"),
        Ok(v) if v > 200_000 => Some("Max Tokens 超过上限 200,000"),
        Ok(_) => None,
        Err(_) => Some("必须为有效正整数"),
    }
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

    let title = match section() {
        Section::Models => "模型与渠道",
        Section::Mcp => "MCP 服务器",
        Section::About => "关于 omenic",
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
                        class: "{nav_cell(section() == Section::Mcp)}",
                        onclick: move |_| section.set(Section::Mcp),
                        Terminal { size: 16, class: "text-label-3" }
                        span { "MCP 服务器" }
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
                        span { class: "text-[14px] leading-5 font-medium text-label", "{title}" }
                        IconButton { title: "关闭", onclick: move |_| on_close.call(()), X { size: 16 } }
                    }
                    div { class: "flex-1 overflow-y-auto",
                        if section() == Section::Models {
                            ConfigForm { config: config.clone(), on_update_config: on_update_config }
                        } else if section() == Section::Mcp {
                            McpServersPane { config: config.clone(), on_update_config: on_update_config }
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

    // Fallback Provider（[[llm.fallbacks]]）表单状态与校验
    let mut fallbacks = use_signal(|| config.llm_fallbacks.clone());
    let fallback_list = fallbacks();
    let mut fallback_empty_model = false;
    let mut fallback_dup_model = false;
    let mut fallback_bad_tokens = false;
    let mut seen_fallback_models = std::collections::HashSet::new();
    for fallback in &fallback_list {
        let model = fallback.model.trim();
        if model.is_empty() {
            fallback_empty_model = true;
        } else if !seen_fallback_models.insert(model.to_string()) {
            fallback_dup_model = true;
        }
        let tokens = fallback.max_tokens.trim();
        if !tokens.is_empty() && tokens.parse::<u32>().is_err() {
            fallback_bad_tokens = true;
        }
    }
    let fallback_hint: Option<&'static str> = if fallback_empty_model {
        Some("每个 Fallback Provider 必须填写 model：保存按 model 匹配既有行")
    } else if fallback_dup_model {
        Some("存在重复的 fallback model：保存时会更新到同一行，请改名区分")
    } else if fallback_bad_tokens {
        Some("fallback max_tokens 必须为空或正整数")
    } else {
        None
    };

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
    let tokens_error = validate_max_tokens(&tokens_val);

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
        && dir_error.is_none()
        && fallback_hint.is_none();

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

    // A stale success banner above a now-invalid form (user kept editing
    // after saving) misleads — hide it whenever validation fails. Pure
    // render-side condition: no signal writes during render. Same treatment
    // as McpServersPane.
    let save_banner_visible = !matches!(save_status.read().as_ref(), Some(Ok(_))) || is_form_valid;

    rsx! {
        div { class: "px-6 py-6 flex flex-col gap-4",
            // 保存反馈
            if save_banner_visible {
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
                                    // LLM 保存沿用当前已保存的 MCP 表单状态：
                                    // 空表单时 [mcp] 完全不被触碰。
                                    mcp_servers: config.mcp_servers.clone(),
                                    // Fallback 表单当前状态随本次保存写出
                                    // [[llm.fallbacks]]（空表单不碰该段）。
                                    llm_fallbacks: fallbacks(),
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
                                // 探针只读 base_url/api_key，不落盘：空 vec
                                // 让它即使被误存也不会碰 [mcp]。
                                mcp_servers: Vec::new(),
                                llm_fallbacks: Vec::new(),
                            };
                            let res = probe_cfg.test_connection();
                            test_status.set(Some(res));
                            is_testing.set(false);
                        },
                        if is_testing() { "正在测试..." } else { "测试连接" }
                    }
                }
            }

            // Fallback Provider 卡（[[llm.fallbacks]]，主 provider 失败后按序切换）
            div { class: "bg-layer-1 border border-b1 rounded-2xl px-6 py-5 flex flex-col gap-4",
                div { class: "flex items-center justify-between pb-3 border-b border-b1",
                    div { class: "text-[15px] leading-[22px] font-medium text-label", "Fallback Provider（主 provider 失败后按序切换）" }
                    Button {
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Sm,
                        onclick: move |_| fallbacks.write().push(LlmFallbackForm::default()),
                        "添加 Fallback Provider"
                    }
                }

                if fallback_list.is_empty() {
                    div { class: "bg-layer-1 border border-b1 rounded-xl px-4 py-6 flex flex-col items-center gap-1.5",
                        span { class: "text-[13px] leading-5 text-label-3", "尚未配置 Fallback Provider" }
                        span { class: "text-[12px] leading-4 text-caption", "主 provider 无可用内容时按列表顺序切换；base_url / api_key 留空 = 继承主 provider" }
                    }
                }
                for (i, fallback) in fallback_list.iter().enumerate() {
                    // `{i}-{model}` key: deleting a middle row must not make
                    // the rows above inherit the deleted row's component
                    // state (`show_key`), while the `i` prefix keeps keys
                    // unique while a duplicate model is mid-edit.
                    LlmFallbackCard { key: "{i}-{fallback.model}", index: i, fallback: fallback.clone(), fallbacks }
                }
                if let Some(hint) = fallback_hint {
                    span { class: "{err_class}", "{hint}" }
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

/// 「MCP 服务器」页：`[[mcp.servers]]` 的卡片列表编辑。表单状态是
/// `Vec<McpServerForm>`（args 以逗号分隔文本承载），保存走
/// `save_to_file` 的增量路径：按 `name` 匹配更新管理键，`env` /
/// `reconnect` 等高级键与表单外的服务器原样保留。
#[component]
fn McpServersPane(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    let mut servers = use_signal(|| config.mcp_servers.clone());
    let mut save_status = use_signal(|| None::<Result<String, String>>);

    let list = servers();

    // 保存前置校验：name 非空且不重复、command/url 至少其一、
    // 超时毫秒数留空或为正整数。失败只禁用保存钮并提示，不改输入。
    let mut seen_names = std::collections::HashSet::new();
    let mut empty_name = false;
    let mut dup_name = false;
    let mut no_transport = false;
    let mut bad_timeout = false;
    for server in &list {
        let name = server.name.trim();
        if name.is_empty() {
            empty_name = true;
        } else if !seen_names.insert(name.to_string()) {
            dup_name = true;
        }
        if server.command.trim().is_empty() && server.url.trim().is_empty() {
            no_transport = true;
        }
        let timeout = server.tool_call_timeout_ms.trim();
        if !timeout.is_empty() && timeout.parse::<u64>().is_err() {
            bad_timeout = true;
        }
    }
    let invalid_hint: Option<&'static str> = if empty_name {
        Some("存在未命名的服务器：保存按 name 匹配，请先填写 name")
    } else if dup_name {
        Some("存在重复的 name：保存时会更新到同一台服务器，请改名区分")
    } else if no_transport {
        Some("每台服务器至少填写 command 或 url 其一")
    } else if bad_timeout {
        Some("tool_call_timeout_ms 必须为空或正整数毫秒")
    } else {
        None
    };
    let is_form_valid = invalid_hint.is_none();

    let err_class = "text-[12px] leading-4 text-danger";

    // A stale success banner above a now-invalid form (user kept editing
    // after saving) misleads — hide it whenever validation fails. Pure
    // render-side condition: no signal writes during render.
    let save_banner_visible = !matches!(save_status.read().as_ref(), Some(Ok(_))) || is_form_valid;

    rsx! {
        div { class: "px-6 py-6 flex flex-col gap-4",
            // 保存反馈
            if save_banner_visible {
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
            }

            // 页头：说明 + 添加按钮
            div { class: "flex items-center justify-between gap-3",
                div { class: "min-w-0 flex flex-col gap-0.5",
                    div { class: "text-[15px] leading-[22px] font-medium text-label", "MCP 服务器" }
                    span { class: "text-[12px] leading-4 text-caption",
                        "stdio 子进程（command）或 HTTP 端点（url）；保存按 name 更新。env / reconnect 等高级键请直接编辑 .oi/config.toml，不会被覆盖；删除仅移除编辑卡，不从配置文件删服务器。"
                    }
                }
                Button {
                    variant: ButtonVariant::Outline,
                    onclick: move |_| servers.write().push(McpServerForm::default()),
                    "添加 MCP 服务器"
                }
            }

            // 服务器卡列表
            if list.is_empty() {
                div { class: "bg-layer-1 border border-b1 rounded-2xl px-6 py-8 flex flex-col items-center gap-1.5",
                    span { class: "text-[13px] leading-5 text-label-3", "尚未配置 MCP 服务器" }
                    span { class: "text-[12px] leading-4 text-caption", "MCP 默认关闭：不添加服务器时 agent 不会启动任何外部工具进程" }
                }
            }
            for (i, server) in list.iter().enumerate() {
                // `{i}-{name}` key: same rationale as the fallback cards —
                // stable across middle-row deletes, unique while a duplicate
                // name is mid-edit.
                McpServerCard { key: "{i}-{server.name}", index: i, server: server.clone(), servers }
            }

            // 保存行
            div { class: "flex items-center gap-2.5",
                Button {
                    variant: ButtonVariant::Primary,
                    disabled: !is_form_valid,
                    onclick: move |_| {
                        if is_form_valid {
                            let new_cfg = LlmRuntimeConfig {
                                base_url: config.base_url.clone(),
                                api_key: config.api_key.clone(),
                                model: config.model.clone(),
                                max_tokens: config.max_tokens,
                                data_dir: config.data_dir.clone(),
                                mcp_servers: servers(),
                                // MCP 保存不动 LLM 区：沿用当前已保存的
                                // fallbacks（空表单时 [[llm.fallbacks]] 不被触碰）。
                                llm_fallbacks: config.llm_fallbacks.clone(),
                            };
                            match new_cfg.save_to_file() {
                                Ok(()) => {
                                    save_status.set(Some(Ok("MCP 配置已写入 .oi/config.toml".into())));
                                    on_update_config.call(new_cfg);
                                }
                                Err(e) => {
                                    save_status.set(Some(Err(format!("保存失败: {}", e))));
                                }
                            }
                        }
                    },
                    "保存 MCP 配置"
                }
                if let Some(hint) = invalid_hint {
                    span { class: "{err_class}", "{hint}" }
                }
            }
        }
    }
}

/// 单台 MCP 服务器的编辑卡：卡头是 name + 启动方式摘要 + 删除钮，
/// 下方两列网格排布可编辑字段。所有输入直接写回 `servers[index]`。
#[component]
fn McpServerCard(
    server: McpServerForm,
    index: usize,
    mut servers: Signal<Vec<McpServerForm>>,
) -> Element {
    let input_class = "w-full h-9 rounded-[10px] bg-layer-2 border border-b2 px-3 text-[14px] leading-[22px] text-label outline-none transition-colors focus:border-brand placeholder:text-caption";
    let label_class = "text-[13px] leading-5 font-medium text-label-2";

    let transport = if !server.command.trim().is_empty() {
        format!("stdio · {}", server.command.trim())
    } else if !server.url.trim().is_empty() {
        format!("http · {}", server.url.trim())
    } else {
        "未配置启动方式".to_string()
    };
    let card_class = "bg-layer-1 border border-b1 rounded-xl px-4 py-4 flex flex-col gap-3";

    rsx! {
        div { class: "{card_class}",
            // 卡头：name + 启动方式摘要 + 删除
            div { class: "flex items-center justify-between gap-3",
                div { class: "min-w-0 flex flex-col gap-0.5",
                    span { class: "text-[14px] leading-5 font-medium text-label truncate",
                        if server.name.trim().is_empty() { "（未命名服务器）" } else { "{server.name}" }
                    }
                    span { class: "text-[12px] leading-4 text-caption font-mono truncate", "{transport}" }
                }
                IconButton {
                    title: "删除",
                    onclick: move |_| {
                        servers.write().remove(index);
                    },
                    Trash { size: 14 }
                }
            }

            div { class: "grid grid-cols-2 gap-3",
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "name" }
                    input {
                        class: "{input_class}",
                        value: "{server.name}",
                        oninput: move |e| servers.write()[index].name = e.value(),
                        placeholder: "fs",
                    }
                }
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "command" }
                    input {
                        class: "{input_class} font-mono",
                        value: "{server.command}",
                        oninput: move |e| servers.write()[index].command = e.value(),
                        placeholder: "npx",
                    }
                }
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "url" }
                    input {
                        class: "{input_class} font-mono",
                        value: "{server.url}",
                        oninput: move |e| servers.write()[index].url = e.value(),
                        placeholder: "http://127.0.0.1:9100/mcp",
                    }
                }
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "cwd" }
                    input {
                        class: "{input_class} font-mono",
                        value: "{server.cwd}",
                        oninput: move |e| servers.write()[index].cwd = e.value(),
                        placeholder: "默认继承进程工作目录",
                    }
                }
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "tool_call_timeout_ms" }
                    input {
                        class: "{input_class}",
                        r#type: "number",
                        value: "{server.tool_call_timeout_ms}",
                        oninput: move |e| servers.write()[index].tool_call_timeout_ms = e.value(),
                        placeholder: "默认",
                    }
                }
                // bool 字段沿用本页既有习惯：Ghost 小按钮切换（同 show_key）
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "fail_on_startup_error" }
                    div { class: "flex items-center h-9",
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Sm,
                            onclick: move |_| {
                                let next = !servers.read()[index].fail_on_startup_error;
                                servers.write()[index].fail_on_startup_error = next;
                            },
                            if server.fail_on_startup_error { "开启" } else { "关闭" }
                        }
                    }
                }
                div { class: "flex flex-col gap-1.5 col-span-2",
                    label { class: "{label_class}", "args (逗号分隔)" }
                    input {
                        class: "{input_class} font-mono",
                        value: "{server.args}",
                        oninput: move |e| servers.write()[index].args = e.value(),
                        placeholder: "-y, @modelcontextprotocol/server-filesystem",
                    }
                }
            }
        }
    }
}

/// 单个 Fallback Provider 的编辑卡：卡头是 model（mono）+ 端点摘要 + 删除钮，
/// 卡体两列网格排 base_url / api_key / max_tokens。所有输入直接写回
/// `fallbacks[index]`。api_key 是 password 输入框，同卡内一个 Ghost 钮切换显隐。
#[component]
fn LlmFallbackCard(
    fallback: LlmFallbackForm,
    index: usize,
    mut fallbacks: Signal<Vec<LlmFallbackForm>>,
) -> Element {
    let input_class = "w-full h-9 rounded-[10px] bg-layer-2 border border-b2 px-3 text-[14px] leading-[22px] text-label outline-none transition-colors focus:border-brand placeholder:text-caption";
    let label_class = "text-[13px] leading-5 font-medium text-label-2";

    let mut show_key = use_signal(|| false);

    let summary = if fallback.base_url.trim().is_empty() {
        "继承主 provider 端点".to_string()
    } else {
        let key_state = if fallback.api_key.trim().is_empty() {
            "无 key（继承主 provider）"
        } else {
            "已设 key"
        };
        format!("{} · {}", fallback.base_url.trim(), key_state)
    };
    let card_class = "bg-layer-1 border border-b1 rounded-xl px-4 py-4 flex flex-col gap-3";

    rsx! {
        div { class: "{card_class}",
            // 卡头：model + 端点摘要 + 删除
            div { class: "flex items-center justify-between gap-3",
                div { class: "min-w-0 flex flex-col gap-0.5",
                    span { class: "text-[14px] leading-5 font-medium text-label truncate",
                        if fallback.model.trim().is_empty() { "（未填写 model）" } else { "{fallback.model}" }
                    }
                    span { class: "text-[12px] leading-4 text-caption font-mono truncate", "{summary}" }
                }
                IconButton {
                    title: "删除",
                    onclick: move |_| {
                        fallbacks.write().remove(index);
                    },
                    Trash { size: 14 }
                }
            }

            div { class: "grid grid-cols-2 gap-3",
                div { class: "flex flex-col gap-1.5 col-span-2",
                    label { class: "{label_class}", "model (必填)" }
                    input {
                        class: "{input_class} font-mono",
                        value: "{fallback.model}",
                        oninput: move |e| fallbacks.write()[index].model = e.value(),
                        placeholder: "fallback-model-id",
                    }
                }
                div { class: "flex flex-col gap-1.5",
                    label { class: "{label_class}", "base_url (空 = 继承主 provider)" }
                    input {
                        class: "{input_class} font-mono",
                        value: "{fallback.base_url}",
                        oninput: move |e| fallbacks.write()[index].base_url = e.value(),
                        placeholder: "http://127.0.0.1:3182",
                    }
                }
                div { class: "flex flex-col gap-1.5",
                    div { class: "flex justify-between",
                        label { class: "{label_class}", "api_key (空 = 继承主 provider)" }
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
                        value: "{fallback.api_key}",
                        oninput: move |e| fallbacks.write()[index].api_key = e.value(),
                        placeholder: "sk-...",
                    }
                }
                div { class: "flex flex-col gap-1.5 col-span-2",
                    label { class: "{label_class}", "max_tokens (留空 = 不带 max_tokens)" }
                    input {
                        class: "{input_class}",
                        r#type: "number",
                        value: "{fallback.max_tokens}",
                        oninput: move |e| fallbacks.write()[index].max_tokens = e.value(),
                        placeholder: "4096",
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
                span { "路线与进度见仓库根 PROGRESS.md；技术教训见 LESSONS.md。" }
                span { "web 数据来自 daemon 实时数据源（会话、事件流与统计均走真实 RPC）。" }
            }
        }
    }
}
