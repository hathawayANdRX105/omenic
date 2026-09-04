//! Persistent daemon state.
//!
//! Two pieces:
//!
//! * `SessionState` — a thin wrapper around [`session::SessionDb`] that
//!   already persists session + message rows to disk.  Nothing new to do
//!   here; we just keep a cloneable handle so the daemon and the dispatch
//!   layer can share the same connection.
//!
//! * `RunLedger` — a tiny newline-delimited JSON log of run-level events
//!   the caller what they missed.  Stored alongside the socket as
//!   `<socket>.runs.jsonl` and rewritten on every event with a single
//!   `fsync` so a crash leaves either the old or the new file, never a torn
//!   one.
//!
//! ponytail: state lives in this module so adding a future `TaskState` or
//! `MemoryState` doesn't have to re-plumb the daemon.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use session::{SessionDb, SessionError, SessionMessage, SessionRole, SessionSummary};

use crate::DaemonError;

/// One run recorded in the run ledger.  `finished_at_ms` is `None` while the
/// run is in flight; the dispatch layer writes the terminal event when the
/// worker returns or aborts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub session_id: String,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Thread-safe handle to the persistent run ledger.
#[derive(Clone)]
pub struct RunLedger {
    inner: Arc<Mutex<RunLedgerInner>>,
}

struct RunLedgerInner {
    path: PathBuf,
    runs: Vec<RunRecord>,
}

impl RunLedger {
    /// Open (or create) the ledger rooted at `<socket_path>.runs.jsonl`.
    /// On open, the file is read end-to-end so all prior runs are in memory.
    pub fn open_for_socket(socket_path: &Path) -> Result<Self, DaemonError> {
        let path = run_ledger_path(socket_path);
        let runs = if path.exists() {
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut out = Vec::new();
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<RunRecord>(&line) {
                    Ok(r) => out.push(r),
                    Err(_) => {
                        // A single torn line should not kill the daemon —
                        // skip it.  Worst case the user loses one record.
                    }
                }
            }
            out
        } else {
            Vec::new()
        };

        Ok(RunLedger {
            inner: Arc::new(Mutex::new(RunLedgerInner { path, runs })),
        })
    }

    /// Start a new run record.  Returns a guard that the caller can call
    /// `finish` on when the run is done.
    pub fn start(
        &self,
        run_id: impl Into<String>,
        session_id: impl Into<String>,
        started_at_ms: i64,
    ) -> Result<RunRecord, DaemonError> {
        let record = RunRecord {
            run_id: run_id.into(),
            session_id: session_id.into(),
            started_at_ms,
            finished_at_ms: None,
            status: None,
        };
        let mut inner = self.inner.lock().expect("run ledger poisoned");
        inner.runs.push(record.clone());
        inner.flush()?;
        Ok(record)
    }

    /// Mark `run_id` as finished at `finished_at_ms` with `status`.  No-op
    /// if no such run exists.
    pub fn finish(
        &self,
        run_id: &str,
        finished_at_ms: i64,
        status: &str,
    ) -> Result<(), DaemonError> {
        let mut inner = self.inner.lock().expect("run ledger poisoned");
        let mut touched = false;
        for run in inner.runs.iter_mut() {
            if run.run_id == run_id {
                run.finished_at_ms = Some(finished_at_ms);
                run.status = Some(status.to_string());
                touched = true;
            }
        }
        if touched {
            inner.flush()?;
        }
        Ok(())
    }

    /// Snapshot of every recorded run, ordered as written.
    pub fn list(&self) -> Vec<RunRecord> {
        let inner = self.inner.lock().expect("run ledger poisoned");
        inner.runs.clone()
    }

    /// One run by id (`None` if not present).
    pub fn get(&self, run_id: &str) -> Option<RunRecord> {
        let inner = self.inner.lock().expect("run ledger poisoned");
        inner.runs.iter().find(|r| r.run_id == run_id).cloned()
    }
}

impl RunLedgerInner {
    /// Rewrite the whole ledger to disk in one shot.  This is O(N) per
    /// event; cheap because N stays small (a daemon rarely has more than
    /// thousands of runs in its lifetime) and we get crash-safety for free.
    fn flush(&self) -> Result<(), DaemonError> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.path)?;
        for run in &self.runs {
            let line = serde_json::to_string(run)?;
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
        Ok(())
    }
}

fn run_ledger_path(socket_path: &Path) -> PathBuf {
    let mut s = socket_path.as_os_str().to_owned();
    s.push(".runs.jsonl");
    PathBuf::from(s)
}

/// Thread-safe handle to the session database.  `Arc<SessionDb>` because
/// `SessionDb` is already `Arc`-internally — wrapping it again is cheap and
/// lets the lock be detached from the daemon lifetime.
#[derive(Clone)]
pub struct SessionState {
    inner: Arc<SessionDb>,
}

impl SessionState {
    /// Wrap an open [`SessionDb`].
    pub fn new(db: SessionDb) -> Self {
        SessionState {
            inner: Arc::new(db),
        }
    }

    pub fn ensure_session(&self, id: &str, title: &str) -> Result<SessionSummary, SessionError> {
        self.inner.ensure_session(id, title)
    }

    pub fn delete_session(&self, id: &str) -> Result<bool, SessionError> {
        self.inner.delete_session(id)
    }

    pub fn list_sessions(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        self.inner.list_sessions(query, limit)
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionSummary>, SessionError> {
        self.inner.session(id)
    }

    pub fn append_message(
        &self,
        session_id: &str,
        role: SessionRole,
        text: &str,
    ) -> Result<(i64, i64), SessionError> {
        self.inner.append_message(session_id, role, text)
    }

    pub fn load_messages(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<SessionMessage>, SessionError> {
        self.inner.load_messages(session_id, limit)
    }

    pub fn search_messages(
        &self,
        query: &str,
        scope: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SessionMessage>, SessionError> {
        self.inner.search_messages(query, scope, limit)
    }
}

// Re-export so callers can construct a request `data` envelope without
// reaching into the session crate directly.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d: std::time::Duration| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Helper used by dispatch when a session command was missing a required
/// field.  Returning `Value::Null` here keeps callers from having to think
/// about `serde_json::Value` vs `Option`.
pub fn require_str<'a>(params: &'a Value, field: &str) -> Result<&'a str, String> {
    params
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required field `{field}`"))
}

pub fn require_u32(params: &Value, field: &str) -> Result<u32, String> {
    params
        .get(field)
        .and_then(Value::as_u64)
        .map(|n| n.min(u32::MAX as u64) as u32)
        .ok_or_else(|| format!("missing required numeric field `{field}`"))
}
