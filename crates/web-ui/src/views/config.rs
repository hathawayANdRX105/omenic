//! 设置页面（dsh SettingsRoot 复刻的提级形态）：中栏视图，分区由侧栏
//! 「设置」二级菜单选定（[`SettingsSection`]），800px 弹窗与 188px 内导航已退役。
//! 「模型与渠道」承载 LLM 配置表单；「MCP 服务器」承载 [[mcp.servers]]
//! 的列表编辑；「关于」放版本与项目信息。

use dioxus::prelude::*;
use ui_kit::button::{Button, ButtonSize, ButtonVariant};
use ui_kit::icons::IconTrash;
use web_client::llm::{LlmRuntimeConfig, McpServerForm};

use crate::nav::SettingsSection;

/// Max Tokens 输入校验：空白修剪后须为 16–200,000 的 u32，合法返回 `None`。
pub fn validate_max_tokens(value: &str) -> Option<&'static str> {
    match value.trim().parse::<u32>() {
        Ok(v) if v < 16 => Some("Max Tokens 至少 16"),
        Ok(v) if v > 200_000 => Some("Max Tokens 超过上限 200,000"),
        Ok(_) => None,
        Err(_) => Some("必须为有效正整数"),
    }
}

/// 设置页面（中栏正文）：分区由侧栏二级菜单写入的 `section` 信号驱动，
/// 页面只渲染对应分区 pane；弹窗壳（Modal + 内导航 + 关闭钮）已删。
#[component]
pub fn SettingsPage(
    section: Signal<SettingsSection>,
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    let s = section();
    rsx! {
        div { class: "flex-1 min-h-0 flex flex-col",
            div { class: "h-[54px] px-6 flex items-center justify-between border-b border-border shrink-0",
                span { class: "role-hint font-medium text-foreground", "{s.label()}" }
            }
            div { class: "flex-1 overflow-y-auto",
                if s == SettingsSection::Models {
                    ConfigForm { config: config.clone(), on_update_config: on_update_config }
                } else if s == SettingsSection::Mcp {
                    McpServersPane { config: config.clone(), on_update_config: on_update_config }
                } else {
                    AboutPane {}
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

    // 两个 `move` 闭包（保存 / 测试连接）各持一份：save 闭包移走
    // `config`，probe 闭包用独立克隆，避免二次 move。
    let probe_config = config.clone();
    let mut is_testing = use_signal(|| false);

    // Fallback 路由（providers.toml 的 provider 行 `fallbacks` 键）表单状态与校验：
    // 每行一条 "provider/model" 路由文本，主 provider 无可用内容时按序切换。
    let mut fallbacks = use_signal(|| config.fallback_routes.clone());
    let fallback_list = fallbacks();
    let fallback_empty = fallback_list.iter().any(|r| r.trim().is_empty());
    let fallback_hint: Option<&'static str> = if fallback_empty {
        Some("fallback 路由不能为空：每行一条「provider/model」路由文本")
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

    let input_class = "w-full h-9 rounded-[10px] bg-secondary border border-border px-3 role-hint text-foreground outline-none transition-colors focus:border-brand placeholder:text-muted-foreground";
    let label_class = "role-caption font-medium";
    let err_class = "role-caption text-destructive";

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
                            div { class: "px-4 py-2.5 rounded-[10px] role-caption bg-success text-success-foreground", "{msg}" }
                        },
                        Err(err) => rsx! {
                            div { class: "px-4 py-2.5 rounded-[10px] role-caption bg-chip-danger text-destructive", "{err}" }
                        },
                    }
                }
            }
            // 连接测试反馈
            if let Some(res) = test_status.read().as_ref() {
                match res {
                    Ok(list) => rsx! {
                        div { class: "px-4 py-2.5 rounded-[10px] role-caption bg-success text-success-foreground",
                            "连接成功：已探测到 {list.len()} 个可用模型"
                        }
                    },
                    Err(err) => rsx! {
                        div { class: "px-4 py-2.5 rounded-[10px] role-caption bg-chip-danger text-destructive",
                            "连接失败: {err}"
                        }
                    },
                }
            }

            // 凭证与端点卡
            div { class: "bg-card border border-border rounded-2xl px-6 py-5 flex flex-col gap-4",
                div { class: "flex items-center justify-between pb-3 border-b border-border",
                    div { class: "role-desc font-medium text-foreground", "LLM API 凭证与端点" }
                    span { class: "font-mono role-label", "OpenAI-compatible" }
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
                            placeholder: "~/.kymido",
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
                                    active_provider: config.active_provider.clone(),
                                    active: config.active.clone(),
                                    // LLM 保存沿用当前已保存的 MCP 表单状态：
                                    // 空表单时 [mcp] 完全不被触碰。
                                    mcp_servers: config.mcp_servers.clone(),
                                    // Fallback 路由当前状态随本次保存整体写回
                                    // active provider 行的 fallbacks 键。
                                    fallback_routes: fallbacks(),
                                    context_max: config.context_max,
                                    image_input: config.image_input,
                                };
                                match new_cfg
                                    .save_to_file()
                                    .and_then(|_| new_cfg.save_providers_to_file())
                                {
                                    Ok(()) => {
                                        save_status.set(Some(Ok(
                                            "配置已写入 .kymido/config.toml 与 providers.toml 并生效".into(),
                                        )));
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
                                active_provider: probe_config.active_provider.clone(),
                                active: probe_config.active.clone(),
                                // 探针只读 base_url/api_key，不落盘：空 vec
                                // 让它即使被误存也不会碰 [mcp]。
                                mcp_servers: Vec::new(),
                                fallback_routes: Vec::new(),
                                context_max: probe_config.context_max,
                                image_input: probe_config.image_input,
                            };
                            let res = probe_cfg.test_connection();
                            test_status.set(Some(res));
                            is_testing.set(false);
                        },
                        if is_testing() { "正在测试..." } else { "测试连接" }
                    }
                }
            }

            // Fallback 路由卡（providers.toml 的 provider 行 fallbacks 键，
            // 主 provider 无可用内容时按序瀑布切换）
            div { class: "bg-card border border-border rounded-2xl px-6 py-5 flex flex-col gap-4",
                div { class: "flex items-center justify-between pb-3 border-b border-border",
                    div { class: "role-desc font-medium text-foreground", "Fallback 路由（主 provider 失败后按序切换）" }
                    Button {
                        variant: ButtonVariant::Outline,
                        size: ButtonSize::Sm,
                        onclick: move |_| fallbacks.write().push(String::new()),
                        "添加 Fallback 路由"
                    }
                }

                if fallback_list.is_empty() {
                    div { class: "bg-card border border-border rounded-xl px-4 py-6 flex flex-col items-center gap-1.5",
                        span { class: "role-caption", "尚未配置 Fallback 路由" }
                        span { class: "role-caption", "主 provider 无可用内容时按列表顺序切换；每行一条「provider/model」路由，凭据继承目标 provider 行" }
                    }
                }
                for (i, route) in fallback_list.iter().enumerate() {
                    // `{i}` key 前缀保证中间行删除时上方行不继承被删组件状态，
                    // 重复路由编辑期间键仍唯一。
                    div { key: "{i}", class: "flex items-center gap-2.5",
                        span { class: "role-caption font-mono w-[32px] text-right", "#{i + 1}" }
                        input {
                            class: "w-full h-9 rounded-[10px] bg-secondary border border-border px-3 role-hint text-foreground font-mono outline-none transition-colors focus:border-brand placeholder:text-muted-foreground",
                            value: "{route}",
                            oninput: move |e| fallbacks.write()[i] = e.value(),
                            placeholder: "other-provider/model-id",
                        }
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::IconSm,
                            title: "删除",
                            onclick: move |_| {
                                fallbacks.write().remove(i);
                            },
                            IconTrash { size: 14 }
                        }
                    }
                }
                if let Some(hint) = fallback_hint {
                    span { class: "{err_class}", "{hint}" }
                }
            }

            // 在线模型卡
            div { class: "bg-card border border-border rounded-2xl px-6 py-5 flex flex-col gap-4",
                div { class: "flex items-center justify-between pb-3 border-b border-border",
                    div { class: "role-desc font-medium text-foreground", "在线可用模型 ({models.len()})" }
                    span { class: "role-label", "点击选用" }
                }

                div { class: "grid grid-cols-2 gap-2.5",
                    for m in &models {
                        {
                            let m_str = m.clone();
                            let is_current = *model.read() == *m;
                            let card_class = if is_current {
                                "p-3 rounded-xl border border-brand bg-secondary cursor-pointer transition-colors"
                            } else {
                                "p-3 rounded-xl border border-border bg-secondary cursor-pointer transition-colors hover:border-border"
                            };
                            rsx! {
                                div { key: "{m}", class: "{card_class}",
                                    div { class: "flex items-center gap-2",
                                        span { class: "text-brand-300 font-mono role-caption font-medium", "{m}" }
                                        if is_current {
                                            span { class: "bg-success text-success-foreground px-1.5 py-px rounded-md role-label font-semibold", "默认" }
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

    let err_class = "role-caption text-destructive";

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
                            div { class: "px-4 py-2.5 rounded-[10px] role-caption bg-success text-success-foreground", "{msg}" }
                        },
                        Err(err) => rsx! {
                            div { class: "px-4 py-2.5 rounded-[10px] role-caption bg-chip-danger text-destructive", "{err}" }
                        },
                    }
                }
            }

            // 页头：说明 + 添加按钮
            div { class: "flex items-center justify-between gap-3",
                div { class: "min-w-0 flex flex-col gap-0.5",
                    div { class: "role-desc font-medium text-foreground", "MCP 服务器" }
                    span { class: "role-caption",
                        "stdio 子进程（command）或 HTTP 端点（url）；保存按 name 更新。env / reconnect 等高级键请直接编辑 .kymido/config.toml，不会被覆盖；删除仅移除编辑卡，不从配置文件删服务器。"
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
                div { class: "bg-card border border-border rounded-2xl px-6 py-8 flex flex-col items-center gap-1.5",
                    span { class: "role-caption", "尚未配置 MCP 服务器" }
                    span { class: "role-caption", "MCP 默认关闭：不添加服务器时 agent 不会启动任何外部工具进程" }
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
                                active_provider: config.active_provider.clone(),
                                active: config.active.clone(),
                                mcp_servers: servers(),
                                // MCP 保存不动 LLM 区：沿用当前已保存的
                                // fallback 路由。
                                fallback_routes: config.fallback_routes.clone(),
                                context_max: config.context_max,
                                image_input: config.image_input,
                            };
                            match new_cfg.save_to_file() {
                                Ok(()) => {
                                    save_status.set(Some(Ok("MCP 配置已写入 .kymido/config.toml".into())));
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
    let input_class = "w-full h-9 rounded-[10px] bg-secondary border border-border px-3 role-hint text-foreground outline-none transition-colors focus:border-brand placeholder:text-muted-foreground";
    let label_class = "role-caption font-medium";

    let transport = if !server.command.trim().is_empty() {
        format!("stdio · {}", server.command.trim())
    } else if !server.url.trim().is_empty() {
        format!("http · {}", server.url.trim())
    } else {
        "未配置启动方式".to_string()
    };
    let card_class = "bg-card border border-border rounded-xl px-4 py-4 flex flex-col gap-3";

    rsx! {
        div { class: "{card_class}",
            // 卡头：name + 启动方式摘要 + 删除
            div { class: "flex items-center justify-between gap-3",
                div { class: "min-w-0 flex flex-col gap-0.5",
                    span { class: "role-hint font-medium text-foreground truncate",
                        if server.name.trim().is_empty() { "（未命名服务器）" } else { "{server.name}" }
                    }
                    span { class: "role-caption font-mono truncate", "{transport}" }
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::IconSm,
                    title: "删除",
                    onclick: move |_| {
                        servers.write().remove(index);
                    },
                    IconTrash { size: 14 }
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
                        placeholder: "--root, /path/to/directory",
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
                span { class: "role-card tracking-[0.04em]", "kymido" }
                span { class: "role-caption font-mono px-2 py-0.5 rounded-full bg-card border border-border", "v{version}" }
            }
            p { class: "role-hint",
                "agent harness 的 Rust 复刻（参考 DeepSeek Harness）：agent 循环、会话持久化、事件流与插件面。web UI 为 Dioxus LiveView，设计语言复刻 dsh web。"
            }
            div { class: "flex flex-col gap-1.5 role-caption",
                span { "路线与进度见仓库根 PROGRESS.md；技术教训见 LESSONS.md。" }
                span { "web 数据来自 daemon 实时数据源（会话、事件流与统计均走真实 RPC）。" }
            }
        }
    }
}
