//! Id-agnostic append-only JSONL file mechanics shared by `store` (work items:
//! tasks / todos / goals) and `memory` (persistent memory entries).
//!
//! Only the low-level IO primitives live here: exclusive-flock append +
//! `fsync`, shared-flock lossy read, a byte-based torn-line cut point, and an
//! unconditional last-line drop. The record-key type (String task titles in
//! `store`, u64 counters in `memory`), dedup/ordering, tombstones,
//! compaction and auto-id assignment stay in the owning crate — those are
//! data semantics, not IO mechanics.
//!
//! Byte math, not string math: a torn multi-byte char lossy-decodes to a
//! wider U+FFFD, which would shift a string offset and truncate mid-line.
//! The cut point below operates on the raw bytes so the heal is correct for
//! any input.
//!
//! No cycle: `memory` depends on `store` for this module; `store` never
//! depends on `memory`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use fs2::FileExt;

/// Exclusive-locked append of one pre-serialized JSON line. The caller passes
/// the line WITHOUT a trailing newline; `\n` is written under the same lock so
/// the record and its terminator stay atomic. `fsync`s before the lock
/// releases.
pub fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.lock_exclusive()?;

    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_all()?;

    // Lock released on drop.
    Ok(())
}

/// Shared-locked read of a JSONL file as a lossy string. `None` when the file
/// is missing or empty.
pub fn read_lines(path: &Path) -> std::io::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut file = File::open(path)?;
    file.lock_shared()?;

    let mut buf: Vec<u8> = Vec::new();
    file.read_to_end(&mut buf)?;

    // Lock released on drop.
    drop(file);

    if buf.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }
}

/// Byte offset to cut to when the final line must be dropped: one past the
/// last `\n`, i.e. the start of the final line (0 when there is no `\n` to cut
/// back to, so a single torn line trims to empty). One trailing `\n` is
/// stripped from the search so a complete final line and a mid-write crash
/// (record bytes landed, the `\n` did not) share one cut point.
///
/// Pure byte math; the caller holds the exclusive lock when it applies the
/// cut via [`set_len`] / a truncating write.
///
/// [`set_len`]: std::fs::File::set_len
pub fn last_line_start(bytes: &[u8]) -> usize {
    let body_len = bytes.len() - (bytes.last() == Some(&b'\n')) as usize;
    bytes[..body_len]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1)
}

/// Unconditionally drop the final line of `path`, trimming to
/// [`last_line_start`]. Callers use this only after establishing the last line
/// is corrupt, so a clean trailing line is dropped solely as the tail of a
/// damaged file. Exclusive-locked and `fsync`ed.
pub fn drop_last_line(path: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    file.lock_exclusive()?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let pos = last_line_start(&buf);
    if pos < buf.len() {
        file.set_len(pos as u64)?;
        file.sync_all()?;
    }

    // Lock released on drop.
    Ok(())
}
