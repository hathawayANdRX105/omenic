use crate::mock;
use dioxus::prelude::*;

#[component]
pub fn ConfigPage() -> Element {
    let config = mock::mock_config();

    rsx! {
        div { class: "config-page",
            div { class: "config-header",
                h1 { "模型与渠道配置" }
                p { class: "config-subtitle", "管理 LLM 提供商和可用模型" }
            }

            div { class: "config-section",
                h2 { "默认模型" }
                div { class: "config-value",
                    code { "{config.default_model}" }
                }
            }

            div { class: "config-section",
                h2 { "数据目录" }
                div { class: "config-value",
                    code { "{config.data_dir}" }
                }
            }

            div { class: "config-section",
                h2 { "Provider 一览" }
                for provider in &config.providers {
                    ProviderCard { key: "{provider.name}", provider: provider.clone() }
                }
            }
        }
    }
}

#[component]
fn ProviderCard(provider: mock::ProviderEntry) -> Element {
    rsx! {
        div { class: "provider-card",
            div { class: "provider-header",
                h3 { "{provider.name}" }
                code { class: "provider-url", "{provider.base_url}" }
            }
            table { class: "model-table",
                thead {
                    tr {
                        th { "模型" }
                        th { "显示名" }
                        th { "状态" }
                    }
                }
                tbody {
                    for model in &provider.models {
                        tr { key: "{model.name}",
                            td { code { "{model.name}" } }
                            td { "{model.display_name}" }
                            td {
                                span {
                                    class: if model.active { "model-status active" } else { "model-status" },
                                    if model.active { "✓ 已启用" } else { "— 未启用" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
