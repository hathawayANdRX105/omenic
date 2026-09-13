//! oi-web — Dioxus LiveView 壳入口（bin/web 即入口 crate，组件/页面/状态在 crates/web/）。

mod app;

#[tokio::main]
async fn main() {
    app::launch().await;
}
