//! JSONL append-only task store: fcntl lock + latest-wins + auto-trim.
//!
//! Port of compass-ws/dev/bin/cx/store.py.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::Task;

/// Errors from store operations.
#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    CorruptLine { line: usize, msg: String },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "IO error: {e}"),
            StoreError::Json(e) => write!(f, "JSON error: {e}"),
            StoreError::CorruptLine { line, msg } => {
                write!(f, "corrupt line {line}: {msg}")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Io(e) => Some(e),
            StoreError::Json(e) => Some(e),
            StoreError::CorruptLine { .. } => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> StoreError {
        StoreError::Io(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> StoreError {
        StoreError::Json(e)
    }
}

/// JSONL append-only task store.
///
/// Thread-safe via OS-level file locking (fcntl flock).
/// Latest-wins on duplicate id; trailing corrupt lines are auto-trimmed.
pub struct Store {
    path: PathBuf,
}
#[allow(dead_code)] // consumed by CLI layer in M1.8
impl Store {
    /// Create a store rooted at `data_dir/tasks.jsonl`.
    pub fn new(data_dir: &Path) -> Self {
        // Ensure the data dir exists so the first append works even when the
        // directory was never created explicitly (e.g. fresh CLI run).
        let _ = std::fs::create_dir_all(data_dir);
        Store {
            path: data_dir.join("tasks.jsonl"),
        }
    }

    /// Append a task line. Exclusive lock held during write + fsync.
    pub fn append(&self, task: &Task) -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.lock_exclusive()?;

        let mut line = serde_json::to_string(task)?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
        file.flush()?;
        file.sync_all()?;

        // Lock released on drop
        Ok(())
    }

    /// Append a tombstone line to mark a task as deleted.
    /// Tombstone format: {"id":"<id>","tombstone":true}
    pub fn append_tombstone(&self, id: &str) -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.lock_exclusive()?;

        let tombstone = serde_json::json!({"id": id, "tombstone": true});
        let line = serde_json::to_string(&tombstone)
            .unwrap_or_else(|_| format!(r#"{{"id":"{id}","tombstone":true}}"#));
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;

        // Lock released on drop
        Ok(())
    }

    /// Load all tasks; latest-wins on duplicate id.
    /// Result sorted by id for determinism.
    pub fn load_all(&self) -> Result<Vec<Task>, StoreError> {
        if !self.path.exists() {
            return Ok(vec![]);
        }

        let mut file = File::open(&self.path)?;
        file.lock_shared()?;

        let mut content = String::new();
        file.read_to_string(&mut content)?;

        // Lock released on drop
        drop(file);

        if content.is_empty() {
            return Ok(vec![]);
        }

        let lines: Vec<&str> = content.lines().collect();
        let mut map: HashMap<String, Task> = HashMap::with_capacity(lines.len());

        for (i, line) in lines.iter().enumerate() {
            // Tombstone lines: {"id":"<id>","tombstone":true} — remove the task.
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
                && v.get("tombstone")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false)
            {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    map.remove(id);
                }
                continue;
            }
            match serde_json::from_str::<Task>(line) {
                Ok(task) => {
                    map.insert(task.id.clone(), task);
                }
                Err(e) => {
                    if i == lines.len() - 1 {
                        // Trailing corrupt line: trim it, return what we have
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

        let mut tasks: Vec<Task> = map.into_values().collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }

    /// Load a single task by id.
    pub fn load_task(&self, id: &str) -> Result<Option<Task>, StoreError> {
        // ponytail: load_all is fine for MVP; O(n) but trivially correct
        let tasks = self.load_all()?;
        Ok(tasks.into_iter().find(|t| t.id == id))
    }

    /// Truncate the last (corrupt) line from the file.
    fn trim_trailing_line(&self) -> Result<(), StoreError> {
        let mut file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.lock_exclusive()?;

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let content = String::from_utf8_lossy(&buf);

        // Keep everything up to and including the second-to-last newline,
        // which drops the final (corrupt) line regardless of trailing `\n`.
        let pos = content
            .rfind('\n')
            .map(|last| content[..last].rfind('\n').map(|p| p + 1).unwrap_or(0))
            .unwrap_or(0);
        file.set_len(pos as u64)?;
        file.sync_all()?;
        Ok(())
    }

    /// Atomically rewrite the store: deduplicate by id (latest wins) and
    /// write tasks id-sorted to a temp file, then rename over the original.
    pub fn compact(&self) -> Result<(), StoreError> {
        let tasks = self.load_all()?;
        let tmp = self.path.with_extension("jsonl.tmp");

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        file.lock_exclusive()?;

        for task in &tasks {
            let mut line = serde_json::to_string(task)?;
            line.push('\n');
            file.write_all(line.as_bytes())?;
        }
        file.flush()?;
        file.sync_all()?;

        // Lock released on drop, then atomically replace the original.
        drop(file);
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
