//! bin/web — thin launcher for the Dioxus LiveView web UI.

#[tokio::main]
async fn main() {
    web::launch().await;
}
