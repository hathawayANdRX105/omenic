//! JSONL session store: append-only chat messages with fcntl lock.
//!
//! One file per session at `<data_dir>/sessions/<id>.jsonl`.
//! Each line is `{"role":"user|assistant","text":"..."}`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::app::ChatMsg;

/// Errors from store operations.
#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    CorruptLine { line: usize, msg: String },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "I/O error: {e}"),
            StoreError::Json(e) => write!(f, "JSON error: {e}"),
            StoreError::CorruptLine { line, msg } => {
                write!(f, "corrupt line {line}: {msg}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

/// Append-only JSONL store for one chat session.
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    /// Open (or create) the store for session `id` under `<data_dir>/sessions/`.
    pub fn open(data_dir: &Path, id: &str) -> Self {
        let dir = data_dir.join("sessions");
        let _ = std::fs::create_dir_all(&dir);
        SessionStore {
            path: dir.join(format!("{id}.jsonl")),
        }
    }

    /// Append a single message. Exclusive lock + fsync.
    pub fn append_msg(&self, msg: &ChatMsg) -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.lock_exclusive()?;

        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        // Lock released on drop
        Ok(())
    }

    /// Load all messages from the file. Trailing corrupt line is auto-trimmed.
    pub fn load(&self) -> Result<Vec<ChatMsg>, StoreError> {
        if !self.path.exists() {
            return Ok(vec![]);
        }

        let mut file = File::open(&self.path)?;
        file.lock_shared()?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        drop(file);

        if content.is_empty() {
            return Ok(vec![]);
        }

        let lines: Vec<&str> = content.lines().collect();
        let mut messages = Vec::with_capacity(lines.len());

        for (i, line) in lines.iter().enumerate() {
            match serde_json::from_str::<ChatMsg>(line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    if i == lines.len() - 1 {
                        // Trailing corrupt line: trim, return what we have
                        self.trim_trailing_line()?;
                        break;
                    } else {
                        return Err(StoreError::CorruptLine {
                            line: i + 1,
                            msg: e.to_string(),
                        });
                    }
                }
            }
        }

        Ok(messages)
    }

    /// Delete a session file. No-op if it doesn't exist.
    pub fn delete(data_dir: &Path, id: &str) -> Result<(), StoreError> {
        let path = data_dir.join("sessions").join(format!("{id}.jsonl"));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Truncate the last (corrupt) line from the file.
    fn trim_trailing_line(&self) -> Result<(), StoreError> {
        let mut file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.lock_exclusive()?;

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let content = String::from_utf8_lossy(&buf);

        let pos = content
            .rfind('\n')
            .map(|last| content[..last].rfind('\n').map(|p| p + 1).unwrap_or(0))
            .unwrap_or(0);
        file.set_len(pos as u64)?;
        file.sync_all()?;
        Ok(())
    }
}

impl SessionStore {
    /// Test-only: expose the internal file path.
    pub fn test_path(&self) -> &Path {
        &self.path
    }
}

/// List all session IDs in `<data_dir>/sessions/`, sorted by mtime descending
/// (newest first).
pub fn list_sessions(data_dir: &Path) -> Vec<(String, std::time::SystemTime)> {
    let dir = data_dir.join("sessions");
    let mut entries = Vec::new();

    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext == "jsonl") {
                let id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                entries.push((id, mtime));
            }
        }
    }

    // newest first
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries
}

/// Daemon-backed listing. When a daemon is reachable on the configured
/// socket, return its `SessionSummary` rows ordered by `updated_at` desc
/// (the daemon already does the ordering). Falls back to an empty list on
/// connect failure so a freshly initialized workspace keeps working
/// without a daemon.
///
/// ponytail: this is the narrow compatibility shim — JSONL stays the
/// fallback for chat messages, the daemon is the canonical source for the
/// session roster when present. No JSONL-to-libSQL migration tool is
/// provided here because the TUI still reads JSONL messages locally.
pub fn list_sessions_via_daemon(cfg: &config::Config) -> Vec<session::SessionSummary> {
    let client = match daemon::DaemonClient::from_config(cfg) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match client.session_list("%", session::MAX_LIMIT) {
        Ok(rows) => rows,
        Err(_) => Vec::new(),
    }
}
