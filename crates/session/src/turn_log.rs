use crate::SessionError;

use serde::{Deserialize, Serialize};

/// One durable run-boundary record of a session's turn log.  The wire tags
/// reuse the frozen orbit `AgentEvent` vocabulary from R2 3.1: `turn_start`
/// / `turn_end` + stop-reason strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnRecord {
    /// A run began; the log is open for `run_id` until its [`TurnRecord::TurnEnd`].
    TurnStart { run_id: String, ts_ms: i64 },
    /// A run finished with `status` (`"ok"`, `"failed"`, `"aborted"`, ...).
    TurnEnd {
        run_id: String,
        ts_ms: i64,
        status: String,
    },
}

/// Status written on synthetic closers by [`interrupted_run_closers`].
pub const ABORTED: &str = "aborted";

/// Crash repair for a persisted turn log (the dsh `interruptedTurnClosers`
/// pattern at run granularity): scan an ordered log and return synthetic
/// [`TurnRecord::TurnEnd`] records with status `"aborted"` for every run that
/// has a `TurnStart` but no `TurnEnd`.
///
/// Pure and lossless — the input is never mutated and existing records are
/// never dropped or rewritten; the returned closers only ever *continue* the
/// log.  Closer timestamps reuse the last log record's time (a crash knows
/// nothing newer), and closers follow start order so encoding is
/// deterministic.  A balanced (or empty) log yields no closers, making the
/// repair idempotent.
pub fn interrupted_run_closers(records: &[TurnRecord]) -> Vec<TurnRecord> {
    // (run_id, start ts) in start order; ends remove, starts append.
    let mut open: Vec<(&String, i64)> = Vec::new();
    for record in records {
        match record {
            TurnRecord::TurnStart { run_id, ts_ms } => open.push((run_id, *ts_ms)),
            TurnRecord::TurnEnd { run_id, .. } => {
                // Close the innermost open instance; nested starts of the
                // same id (restarts) each need their own closer.
                if let Some(pos) = open.iter().rposition(|(r, _)| *r == run_id) {
                    open.swap_remove(pos);
                }
            }
        }
    }
    let Some(last) = records.last() else {
        return Vec::new();
    };
    let ts_ms = match last {
        TurnRecord::TurnStart { ts_ms, .. } | TurnRecord::TurnEnd { ts_ms, .. } => *ts_ms,
    };
    // `swap_remove` above shuffles open order; sort by (start ts, run id)
    // for deterministic closers.
    open.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    open.into_iter()
        .map(|(run_id, _)| TurnRecord::TurnEnd {
            run_id: run_id.clone(),
            ts_ms,
            status: ABORTED.to_string(),
        })
        .collect()
}

/// Encode a turn log losslessly to JSONL (one record per line, `\n`-terminated).
pub fn encode_turn_log(records: &[TurnRecord]) -> String {
    let mut out = String::new();
    for record in records {
        // TurnRecord is plain JSON-safe data; serialization cannot fail here.
        out.push_str(&serde_json::to_string(record).expect("encode TurnRecord"));
        out.push('\n');
    }
    out
}

/// Decode a JSONL turn log.  Any line that is neither a valid record nor
/// empty fails the whole read with [`SessionError::MalformedTurnLog`] —
/// repair never runs against a log it cannot fully trust.
pub fn decode_turn_log(text: &str) -> Result<Vec<TurnRecord>, SessionError> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<TurnRecord>(line).map_err(|e| {
            SessionError::MalformedTurnLog {
                line: i + 1,
                reason: e.to_string(),
            }
        })?;
        out.push(record);
    }
    Ok(out)
}
