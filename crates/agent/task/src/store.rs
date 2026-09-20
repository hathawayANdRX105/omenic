//! JSONL append-only stores: fcntl lock + latest-wins + auto-trim.
//!
//! Port of compass-ws/dev/bin/cx/store.py.
//!
//! One [`Store`] handle per data dir owns three independent jsonl files
//! (`tasks.jsonl`, `todos.jsonl`, `goals.jsonl`). Every append takes an
//! exclusive `flock` and `fsync`s before release; every load takes a shared
//! lock. Duplicate ids collapse to the last appended line, so appending a
//! record with a known id *is* the update path — no read-modify-write round
//! trip needed.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::Task;
use crate::goal::Goal;
use crate::todo::Todo;

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

/// JSONL append-only store rooted at a data dir.
///
/// Thread-safe via OS-level file locking (fcntl flock).
/// Latest-wins on duplicate id; trailing corrupt lines are auto-trimmed.
pub struct Store {
    data_dir: PathBuf,
}
#[allow(dead_code)] // consumed by CLI layer in M1.8
impl Store {
    /// Create a store rooted at `data_dir` (`tasks.jsonl` / `todos.jsonl` /
    /// `goals.jsonl` live directly inside it).
    pub fn new(data_dir: &Path) -> Self {
        // Ensure the data dir exists so the first append works even when the
        // directory was never created explicitly (e.g. fresh CLI run).
        let _ = std::fs::create_dir_all(data_dir);
        Store {
            data_dir: data_dir.to_path_buf(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.data_dir.join(name)
    }

    /// Take an exclusive lock and append one pre-serialized line, fsyncing
    /// before the lock releases. Used by every append path below.
    fn append_line(&self, path: &Path, line: &str) -> Result<(), StoreError> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.lock_exclusive()?;

        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;

        // Lock released on drop
        Ok(())
    }

    /// Shared-lock read of a jsonl file. `None` when the file does not exist
    /// or is empty.
    fn read_locked(&self, path: &Path) -> Result<Option<String>, StoreError> {
        if !path.exists() {
            return Ok(None);
        }
        let mut file = File::open(path)?;
        file.lock_shared()?;

        let mut content = String::new();
        file.read_to_string(&mut content)?;

        // Lock released on drop
        drop(file);

        if content.is_empty() {
            Ok(None)
        } else {
            Ok(Some(content))
        }
    }

    /// Handle a line that did not parse into a record: if it is the trailing
    /// line, trim it and let the caller keep the records parsed so far;
    /// otherwise surface it as a corrupt-line error.
    fn corrupt_or_trim(
        &self,
        path: &Path,
        i: usize,
        last: usize,
        msg: String,
    ) -> Result<(), StoreError> {
        if i == last {
            self.trim_trailing_line(path)
        } else {
            Err(StoreError::CorruptLine { line: i + 1, msg })
        }
    }

    /// Load every record of `name`, latest-wins on duplicate id and sorted by
    /// id for determinism. Tombstone lines (`{"id":..,"tombstone":true}`)
    /// remove the record. A trailing corrupt line is trimmed, not fatal.
    fn load_records<T: serde::de::DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Vec<T>, StoreError> {
        let path = self.path(name);
        let Some(content) = self.read_locked(&path)? else {
            return Ok(vec![]);
        };

        let lines: Vec<&str> = content.lines().collect();
        let last = lines.len().saturating_sub(1);
        let mut map: HashMap<String, T> = HashMap::with_capacity(lines.len());

        for (i, line) in lines.iter().enumerate() {
            // Tombstone lines: {"id":"<id>","tombstone":true} — remove the record.
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
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => {
                    // Take the id before the record moves; a shape that
                    // lacks one is reported instead of silently dropped.
                    let id = v.get("id").and_then(|i| i.as_str()).map(str::to_string);
                    match (id, serde_json::from_value::<T>(v)) {
                        (Some(id), Ok(record)) => {
                            map.insert(id, record);
                        }
                        (Some(_), Err(e)) => self.corrupt_or_trim(&path, i, last, e.to_string())?,
                        (None, _) => self.corrupt_or_trim(
                            &path,
                            i,
                            last,
                            "record has no id field".to_string(),
                        )?,
                    }
                }
                Err(e) => self.corrupt_or_trim(&path, i, last, e.to_string())?,
            }
        }

        let mut records: Vec<(String, T)> = map.into_iter().collect();
        records.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(records.into_iter().map(|(_, r)| r).collect())
    }

    // --- tasks ---------------------------------------------------------------

    /// Append a task line. Exclusive lock held during write + fsync.
    pub fn append(&self, task: &Task) -> Result<(), StoreError> {
        let line = serde_json::to_string(task)?;
        self.append_line(&self.path("tasks.jsonl"), &line)
    }

    /// Append a tombstone line to mark a task as deleted.
    /// Tombstone format: {"id":"<id>","tombstone":true}
    pub fn append_tombstone(&self, id: &str) -> Result<(), StoreError> {
        let tombstone = serde_json::json!({"id": id, "tombstone": true});
        let line = serde_json::to_string(&tombstone)
            .unwrap_or_else(|_| format!(r#"{{"id":"{id}","tombstone":true}}"#));
        self.append_line(&self.path("tasks.jsonl"), &line)
    }

    /// Load all tasks; latest-wins on duplicate id.
    /// Result sorted by id for determinism.
    pub fn load_all(&self) -> Result<Vec<Task>, StoreError> {
        self.load_records("tasks.jsonl")
    }

    // --- todos ---------------------------------------------------------------

    /// Append a todo line. Same lock + fsync + latest-wins semantics as
    /// [`Store::append`]: appending a todo whose id already exists updates it.
    pub fn append_todo(&self, todo: &Todo) -> Result<(), StoreError> {
        let line = serde_json::to_string(todo)?;
        self.append_line(&self.path("todos.jsonl"), &line)
    }

    /// Load all todos; latest-wins on duplicate id, sorted by id.
    pub fn load_todos(&self) -> Result<Vec<Todo>, StoreError> {
        self.load_records("todos.jsonl")
    }

    // --- goals ---------------------------------------------------------------

    /// Append a goal line. Same lock + fsync + latest-wins semantics as
    /// [`Store::append`].
    pub fn append_goal(&self, goal: &Goal) -> Result<(), StoreError> {
        let line = serde_json::to_string(goal)?;
        self.append_line(&self.path("goals.jsonl"), &line)
    }

    /// Load all goals; latest-wins on duplicate id, sorted by id.
    pub fn load_goals(&self) -> Result<Vec<Goal>, StoreError> {
        self.load_records("goals.jsonl")
    }

    // --- by id ---------------------------------------------------------------

    /// Load a single task by id.
    pub fn load_task(&self, id: &str) -> Result<Option<Task>, StoreError> {
        // ponytail: load_all is fine for MVP; O(n) but trivially correct
        let tasks = self.load_all()?;
        Ok(tasks.into_iter().find(|t| t.id == id))
    }

    /// Truncate the last (corrupt) line from the file.
    fn trim_trailing_line(&self, path: &Path) -> Result<(), StoreError> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
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
        let tmp = self.path("tasks.jsonl.tmp");

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
        std::fs::rename(&tmp, &self.path("tasks.jsonl"))?;
        Ok(())
    }
}
