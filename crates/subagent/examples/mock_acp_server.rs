//! Scripted ACP agent binary for the `acp_provider` integration tests.
//!
//! Behaviour is selected by environment variables, the same approach as dsh's
//! `tests/mock-acp-server.ts`: a test spawns this binary through
//! [`subagent::AcpProvider`] with the scenario's env set, and
//! the provider exercises the real stdio transport end to end.
//!
//! # Scenarios
//!
//! | var | effect |
//! |---|---|
//! | `MOCK_TEXT` | assistant text streamed for the turn (canned default) |
//! | `MOCK_STOP` | stop reason reported for the turn (default `end_turn`) |
//! | `MOCK_SESSION_ID` | session id returned by `session/new` |
//! | `MOCK_NO_SESSION_ID` | `session/new` answers with no session id |
//! | `MOCK_ECHO_CWD` | stream `<process cwd>\n<announced cwd>` instead of `MOCK_TEXT` |
//! | `MOCK_PERMISSION` | ask permission before answering; a deny settles the turn `cancelled` |
//! | `MOCK_HANG` | never answer the turn on our own — wait for cancel or teardown |
//! | `MOCK_IGNORE_CANCEL` | with `MOCK_HANG`: swallow cancel and outlive stdin EOF — only SIGKILL ends us |
//! | `MOCK_CRASH` | `exit(1)` the moment the prompt arrives |
//! | `MOCK_EOF_FLUSH_MS` | on stdin EOF, wait this long before exiting (the cooperative flush window) |

use std::env;
use std::io::{self, BufRead, Write};
use std::process::exit;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const JSONRPC: &str = "2.0";
const PROTOCOL_VERSION: &str = "1";

const METHOD_INITIALIZE: &str = "initialize";
const METHOD_SESSION_NEW: &str = "session/new";
const METHOD_SESSION_PROMPT: &str = "session/prompt";
const METHOD_SESSION_CANCEL: &str = "session/cancel";
const METHOD_REQUEST_PERMISSION: &str = "session/requestPermission";
const METHOD_SESSION_UPDATE: &str = "session/update";

/// Id of the one permission request we send. Client request ids count up from
/// 1 and there are exactly three, so this cannot collide with a reply.
const PERMISSION_ID: u64 = 900;

/// A decision the main thread forwards to the prompt thread.
enum Decision {
    /// The client's permission answer: `allow` / `deny` (or unreadable).
    Permission(Option<String>),
    /// The client cancelled the turn.
    Cancel,
}

/// Everything the prompt thread needs to play the turn out.
struct ServePrompt {
    out: Arc<Mutex<io::Stdout>>,
    decision_rx: Option<mpsc::Receiver<Decision>>,
    prompt_id: Value,
    session_id: String,
    announced_cwd: String,
    text: String,
    stop_reason: String,
    echo_cwd: bool,
    hang: bool,
    want_permission: bool,
    ignore_cancel: bool,
}

fn env_flag(name: &str) -> bool {
    // Presence with a non-empty, non-"0" value; absent means off.
    env::var(name)
        .map(|value| value != "0" && !value.is_empty())
        .unwrap_or(false)
}

fn main() {
    let text = env::var("MOCK_TEXT").unwrap_or_else(|_| "mock child answer".to_string());
    let stop_reason = env::var("MOCK_STOP").unwrap_or_else(|_| "end_turn".to_string());
    let echo_cwd = env_flag("MOCK_ECHO_CWD");
    let hang = env_flag("MOCK_HANG");
    let crash = env_flag("MOCK_CRASH");
    let want_permission = env_flag("MOCK_PERMISSION");
    let ignore_cancel = env_flag("MOCK_IGNORE_CANCEL");
    let no_session_id = env_flag("MOCK_NO_SESSION_ID");
    let session_id = env::var("MOCK_SESSION_ID").unwrap_or_else(|_| "mock-session".to_string());
    let eof_flush_ms = env::var("MOCK_EOF_FLUSH_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(0);

    let out = Arc::new(Mutex::new(io::stdout()));
    let (decision_tx, decision_rx) = mpsc::channel::<Decision>();
    // One prompt per run: the receiver moves into the prompt thread exactly
    // once, so it lives behind an `Option` the loop can take.
    let mut decision_rx = Some(decision_rx);

    // The session id is established by `session/new` and consumed by the
    // prompt thread; the turn itself starts at `session/prompt`.
    let mut established: Option<String> = None;
    let mut announced_cwd = String::new();

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(line) = lines.next() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            // Malformed lines are skipped, never fatal — the client does the
            // same, and an agent may emit logs on stdout.
            Err(_) => continue,
        };
        match (
            value.get("id").cloned(),
            value.get("method").and_then(Value::as_str),
        ) {
            (Some(id), Some(METHOD_INITIALIZE)) => reply(
                &out,
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "agentCapabilities": {},
                    "authMethods": [],
                }),
            ),
            (Some(id), Some(METHOD_SESSION_NEW)) => {
                announced_cwd = value
                    .get("params")
                    .and_then(|params| params.get("cwd"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if no_session_id {
                    reply(&out, id, json!({}));
                } else {
                    reply(&out, id, json!({"sessionId": session_id}));
                    established = Some(session_id.clone());
                }
            }
            (Some(id), Some(METHOD_SESSION_PROMPT)) => {
                if crash {
                    exit(1);
                }
                // One turn per run: hand the decision channel and the
                // scenario to the prompt thread, then keep reading so cancel
                // and permission replies still arrive while it works. The
                // values are rebound here because a `move` closure would
                // capture them on the first pass and starve the next.
                let out = out.clone();
                let decision_rx = decision_rx.take();
                let session_id = established.clone().unwrap_or_default();
                let announced_cwd = announced_cwd.clone();
                let text = text.clone();
                let stop_reason = stop_reason.clone();
                thread::spawn(move || {
                    serve_prompt(ServePrompt {
                        out,
                        decision_rx,
                        prompt_id: id,
                        session_id,
                        announced_cwd,
                        text,
                        stop_reason,
                        echo_cwd,
                        hang,
                        want_permission,
                        ignore_cancel,
                    });
                });
            }
            (Some(id), None) => {
                // A reply to our permission request.
                if id.as_u64() == Some(PERMISSION_ID) {
                    // The client replies with { "result": { "outcome":
                    // { "type": "allow"|"deny", ... } } }.
                    let outcome = value
                        .get("result")
                        .and_then(|result| result.get("outcome"))
                        .and_then(|outcome| outcome.get("type"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let _ = decision_tx.send(Decision::Permission(outcome));
                }
            }
            (None, Some(METHOD_SESSION_CANCEL)) => {
                let _ = decision_tx.send(Decision::Cancel);
            }
            // Unknown requests and notifications: ignored, never fatal.
            _ => {}
        }
    }

    // stdin EOF: the harness closed our input (dispose) or exited. Wait out
    // the cooperative flush window before going down.
    if eof_flush_ms > 0 {
        thread::sleep(Duration::from_millis(eof_flush_ms));
    }
    if ignore_cancel {
        // A child that neither answers nor quiesces — this is the case the
        // provider's SIGKILL tier exists for, so stay until it lands.
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
}

fn serve_prompt(prompt: ServePrompt) {
    let ServePrompt {
        out,
        decision_rx,
        prompt_id,
        session_id,
        announced_cwd,
        text,
        stop_reason,
        echo_cwd,
        hang,
        want_permission,
        ignore_cancel,
    } = prompt;
    let decision_rx = match decision_rx {
        Some(decision_rx) => decision_rx,
        // No decision channel means the session was never established; there
        // is nothing to drive but teardown.
        None => return,
    };

    if want_permission {
        notify(
            &out,
            json!({
                "jsonrpc": JSONRPC,
                "id": PERMISSION_ID,
                "method": METHOD_REQUEST_PERMISSION,
                "params": {
                    "sessionId": session_id,
                    "permissionId": "mock-permission",
                    "options": [
                        {"optionId": "yes", "title": "Allow"},
                        {"optionId": "no", "title": "Reject"},
                    ],
                },
            }),
        );
        // A deny (or an unreadable answer) cancels the turn.
        match decision_rx.recv() {
            Ok(Decision::Permission(Some(outcome))) if outcome == "allow" => {}
            _ => {
                reply(&out, prompt_id, json!({"stopReason": "cancelled"}));
                exit(0);
            }
        }
    }

    let body = if echo_cwd {
        // Both cwds, on separate lines: the process cwd proves the spawn, the
        // announced cwd proves `session/new` plumbing.
        let real_cwd = env::current_dir()
            .map(|cwd| cwd.display().to_string())
            .unwrap_or_default();
        format!("{real_cwd}\n{announced_cwd}")
    } else {
        text
    };
    notify(
        &out,
        json!({
            "jsonrpc": JSONRPC,
            "method": METHOD_SESSION_UPDATE,
            "params": {
                "sessionId": session_id,
                "update": {
                    "type": "agentMessageChunk",
                    "content": [{"text": body}],
                },
            },
        }),
    );

    // The terminal stop reason, unless the scenario makes us wait.
    let final_reason = if hang {
        loop {
            match decision_rx.recv() {
                Ok(Decision::Cancel) if !ignore_cancel => break "cancelled".to_string(),
                // Swallow the cancel (and any stray permission reply); only
                // teardown ends us.
                Ok(_) => continue,
                // The harness closed the transport — no reply to write.
                Err(_) => return,
            }
        }
    } else {
        stop_reason
    };

    reply(&out, prompt_id, json!({"stopReason": final_reason}));
    // The turn is over; a well-behaved agent exits so the harness reaps
    // without waiting on the ladder.
    exit(0);
}

/// Write one NDJSON frame and flush — the client blocks on a full line.
fn send(out: &Arc<Mutex<io::Stdout>>, value: Value) {
    let mut line = match serde_json::to_string(&value) {
        Ok(line) => line,
        Err(_) => return,
    };
    line.push('\n');
    let mut out = out.lock().unwrap();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

/// Answer a client request with the same id.
fn reply(out: &Arc<Mutex<io::Stdout>>, id: Value, result: Value) {
    send(out, json!({"jsonrpc": JSONRPC, "id": id, "result": result}));
}

/// Emit a notification (no id, no reply expected).
fn notify(out: &Arc<Mutex<io::Stdout>>, value: Value) {
    send(out, value);
}
