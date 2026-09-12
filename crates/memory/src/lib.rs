//! Persistent local memory: append-only JSONL, default-off.
//!
//! Same storage contract as `task::store`: one JSON object per line, an
//! exclusive `flock` around every write plus `fsync`, latest-wins on a
//! duplicate id, and a trailing corrupt line (torn write) auto-trimmed on
//! read. A `Memory::disabled()` handle makes every operation a no-op so
//! call sites never branch on a feature flag.
//!
//! Entries are the single source of truth; the derived graph
//! ([`graph::MemoryGraph`]) and the recall pipeline ([`recall`]) are
//! rebuildable views over them (jcode `jcode-memory-types` lineage).

pub mod graph;
pub mod recall;

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use graph::MemoryGraph;
pub use recall::RecallHit;

/// Who asserted this memory. Trust never decays on its own (jcode: High =
/// user said it, Medium = observed, Low = inferred).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    High,
    #[default]
    Medium,
    Low,
}

/// What kind of fact this is. Half-life decay per category (jcode:
/// Correction 365d / Preference 90d / Entity 60d / Fact 30d) lands with the
/// write pipeline; the enum is fixed now so stored entries survive it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Fact,
    Preference,
    Entity,
    Correction,
    #[default]
    Custom,
}

/// One remembered line.
///
/// `id` is assigned by [`Memory::append`] (monotonic per store, starting at
/// 1); whatever the caller puts there is overwritten. New fields all carry
/// `#[serde(default)]` so stores written by the old `{id, ts, text}` shape
/// keep loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: u64,
    pub ts: String,
    pub text: String,
    #[serde(default)]
    pub category: Category,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trust: Trust,
    /// 0.0–1.0 relevance confidence; half-life decay is a write-pipeline job.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// How often this memory was reinforced (re-derived, re-affirmed).
    #[serde(default)]
    pub strength: u32,
    /// Soft delete: superseded entries stay for history and graph walks.
    #[serde(default = "default_true")]
    pub active: bool,
    /// The entry that replaced this one, if any.
    #[serde(default)]
    pub superseded_by: Option<u64>,
    /// An entry whose statement conflicts with this one.
    #[serde(default)]
    pub contradicts: Option<u64>,
}

fn default_confidence() -> f32 {
    0.5
}

fn default_true() -> bool {
    true
}

impl MemoryEntry {
    /// New entry stamped with the current UTC time. `id` is filled in on append.
    pub fn new(text: impl Into<String>) -> MemoryEntry {
        MemoryEntry {
            id: 0,
            ts: now_iso(),
            text: text.into(),
            category: Category::Custom,
            tags: Vec::new(),
            trust: Trust::Medium,
            confidence: default_confidence(),
            strength: 0,
            active: true,
            superseded_by: None,
            contradicts: None,
        }
    }

    /// The derived-graph view over these entries.
    pub fn graph(entries: &[MemoryEntry]) -> MemoryGraph {
        MemoryGraph::build(entries)
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

    /// Append one entry. Exclusive lock held across id assignment, torn-line
    /// repair, write and fsync, so concurrent writers cannot collide on an id
    /// nor glue a new entry onto a half-written one.
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

        // Bytes, not `read_to_string`: a torn write can split a multi-byte
        // char, and lossy decoding keeps that recoverable instead of failing
        // the whole append with InvalidData.
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        // Heal a torn trailing line under this same lock, so the new entry is
        // never appended onto a partial one.
        if buf.last().is_some_and(|&b| b != b'\n') {
            let pos = buf.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
            file.set_len(pos as u64)?;
            buf.truncate(pos);
        }

        entry.id = max_id(&String::from_utf8_lossy(&buf)) + 1;

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
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        drop(file);
        // Lossy: a torn multi-byte char must not fail the whole read.
        let content = String::from_utf8_lossy(&buf);

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

    /// Derived graph over the current store (rebuilt on every call — the
    /// JSONL stays the only persistent state).
    pub fn graph_view(&self) -> Result<MemoryGraph, MemoryError> {
        Ok(MemoryGraph::build(&self.list()?))
    }

    /// Top-`k` entries for `query`: direct idf-weighted match plus the graph
    /// cascade. See [`recall::recall`].
    pub fn recall(&self, query: &str, k: usize) -> Result<Vec<RecallHit>, MemoryError> {
        Ok(recall::recall(&self.graph_view()?, query, k))
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

    // Byte offsets, not offsets into a lossy string: each invalid byte
    // widens to U+FFFD, which would shift `pos` and truncate mid-line.
    let end = buf.iter().rposition(|&b| b != b'\n').map_or(0, |p| p + 1);
    let pos = buf[..end]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
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
