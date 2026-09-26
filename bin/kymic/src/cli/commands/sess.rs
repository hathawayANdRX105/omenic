//! Session + daemon RPC handlers, split out of `cli.rs`.

use super::*;

use crate::cli::{DaemonCmd, SessionCmd};
use config::Config;

// ---------------- Session commands ----------------

/// Build a daemon client from `Config` (honors `OMENIC_DAEMON_SOCKET`).
pub fn daemon_client_from_config() -> Result<daemon::DaemonClient, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    daemon::DaemonClient::from_config(&config).map_err(|e| format!("daemon socket error: {e}"))
}

/// Dispatch `session ...` subcommands. Routes through `DaemonClient`.
pub fn session_cmd_dispatch(sub: SessionCmd, json: bool) -> Result<u8, String> {
    let client = daemon_client_from_config()?;
    match sub {
        SessionCmd::List { query, limit } => session_list_cmd(&client, &query, limit, json),
        SessionCmd::Get { id } => session_get_cmd(&client, &id, json),
        SessionCmd::Search {
            query,
            scope,
            limit,
        } => session_search_cmd(&client, &query, scope.as_deref(), limit, json),
        SessionCmd::Delete { id } => session_delete_cmd(&client, &id, json),
        SessionCmd::Query {
            kind,
            query,
            session_id,
            limit,
        } => session_query_cmd(
            &client,
            &kind,
            query.as_deref(),
            session_id.as_deref(),
            limit,
            json,
        ),
        SessionCmd::Attach { id, limit } => session_attach_cmd(&client, &id, limit, json),
        SessionCmd::Resume { id, message } => session_resume_cmd(&client, &id, &message, json),
    }
}

/// `oi session list <query> [--limit N]` — text or JSON list.
pub fn session_list_cmd(
    client: &daemon::DaemonClient,
    query: &str,
    limit: u32,
    json: bool,
) -> Result<u8, String> {
    let rows = client
        .session_list(query, limit)
        .map_err(|e| format!("daemon error: {e}"))?;
    if json {
        print_json(&rows);
        return Ok(0);
    }
    if rows.is_empty() {
        println!("(no sessions matching `{query}`)");
        return Ok(0);
    }
    println!("{} session(s) matching `{query}`:", rows.len());
    for row in &rows {
        println!(
            "  {} | {} | {} msgs | updated {}",
            row.id, row.title, row.message_count, row.updated_at_ms
        );
    }
    Ok(0)
}

/// `oi session attach <id> [--limit N]` — session summary + recent messages.
pub fn session_attach_cmd(
    client: &daemon::DaemonClient,
    id: &str,
    limit: u32,
    json: bool,
) -> Result<u8, String> {
    let sess = client
        .session_get(id)
        .map_err(|e| format!("daemon error: {e}"))?;
    let Some(sess) = sess else {
        return Err(format!("session not found: {id}"));
    };
    let msgs = client
        .session_load_messages(id, limit)
        .map_err(|e| format!("daemon error: {e}"))?;
    if json {
        print_json(&serde_json::json!({ "session": sess, "messages": msgs }));
        return Ok(0);
    }
    println!(
        "session {} | {} | {} msgs | updated {}",
        sess.id, sess.title, sess.message_count, sess.updated_at_ms
    );
    for m in &msgs {
        let text: String = m.text.chars().take(120).collect();
        println!("  #{} [{:?}] {}", m.seq, m.role, text);
    }
    Ok(0)
}

/// `oi session resume <id> <message>` — route a follow-up prompt to the
/// daemon worker. The run is recorded in the ledger (correlatable across
/// reconnects via run.list); a failed worker spawn surfaces as an error and
/// the ledger keeps the `spawn_failed` record.
pub fn session_resume_cmd(
    client: &daemon::DaemonClient,
    id: &str,
    message: &str,
    json: bool,
) -> Result<u8, String> {
    if client
        .session_get(id)
        .map_err(|e| format!("daemon error: {e}"))?
        .is_none()
    {
        return Err(format!("session not found: {id}"));
    }
    let run_id = format!(
        "r-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let result: serde_json::Value = client
        .call(
            daemon::Command::WorkerPrompt,
            serde_json::json!({
                "message": message,
                "session_id": id,
                "run_id": run_id,
            }),
        )
        .map_err(|e| format!("daemon error: {e}"))?;
    if !json {
        println!("resumed session {id} (run {run_id})");
    }
    print_json(&result);
    Ok(0)
}

/// `oi session get <id>` — text or JSON single summary.
pub fn session_get_cmd(client: &daemon::DaemonClient, id: &str, json: bool) -> Result<u8, String> {
    let row: Option<session::SessionSummary> = client
        .session_get(id)
        .map_err(|e| format!("daemon error: {e}"))?;
    let row = row.ok_or_else(|| format!("session `{id}` not found"))?;
    if json {
        print_json(&row);
        return Ok(0);
    }
    println!(
        "{} | {} | {} msgs | created {} | updated {}",
        row.id, row.title, row.message_count, row.created_at_ms, row.updated_at_ms
    );
    Ok(0)
}

/// `oi session search <query> [--scope ID] [--limit N]`.
pub fn session_search_cmd(
    client: &daemon::DaemonClient,
    query: &str,
    scope: Option<&str>,
    limit: u32,
    json: bool,
) -> Result<u8, String> {
    let msgs = client
        .session_search(query, scope, limit)
        .map_err(|e| format!("daemon error: {e}"))?;
    if json {
        print_json(&msgs);
        return Ok(0);
    }
    if msgs.is_empty() {
        println!("(no messages matching `{query}`)");
        return Ok(0);
    }
    println!("{} message(s) matching `{query}`:", msgs.len());
    for m in &msgs {
        println!(
            "  [{}:{}] {} | {}",
            m.session_id,
            m.seq,
            m.role.as_str(),
            m.text.replace('\n', " ")
        );
    }
    Ok(0)
}

/// `oi session delete <id>` — JSON `{"deleted":bool}`.
pub fn session_delete_cmd(
    client: &daemon::DaemonClient,
    id: &str,
    json: bool,
) -> Result<u8, String> {
    let deleted = client
        .session_delete(id)
        .map_err(|e| format!("daemon error: {e}"))?;
    if json {
        print_json(&serde_json::json!({ "deleted": deleted }));
        return Ok(0);
    }
    println!(
        "{}",
        if deleted {
            format!("deleted session `{id}`")
        } else {
            format!("session `{id}` not found")
        }
    );
    Ok(0)
}

/// `oi session query ...` — agent-facing dispatcher with the same args
/// shape as the `session_query` ToolDef.
pub fn session_query_cmd(
    client: &daemon::DaemonClient,
    kind: &str,
    query: Option<&str>,
    session_id: Option<&str>,
    limit: u32,
    json: bool,
) -> Result<u8, String> {
    let mut args = serde_json::json!({ "kind": kind, "limit": limit });
    if let Some(q) = query {
        args["query"] = serde_json::Value::String(q.to_string());
    }
    if let Some(id) = session_id {
        args["session_id"] = serde_json::Value::String(id.to_string());
    }
    let result = client
        .session_query(&args)
        .map_err(|e| format!("daemon error: {e}"))?;
    if json {
        print_json(&result);
        return Ok(0);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    Ok(0)
}

// ---------------- Daemon commands ----------------

/// Locate the daemon binary: explicit override first, then the sibling of
/// this executable (workspace layout: target/debug/{oi,daemon}).
///
/// Single source for the `OMENIC_DAEMON_PATH` / sibling rule — `oi daemon
/// start` resolves through here, and so does the `oi tui` autostart gate
/// (T14) in `cli.rs`; neither caller re-implements the lookup.
pub fn resolve_daemon_bin() -> Result<std::path::PathBuf, String> {
    let bin = match std::env::var_os("OMENIC_DAEMON_PATH") {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let exe =
                std::env::current_exe().map_err(|e| format!("cannot resolve own path: {e}"))?;
            exe.parent()
                .map(|d| d.join("daemon"))
                .ok_or_else(|| "no executable directory".to_string())?
        }
    };
    if !bin.is_file() {
        return Err(format!(
            "daemon binary not found at {} (set OMENIC_DAEMON_PATH to override; or build it with `cargo build --bin daemon`)",
            bin.display()
        ));
    }
    Ok(bin)
}

pub fn daemon_cmd_dispatch(sub: DaemonCmd, json: bool) -> Result<u8, String> {
    let client = daemon_client_from_config()?;
    match sub {
        DaemonCmd::Start => {
            if client.ping().unwrap_or(false) {
                let info = client.info().map_err(|e| format!("daemon error: {e}"))?;
                if json {
                    print_json(&serde_json::json!({
                        "running": true, "already_running": true, "pid": info.pid,
                    }));
                } else {
                    println!("daemon already running (pid {})", info.pid);
                }
                return Ok(0);
            }
            // Binary lookup lives in `resolve_daemon_bin` (shared with the
            // `oi tui` autostart gate); the ping check above must stay first
            // so a running daemon never depends on a resolvable path.
            let bin = resolve_daemon_bin()?;
            std::process::Command::new(&bin)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("failed to spawn {}: {e}", bin.display()))?;
            for _ in 0..250 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                if client.ping().unwrap_or(false) {
                    let info = client.info().map_err(|e| format!("daemon error: {e}"))?;
                    if json {
                        print_json(&serde_json::json!({
                            "running": true, "started": true, "pid": info.pid,
                        }));
                    } else {
                        println!("daemon started (pid {})", info.pid);
                    }
                    return Ok(0);
                }
            }
            Err("daemon did not come up within 5s".to_string())
        }
        DaemonCmd::Status => {
            let info = client.info().map_err(|e| format!("daemon error: {e}"))?;
            if json {
                print_json(&info);
            } else {
                println!(
                    "daemon running | pid {} | uptime {} ms | worker pid {}",
                    info.pid, info.uptime_ms, info.worker_pid
                );
            }
            Ok(0)
        }
        DaemonCmd::Stop => {
            client
                .shutdown()
                .map_err(|e| format!("daemon error: {e}"))?;
            if json {
                print_json(&serde_json::json!({ "stopping": true }));
            } else {
                println!("daemon stopping");
            }
            Ok(0)
        }
    }
}
