//! Persistent local memory: append-only JSONL, default-off.
//!
//! Same storage contract as `task::store`: one JSON object per line, an
//! exclusive `flock` around every write plus `fsync`, latest-wins on a
//! duplicate id, and a trailing corrupt line (torn write) auto-trimmed on
//! read. A `Memory::disabled()` handle makes every operation a no-op so
//! call sites never branch on a feature flag.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One remembered line.
///
/// `id` is assigned by [`Memory::append`] (monotonic per store, starting at
/// 1); whatever the caller puts there is overwritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: u64,
    pub ts: String,
    pub text: String,
}

impl MemoryEntry {
    /// New entry stamped with the current UTC time. `id` is filled in on append.
    pub fn new(text: impl Into<String>) -> MemoryEntry {
        MemoryEntry {
            id: 0,
            ts: now_iso(),
            text: text.into(),
        }
    }
}

/// Errors from memory operations.
#[derive(Debug)]
pub enum MemoryError {
    Io(std::io::Error),
    Json(serde_json::Error),
    CorruptLine { line: usize, msg: String },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::Io(e) => write!(f, "IO error: {e}"),
            MemoryError::Json(e) => write!(f, "JSON error: {e}"),
            MemoryError::CorruptLine { line, msg } => write!(f, "corrupt line {line}: {msg}"),
        }
    }
}

impl std::error::Error for MemoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MemoryError::Io(e) => Some(e),
            MemoryError::Json(e) => Some(e),
            MemoryError::CorruptLine { .. } => None,
        }
    }
}

impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> MemoryError {
        MemoryError::Io(e)
    }
}

impl From<serde_json::Error> for MemoryError {
    fn from(e: serde_json::Error) -> MemoryError {
        MemoryError::Json(e)
    }
}

/// Handle on the memory store. `disabled()` is the default state: every
/// method succeeds and does nothing.
#[derive(Debug, Clone)]
pub struct Memory {
    /// `None` = disabled.
    path: Option<PathBuf>,
}

impl Memory {
    /// Disabled handle: `enabled()` is false, every operation is a no-op.
    pub fn disabled() -> Memory {
        Memory { path: None }
    }

    /// Open (creating if needed) `{dir}/memory.jsonl`.
    ///
    /// Only file system failures error; an empty or missing store is fine.
    pub fn open(dir: &Path) -> Result<Memory, MemoryError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("memory.jsonl");
        OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Memory { path: Some(path) })
    }

    /// Whether this handle writes anything.
    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Append one entry. Exclusive lock held across id assignment, write and
    /// fsync, so concurrent writers cannot collide on an id.
    pub fn append(&mut self, mut entry: MemoryEntry) -> Result<(), MemoryError> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };

        // O_APPEND + read: writes always land at EOF, reads start at offset 0,
        // so one handle (and one lock) covers both the id scan and the write.
        let mut file = OpenOptions::new()
            .read(true)
            .create(true)
            .append(true)
            .open(&path)?;
        file.lock()?;

        let mut content = String::new();
        file.read_to_string(&mut content)?;
        entry.id = max_id(&content) + 1;

        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
        file.flush()?;
        file.sync_all()?;

        // Lock released on drop.
        Ok(())
    }

    /// All entries, id-sorted, latest-wins on duplicate id.
    /// A trailing corrupt line is trimmed from the file; a corrupt line in the
    /// middle is an error (that is real damage, not a torn write).
    pub fn list(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
        let Some(path) = self.path.as_deref() else {
            return Ok(vec![]);
        };
        if !path.exists() {
            return Ok(vec![]);
        }

        let mut file = File::open(path)?;
        file.lock_shared()?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        drop(file);

        let lines: Vec<&str> = content.lines().collect();
        let mut map: BTreeMap<u64, MemoryEntry> = BTreeMap::new();
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MemoryEntry>(line) {
                Ok(entry) => {
                    map.insert(entry.id, entry);
                }
                Err(_) if i == lines.len() - 1 => {
                    trim_trailing_line(path)?;
                    break;
                }
                Err(e) => {
                    return Err(MemoryError::CorruptLine {
                        line: i + 1,
                        msg: e.to_string(),
                    });
                }
            }
        }
        Ok(map.into_values().collect())
    }

    /// Entries whose text contains `query`, case-insensitive.
    /// ponytail: substring scan over the whole store; index it when the store
    /// outgrows a linear pass (thousands of entries).
    pub fn search(&self, query: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let needle = query.to_lowercase();
        let mut out = self.list()?;
        out.retain(|e| e.text.to_lowercase().contains(&needle));
        Ok(out)
    }
}

/// Highest id already stored; 0 when the store is empty or unreadable.
fn max_id(content: &str) -> u64 {
    content
        .lines()
        .filter_map(|l| serde_json::from_str::<MemoryEntry>(l).ok())
        .map(|e| e.id)
        .max()
        .unwrap_or(0)
}

/// Drop the last line of the file (a torn write).
fn trim_trailing_line(path: &Path) -> Result<(), MemoryError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    file.lock()?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let content = String::from_utf8_lossy(&buf);

    // Start of the final line, whether or not the file ends in a newline.
    let body = content.trim_end_matches('\n');
    let pos = body.rfind('\n').map(|p| p + 1).unwrap_or(0);
    file.set_len(pos as u64)?;
    file.sync_all()?;
    Ok(())
}

/// ISO-8601-ish UTC timestamp, seconds precision.
/// ponytail: duplicated from `task::now_iso` on purpose — this crate stays
/// dependency-free apart from serde; fold both into one crate if a third
/// caller needs it.
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Memory, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mem = Memory::open(tmp.path()).expect("open");
        (mem, tmp)
    }

    #[test]
    fn disabled_is_noop() {
        let mut mem = Memory::disabled();
        assert!(!mem.enabled());
        mem.append(MemoryEntry::new("secret"))
            .expect("no-op append");
        assert!(mem.list().expect("no-op list").is_empty());
        assert!(mem.search("secret").expect("no-op search").is_empty());
    }

    #[test]
    fn append_then_list_round_trip() {
        let (mut mem, _tmp) = store();
        assert!(mem.enabled());
        mem.append(MemoryEntry::new("user prefers tabs")).unwrap();
        mem.append(MemoryEntry::new("deploy target is fly.io"))
            .unwrap();

        let all = mem.list().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].text, "user prefers tabs");
        assert_eq!(all[1].text, "deploy target is fly.io");
        assert!(
            all[0].ts.ends_with('Z'),
            "ts should be ISO-ish: {}",
            all[0].ts
        );
    }

    #[test]
    fn ids_are_monotonic_across_handles() {
        let (mut mem, tmp) = store();
        mem.append(MemoryEntry::new("one")).unwrap();
        mem.append(MemoryEntry::new("two")).unwrap();
        // A fresh handle keeps counting from the stored max, not from 1.
        let mut again = Memory::open(tmp.path()).unwrap();
        again.append(MemoryEntry::new("three")).unwrap();

        let ids: Vec<u64> = again.list().unwrap().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn caller_supplied_id_is_ignored() {
        let (mut mem, _tmp) = store();
        let mut entry = MemoryEntry::new("first");
        entry.id = 999;
        mem.append(entry).unwrap();
        assert_eq!(mem.list().unwrap()[0].id, 1);
    }

    #[test]
    fn search_is_case_insensitive_substring() {
        let (mut mem, _tmp) = store();
        mem.append(MemoryEntry::new("Prefers Ripgrep over grep"))
            .unwrap();
        mem.append(MemoryEntry::new("uses zsh")).unwrap();

        assert_eq!(mem.search("RIPGREP").unwrap().len(), 1);
        assert_eq!(mem.search("zsh").unwrap()[0].text, "uses zsh");
        assert!(mem.search("nothing here").unwrap().is_empty());
        // Empty query matches everything.
        assert_eq!(mem.search("").unwrap().len(), 2);
    }

    #[test]
    fn trailing_corrupt_line_is_trimmed() {
        let (mut mem, tmp) = store();
        mem.append(MemoryEntry::new("good")).unwrap();
        let path = tmp.path().join("memory.jsonl");
        // Torn write: partial line, no trailing newline.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"id\":2,\"ts\":\"tor").unwrap();
        drop(f);

        let all = mem.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "good");
        // The garbage is gone from disk, so the next append is clean.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 1, "trailing garbage left: {raw:?}");
        assert!(!raw.contains("\"id\":2"), "trailing garbage left: {raw:?}");
        mem.append(MemoryEntry::new("after")).unwrap();
        assert_eq!(mem.list().unwrap().len(), 2);
    }

    #[test]
    fn corrupt_middle_line_is_an_error() {
        let (mem, tmp) = store();
        let path = tmp.path().join("memory.jsonl");
        std::fs::write(&path, "not json\n{\"id\":1,\"ts\":\"t\",\"text\":\"ok\"}\n").unwrap();
        match mem.list() {
            Err(MemoryError::CorruptLine { line, .. }) => assert_eq!(line, 1),
            other => panic!("expected CorruptLine, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_id_latest_wins() {
        let (mem, tmp) = store();
        let path = tmp.path().join("memory.jsonl");
        std::fs::write(
            &path,
            "{\"id\":1,\"ts\":\"t\",\"text\":\"old\"}\n{\"id\":1,\"ts\":\"t\",\"text\":\"new\"}\n",
        )
        .unwrap();
        let all = mem.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "new");
    }

    #[test]
    fn empty_store_lists_nothing() {
        let (mem, _tmp) = store();
        assert!(mem.list().unwrap().is_empty());
    }
}
