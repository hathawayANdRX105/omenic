//! Headless omenic daemon.
//!
//! Provides a single-instance, JSONL-over-UDS daemon that owns a long-lived
//! `rpc::worker::Worker` and a `session::SessionDb`, exposing a small request
//! surface to local CLI/TUI clients.
//!
//! Lifecycle:
//!
//! ```text
//! Daemon::start(config)
//!   ├─ acquire exclusive InstanceLock (pid file + fs2 lock)
//!   ├─ bind Unix-domain socket (clean up stale file first)
//!   ├─ open SessionDb
//!   ├─ spawn omp Worker (best-effort; deferred until first prompt)
//!   ├─ accept loop on a background thread
//!   └─ Drop → join thread, drop Worker, close socket, release lock
//! ```
//!
//! Protocol: newline-delimited JSON.  Every request is
//! `{ "id": "...", "type": "<command>", ... }` and every response is
//! `{ "id": "...", "type": "response", "success": bool, ... }`.  The set of
//! `type`s is closed and listed in [`protocol::Command`].
//!
//! Cross-platform: the Unix-domain-socket listener is `cfg(target_family =
//! "unix")`; Windows is currently a stub that returns
//! [`DaemonError::UnsupportedPlatform`].

pub mod client;
mod dispatch;
pub mod lock;
pub mod protocol;
mod server;
pub mod session_query;
mod socket;
pub mod state;

pub use client::{AppendOutcome, ClientError, DaemonClient, DaemonInfo};
pub use lock::InstanceLock;
pub use protocol::{Command, Request, Response, ResponseError};
pub use server::{Daemon, DaemonConfig};
pub use session_query::SESSION_QUERY_NAME;
pub use socket::SocketAddr;

/// Daemon-side error type.  All variants are `Display + Error`; callers can
/// downcast `Box<dyn Error>` if they need to distinguish.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// I/O error (socket bind, lock file, pid file).
    #[error("daemon I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Config error.
    #[error("daemon config error: {0}")]
    Config(#[from] config::ConfigError),

    /// JSON (de)serialization error.
    #[error("daemon JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("daemon session error: {0}")]
    Session(#[from] session::SessionError),
    #[error("another omenic daemon is already running (pid {pid})")]
    AlreadyRunning { pid: u32 },

    /// Platform is not supported by this build (e.g. Windows).
    #[error("omenic daemon is not supported on this platform")]
    UnsupportedPlatform,

    /// A request was malformed (parse OK, but unknown command / bad shape).
    #[error("protocol error: {0}")]
    Protocol(String),
}
