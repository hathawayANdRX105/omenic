//! JSONL append-only task store: fcntl lock + latest-wins + auto-trim.
//!
//! Port of compass-ws/dev/bin/cx/store.py.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::task::Task;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{TaskKind, TaskStatus};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_task(id: &str, title: &str) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            kind: TaskKind::Task,
            priority: 2,
            status: TaskStatus::Open,
            attempts: 0,
            parent: None,
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn temp_dir() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("omenic_store_test_{}_{}", std::process::id(), ts));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_then_load() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        let task = make_task("t1", "first task");
        store.append(&task).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "t1");
        assert_eq!(loaded[0].title, "first task");
        assert_eq!(loaded[0].kind, TaskKind::Task);
        assert_eq!(loaded[0].status, TaskStatus::Open);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn latest_wins() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("t1", "original")).unwrap();
        store.append(&make_task("t1", "updated")).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "t1");
        assert_eq!(loaded[0].title, "updated");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn multiple_tasks() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("a", "alpha")).unwrap();
        store.append(&make_task("b", "bravo")).unwrap();
        store.append(&make_task("c", "charlie")).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(
            loaded.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn trailing_corrupt_line() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("t1", "good")).unwrap();

        // Write a corrupt second line
        let mut f = OpenOptions::new().append(true).open(&store.path).unwrap();
        writeln!(f, "not-json").unwrap();
        drop(f);

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "good");

        // File should be trimmed — append should succeed again
        store.append(&make_task("t2", "after trim")).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 2);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_file() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        let loaded = store.load_all().unwrap();
        assert!(loaded.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_existing_file() {
        let dir = temp_dir();
        let path = dir.join("tasks.jsonl");
        fs::write(&path, "").unwrap();
        let store = Store::new(&dir);
        let loaded = store.load_all().unwrap();
        assert!(loaded.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_task_by_id() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("t1", "first")).unwrap();
        store.append(&make_task("t2", "second")).unwrap();

        let t = store.load_task("t1").unwrap();
        assert!(t.is_some());
        assert_eq!(t.unwrap().title, "first");

        let none = store.load_task("nonexistent").unwrap();
        assert!(none.is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compact_dedups_duplicates() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("a", "old")).unwrap();
        store.append(&make_task("b", "b-title")).unwrap();
        store.append(&make_task("a", "new")).unwrap();

        let before_lines = fs::read_to_string(&store.path).unwrap().lines().count();
        assert_eq!(before_lines, 3);

        store.compact().unwrap();

        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 2);
        let a = after.iter().find(|t| t.id == "a").unwrap();
        assert_eq!(a.title, "new");

        let after_lines = fs::read_to_string(&store.path).unwrap().lines().count();
        assert_eq!(after_lines, 2);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compact_sorts_by_id() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("c", "c")).unwrap();
        store.append(&make_task("a", "a")).unwrap();
        store.append(&make_task("b", "b")).unwrap();

        store.compact().unwrap();

        let content = fs::read_to_string(&store.path).unwrap();
        let mut ids = Vec::new();
        for line in content.lines() {
            ids.push(serde_json::from_str::<Task>(line).unwrap().id);
        }
        assert_eq!(ids, vec!["a", "b", "c"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compact_on_missing_file() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.compact().unwrap();
        assert!(store.load_all().unwrap().is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compact_no_temp_left() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("a", "x")).unwrap();
        store.compact().unwrap();
        let tmp = store.path.with_extension("jsonl.tmp");
        assert!(!tmp.exists(), "temp file should not remain: {:?}", tmp);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compact_then_append() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("a", "v1")).unwrap();
        store.append(&make_task("a", "v2")).unwrap();
        store.compact().unwrap();
        store.append(&make_task("a", "v3")).unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "v3");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tombstone_hides_task() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("t1", "first")).unwrap();
        store.append(&make_task("t2", "second")).unwrap();
        store.append_tombstone("t1").unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "t2");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tombstone_after_compact() {
        let dir = temp_dir();
        let store = Store::new(&dir);
        store.append(&make_task("t1", "first")).unwrap();
        store.append_tombstone("t1").unwrap();
        store.compact().unwrap();

        let all = store.load_all().unwrap();
        assert!(all.is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn concurrent_append_latest_wins() {
        // #43: parallel appends under flock must not interleave corrupt
        // lines, and latest-wins resolves the winner deterministically.
        let dir = temp_dir();
        let store = Store::new(&dir);
        let mut handles = Vec::new();
        for i in 0..8 {
            let dir = dir.clone();
            handles.push(std::thread::spawn(move || {
                let s = Store::new(&dir);
                for j in 0..25 {
                    s.append(&make_task("shared", &format!("w{i}-{j}")))
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // 200 lines, every line valid JSON (no interleaving), latest wins.
        assert_eq!(
            fs::read_to_string(&store.path).unwrap().lines().count(),
            200
        );
        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "shared");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_task_and_dep_semantics() {
        // #43: loading a missing id is None; a dep referencing a missing
        // task simply never matches Done (graph gate owns the decision).
        let dir = temp_dir();
        let store = Store::new(&dir);
        assert!(store.load_task("ghost").unwrap().is_none());

        let mut t = make_task("t1", "has ghost dep");
        t.deps = vec!["ghost".to_string()];
        store.append(&t).unwrap();
        let loaded = store.load_task("t1").unwrap().unwrap();
        assert_eq!(loaded.deps, vec!["ghost"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compact_empty_file_stays_empty_and_appendable() {
        // #43: compact on an empty existing file keeps it empty; append
        // after compact works and survives another compact.
        let dir = temp_dir();
        let path = dir.join("tasks.jsonl");
        fs::write(&path, "").unwrap();
        let store = Store::new(&dir);
        store.compact().unwrap();
        assert!(store.load_all().unwrap().is_empty());

        store.append(&make_task("post", "after compact")).unwrap();
        store.compact().unwrap();
        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "post");

        fs::remove_dir_all(&dir).ok();
    }
}
