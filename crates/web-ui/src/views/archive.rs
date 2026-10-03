//! 归档页（中心列）：列出软删（归档）会话，每条带「恢复 / 彻底删除」。
//!
//! 数据来自 daemon 的 `session.list_archived`（冷归档存储，见 session 层
//! `archive`）。取数遵循 [`super::stats::StatsView`] 的约束：渲染/effect 里不能
//! 同步 RPC（LiveView 跑在 tokio，会 runtime-within-a-runtime panic），一律套
//! `std::thread::spawn(...).join()`。无 daemon / RPC 失败 → 空态，不回退假数据。

use dioxus::prelude::*;
use web_client::daemon::WebDaemon;
use web_state::types::Session;

use crate::shared as sh;
use ui_kit::icons::{IconTrash, IconUndo};

/// 归档页：软删会话列表 + 恢复 / 彻底删除。`rev` 变化（本页操作后自增）即重取。
#[component]
pub fn ArchiveView(rev: Signal<u32>) -> Element {
    // daemon 探测只做一次（socket 由 KYMIDO_DAEMON_SOCKET / 平台配置目录决定）。
    let daemon = use_signal(|| {
        match std::thread::spawn(|| WebDaemon::from_env_or_default().filter(|d| d.ping())).join() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[web] archive daemon probe thread panicked: {e:?}");
                None
            }
        }
    });

    // 归档会话列表。None = 未取到（无 daemon / 失败）→ 空态。
    let mut items: Signal<Option<Vec<Session>>> = use_signal(|| None);

    // `rev` 变化（本页 restore/purge 之后）即重取。
    use_effect(move || {
        let _ = rev();
        let Some(d) = daemon() else {
            items.set(None);
            return;
        };
        let fetched = std::thread::spawn(move || d.list_archived_sessions().ok())
            .join()
            .ok()
            .flatten();
        items.set(Some(fetched.unwrap_or_default()));
    });

    let list = items.read().clone().unwrap_or_default();

    rsx! {
        div { class: "flex-1 overflow-y-auto flex flex-col gap-4 px-10 pt-8 pb-14 max-w-[860px] w-full mx-auto",
            // 头部
            div { class: "pb-4 border-b border-border",
                h2 { class: "role-card", "{sh::LBL_ARCHIVE}" }
                p { class: "role-caption mt-1", "软删的会话存于此处（冷归档），可恢复回会话列表，或彻底删除。" }
            }
            if items.read().is_none() {
                div { class: "py-10 text-center role-caption", "未连接 daemon，暂无归档会话" }
            } else if list.is_empty() {
                div { class: "py-10 text-center role-caption", "暂无归档会话" }
            } else {
                div { class: "flex flex-col gap-1.5",
                    for s in list.iter().cloned() {
                        {archive_row(s, daemon, rev)}
                    }
                }
            }
        }
    }
}

/// 单条归档会话行：标题 + 最近活跃 + 恢复 / 彻底删除。
/// `daemon` / `rev` 为 Copy 信号（'static），onclick 闭包按值捕获后可入事件树；
/// RPC 放线程 join，完成后自增 rev 触发重取（同 StatsView 约束：LiveView 内不能同步 RPC）。
fn archive_row(s: Session, daemon: Signal<Option<WebDaemon>>, mut rev: Signal<u32>) -> Element {
    let id_restore = s.id.clone();
    let id_purge = s.id.clone();
    rsx! {
        div { key: "{s.id}", class: "group flex items-center gap-3 h-11 px-3 rounded-xl border border-border bg-card hover:border-border transition-colors",
            span { class: "role-hint text-foreground truncate min-w-0 flex-1", "{s.title}" }
            span { class: "role-caption tabular-nums shrink-0", "{s.last_active}" }
            button {
                class: "shrink-0 flex items-center justify-center w-7 h-7 rounded-lg text-muted-foreground hover:text-foreground hover:bg-secondary-hover transition-colors cursor-pointer bg-transparent border-none",
                title: sh::BTN_RESTORE_SESSION,
                onclick: move |_| {
                    let Some(d) = daemon() else { return };
                    let id = id_restore.clone();
                    let _ = std::thread::spawn(move || d.restore_session(&id)).join();
                    rev.set(rev() + 1);
                },
                IconUndo { size: 15 }
            }
            button {
                class: "shrink-0 flex items-center justify-center w-7 h-7 rounded-lg text-muted-foreground hover:text-destructive hover:bg-destructive/15 transition-colors cursor-pointer bg-transparent border-none",
                title: sh::BTN_PURGE_SESSION,
                onclick: move |_| {
                    let Some(d) = daemon() else { return };
                    let id = id_purge.clone();
                    let _ = std::thread::spawn(move || d.purge_archived_session(&id)).join();
                    rev.set(rev() + 1);
                },
                IconTrash { size: 15 }
            }
        }
    }
}
