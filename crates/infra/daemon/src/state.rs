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
    #[serde(default)]
    pub seq: i64,
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
    next_seq: i64,
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

        // Initialize next_seq based on max existing seq
        let mut max_seq = 0i64;
        for run in &runs {
            if run.seq > max_seq {
                max_seq = run.seq;
            }
        }
        let inner = RunLedgerInner {
            path,
            runs,
            next_seq: max_seq + 1,
        };

        Ok(RunLedger {
            inner: Arc::new(Mutex::new(inner)),
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
        let mut inner = self.inner.lock().expect("run ledger poisoned");
        let seq = inner.next_seq;
        inner.next_seq += 1;
        let record = RunRecord {
            seq,
            run_id: run_id.into(),
            session_id: session_id.into(),
            started_at_ms,
            finished_at_ms: None,
            status: None,
        };
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

    /// Aggregate the ledger into stats for `range` (see
    /// [`parse_stats_range`]).  `now_ms` is the aggregation reference time;
    /// dispatch passes [`now_ms()`].  Holds the lock only long enough to
    /// snapshot, then aggregates outside it.
    pub fn stats(&self, range: &str, now_ms: i64) -> StatsSummary {
        let window = parse_stats_range(range);
        // Snapshot floor is the *earlier* of the current and previous window
        // start: aggregation counts the current window but also reads the
        // previous one for the delta chips (see `prev_start` in
        // `aggregate_stats`). Snapshotting only the current window would make
        // every delta read "no comparable window". `All` needs every record
        // anyway so its buckets can start at the earliest run.
        let floor = match window {
            StatsWindow::Last(w) => now_ms - 2 * w,
            StatsWindow::All => i64::MIN,
        };
        let runs = self.snapshot_since(floor);
        aggregate_stats(&runs, range, now_ms)
    }

    /// Snapshot runs that started at or after `floor_ms`, filtering under the
    /// lock so only the records the caller actually needs are cloned.
    pub(crate) fn snapshot_since(&self, floor_ms: i64) -> Vec<RunRecord> {
        let inner = self.inner.lock().expect("run ledger poisoned");
        inner
            .runs
            .iter()
            .filter(|r| r.started_at_ms >= floor_ms)
            .cloned()
            .collect()
    }

    /// Read runs from cursor position (exclusive). Returns (remaining_runs, next_cursor).
    pub fn read_from_cursor(&self, cursor: i64) -> (Vec<RunRecord>, i64) {
        let inner = self.inner.lock().expect("run ledger poisoned");
        let mut remaining = Vec::new();
        for run in &inner.runs {
            if run.seq > cursor {
                remaining.push(run.clone());
            }
        }
        let next_cursor = if inner.runs.is_empty() {
            cursor
        } else {
            inner.runs.last().unwrap().seq
        };
        (remaining, next_cursor)
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

// ---------------------------------------------------------------------------
// Stats aggregation (G5)
// ---------------------------------------------------------------------------

/// How many buckets the throughput series is split into.  Fixed so the
/// chart width is stable across ranges.
pub const STATS_BUCKETS: usize = 12;

/// Metric keys the stats page asks for but that **no persisted omenic data
/// backs today**.  Returned verbatim in [`StatsSummary::unavailable`] so the
/// UI can hide those cards instead of rendering a fabricated zero.
///
/// Why each is missing:
///
/// * `tokens` / `cost` / `cache` — token metering is C8 (deferred).  Neither
///   [`RunRecord`] nor `session::SessionMessage` carries a token or price
///   column, so any number here would be invented.
/// * `ttft` — no first-token timestamp is recorded; only run start/finish.
/// * `agent_token_split` — no per-agent (main vs subagent) attribution
///   exists in the ledger.
/// * `model` / `provider` — runs are not tagged with the model that served
///   them.
pub const STATS_UNAVAILABLE: &[&str] = &[
    "tokens",
    "cost",
    "cache",
    "ttft",
    "agent_token_split",
    "model",
    "provider",
];

/// The time window a stats query covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsWindow {
    /// Trailing window of N milliseconds ending at "now".
    Last(i64),
    /// Everything on record (start = earliest run).
    All,
}

/// Parse a UI range token (`"1h"`, `"24h"`, `"7d"`, `"30d"`, `"90d"`,
/// `"All"`) into a window.  Unknown tokens fall back to 24h so a stale
/// client can never make the daemon error out.
/// Parse the stats range token. Unknown input is deliberately mapped to the
/// 24h window rather than rejected: the only caller is the stats page, which
/// emits this fixed chip set, so an unrecognised token means a caller bug, and
/// serving the default window keeps the page usable instead of erroring. The
/// response echoes the request token verbatim in `StatsSummary::range`, so the
/// UI never shows a label that disagrees with the numbers behind it.
pub fn parse_stats_range(range: &str) -> StatsWindow {
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    match range.trim().to_ascii_lowercase().as_str() {
        "1h" => StatsWindow::Last(HOUR),
        "24h" => StatsWindow::Last(DAY),
        "7d" => StatsWindow::Last(7 * DAY),
        "30d" => StatsWindow::Last(30 * DAY),
        "90d" => StatsWindow::Last(90 * DAY),
        "all" => StatsWindow::All,
        _ => StatsWindow::Last(DAY),
    }
}

/// One point of the throughput series: how many runs started inside
/// `[start_ms, end_ms)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsBucket {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Axis label, `HH:MM` (UTC) for sub-day buckets else `MM-DD`.
    pub label: String,
    pub runs: u64,
}

/// One row of the "recent runs" feed.  Only fields the ledger actually
/// records — no model, provider or cost (see [`STATS_UNAVAILABLE`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsRecentRun {
    pub run_id: String,
    pub session_id: String,
    pub started_at_ms: i64,
    /// `finished_at_ms - started_at_ms`; `None` while the run is in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Terminal status from the ledger, or `"running"` for a half-open run.
    pub status: String,
}

/// Aggregated run statistics for one range.  Every number here is derived
/// from real [`RunRecord`] rows; anything that would need token metering is
/// absent by design and listed in `unavailable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsSummary {
    /// Echo of the requested range token.
    pub range: String,
    /// Aggregation reference time (window end).
    pub now_ms: i64,
    /// Inclusive window start.  For [`StatsWindow::All`] this is the
    /// earliest recorded run (or `now_ms` when the ledger is empty).
    pub window_start_ms: i64,
    pub total_runs: u64,
    /// Runs whose terminal status is exactly `"ok"`.
    pub ok_runs: u64,
    /// Terminal runs with any non-ok, non-aborted status (`"failed"`,
    /// `"spawn_failed"`, …).
    pub failed_runs: u64,
    /// Terminal runs explicitly marked `"aborted"`.
    pub aborted_runs: u64,
    /// Half-open runs (`finished_at_ms` is `None`) — still running, or
    /// orphaned by a crash.
    pub in_flight_runs: u64,
    /// Mean `finished_at_ms - started_at_ms` over terminal runs in window.
    /// `None` when no run in the window has finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_duration_ms: Option<f64>,
    /// Distinct non-empty `session_id`s that started a run in the window.
    pub active_sessions: u64,
    /// `total_runs` divided by window length in hours.  `None` when the
    /// window has zero length (empty `All` range).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_per_hour: Option<f64>,
    pub throughput: Vec<StatsBucket>,
    /// Most recent runs in the window, newest first.
    pub recent: Vec<StatsRecentRun>,
    /// Same counts over the immediately preceding window of equal length,
    /// for delta chips.  All `None` for [`StatsWindow::All`] (no earlier
    /// window to compare against).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_total_runs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_ok_runs: Option<u64>,
    /// Previous-window `failed + aborted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_error_runs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_avg_duration_ms: Option<f64>,
    /// Metric keys with no backing data — the UI hides these cards.
    pub unavailable: Vec<String>,
}

impl StatsSummary {
    /// Runs that ended badly (`failed` + `aborted`).
    pub fn error_runs(&self) -> u64 {
        self.failed_runs + self.aborted_runs
    }

    /// Whether a metric key is known to have no backing data.
    pub fn is_unavailable(&self, key: &str) -> bool {
        self.unavailable.iter().any(|k| k == key)
    }
}

/// Pure aggregation over a run slice.  Split out from [`RunLedger`] so it
/// can be unit-tested with hand-built records and no daemon or socket.
///
/// `now_ms` is injected (not read from the clock) so tests are
/// deterministic.
pub fn aggregate_stats(runs: &[RunRecord], range: &str, now_ms: i64) -> StatsSummary {
    let window = parse_stats_range(range);

    // Window bounds.  `All` starts at the earliest run so buckets span
    // only the period that actually has data.
    let (start, prev_start) = match window {
        StatsWindow::Last(w) => (now_ms - w, Some(now_ms - 2 * w)),
        StatsWindow::All => {
            let earliest = runs.iter().map(|r| r.started_at_ms).min().unwrap_or(now_ms);
            (earliest.min(now_ms), None)
        }
    };

    let in_window: Vec<&RunRecord> = runs.iter().filter(|r| r.started_at_ms >= start).collect();

    let mut ok_runs = 0u64;
    let mut failed_runs = 0u64;
    let mut aborted_runs = 0u64;
    let mut in_flight_runs = 0u64;
    let mut duration_sum = 0i128;
    let mut duration_n = 0u64;
    let mut sessions: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for run in &in_window {
        match run.finished_at_ms {
            None => in_flight_runs += 1,
            Some(fin) => {
                match run.status.as_deref() {
                    Some("ok") => ok_runs += 1,
                    Some("aborted") => aborted_runs += 1,
                    _ => failed_runs += 1,
                }
                if fin >= run.started_at_ms {
                    duration_sum += (fin - run.started_at_ms) as i128;
                    duration_n += 1;
                }
            }
        }
        if !run.session_id.is_empty() {
            sessions.insert(run.session_id.as_str());
        }
    }

    let total_runs = in_window.len() as u64;
    let avg_duration_ms = (duration_n > 0).then(|| duration_sum as f64 / duration_n as f64);

    let window_ms = (now_ms - start).max(0);
    let runs_per_hour = (window_ms > 0).then(|| total_runs as f64 * 3_600_000.0 / window_ms as f64);

    // Previous window of equal length, for delta chips.
    let prev = prev_start.map(|ps| {
        let slice: Vec<&RunRecord> = runs
            .iter()
            .filter(|r| r.started_at_ms >= ps && r.started_at_ms < start)
            .collect();
        let mut ok = 0u64;
        let mut err = 0u64;
        let mut dsum = 0i128;
        let mut dn = 0u64;
        for run in &slice {
            if let Some(fin) = run.finished_at_ms {
                if run.status.as_deref() == Some("ok") {
                    ok += 1;
                } else {
                    err += 1;
                }
                if fin >= run.started_at_ms {
                    dsum += (fin - run.started_at_ms) as i128;
                    dn += 1;
                }
            }
        }
        let avg = (dn > 0).then(|| dsum as f64 / dn as f64);
        (slice.len() as u64, ok, err, avg)
    });

    // Recent feed, newest first.
    let mut recent_src = in_window.clone();
    recent_src.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    let recent: Vec<StatsRecentRun> = recent_src
        .iter()
        .take(STATS_BUCKETS)
        .map(|r| StatsRecentRun {
            run_id: r.run_id.clone(),
            session_id: r.session_id.clone(),
            started_at_ms: r.started_at_ms,
            duration_ms: r
                .finished_at_ms
                .filter(|fin| *fin >= r.started_at_ms)
                .map(|fin| fin - r.started_at_ms),
            status: match (r.finished_at_ms, r.status.as_deref()) {
                (None, _) => "running".to_string(),
                (Some(_), Some(s)) => s.to_string(),
                (Some(_), None) => "unknown".to_string(),
            },
        })
        .collect();

    StatsSummary {
        range: range.to_string(),
        now_ms,
        window_start_ms: start,
        total_runs,
        ok_runs,
        failed_runs,
        aborted_runs,
        in_flight_runs,
        avg_duration_ms,
        active_sessions: sessions.len() as u64,
        runs_per_hour,
        throughput: bucket_runs(&in_window, start, now_ms),
        recent,
        prev_total_runs: prev.map(|p| p.0),
        prev_ok_runs: prev.map(|p| p.1),
        prev_error_runs: prev.map(|p| p.2),
        prev_avg_duration_ms: prev.and_then(|p| p.3),
        unavailable: STATS_UNAVAILABLE.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// Split `[start, now]` into [`STATS_BUCKETS`] equal buckets and count run
/// starts per bucket.  Returns an empty series for a zero-length window
/// (nothing to plot).
fn bucket_runs(runs: &[&RunRecord], start: i64, now_ms: i64) -> Vec<StatsBucket> {
    let span = now_ms - start;
    if span <= 0 {
        return Vec::new();
    }
    let n = STATS_BUCKETS as i64;
    // Sub-day buckets get a clock label; coarser ones a calendar label.
    let per_bucket = span / n;
    let day_ms = 86_400_000i64;
    let mut out = Vec::with_capacity(STATS_BUCKETS);
    for i in 0..n {
        let b_start = start + span * i / n;
        let b_end = start + span * (i + 1) / n;
        let label = if per_bucket < day_ms {
            utc_hh_mm(b_start)
        } else {
            utc_mm_dd(b_start)
        };
        // Last bucket is inclusive of `now_ms` so a run at exactly `now`
        // is not dropped.
        let last = i == n - 1;
        let count = runs
            .iter()
            .filter(|r| {
                r.started_at_ms >= b_start
                    && (r.started_at_ms < b_end || (last && r.started_at_ms <= now_ms))
            })
            .count() as u64;
        out.push(StatsBucket {
            start_ms: b_start,
            end_ms: b_end,
            label,
            runs: count,
        });
    }
    out
}

/// `HH:MM` in UTC.  Deliberately UTC (not local) so aggregation stays a
/// pure function of the input and tests are reproducible anywhere.
fn utc_hh_mm(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let day_secs = secs.rem_euclid(86_400);
    format!("{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60)
}

/// `MM-DD` in UTC (civil date from days since epoch).
fn utc_mm_dd(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (_, m, d) = civil_from_days(days);
    format!("{m:02}-{d:02}")
}

/// Days since 1970-01-01 → (year, month, day).  Hinnant's civil-from-days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
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

// ---------------------------------------------------------------------------
// EventBus (R2 3.3)
// ---------------------------------------------------------------------------

/// Fan-out table behind `event.subscribe`: topic → live registrations.
/// A registration is `(subscription id, connection write-channel)`; pushing
/// an event serializes to one already-formatted JSONL line and sends it to
/// every subscriber of the topic.  Dead connections (receiver dropped, e.g.
/// the client hung up) are unregistered on the spot; connection teardown
/// removes everything a connection owned via [`EventBus::remove_conn`].
#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<Mutex<BusInner>>,
}

#[derive(Default)]
struct BusInner {
    by_id: std::collections::HashMap<u64, Registration>,
    by_conn: std::collections::HashMap<u64, Vec<u64>>,
    by_topic: std::collections::HashMap<String, Vec<u64>>,
    next_id: u64,
}

struct Registration {
    topic: String,
    conn_id: u64,
    tx: std::sync::mpsc::Sender<String>,
}

impl EventBus {
    /// An empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `tx` as a writer for `topic` on connection `conn_id`.
    /// Returns the subscription id echoed back to the client.
    pub fn subscribe(&self, topic: &str, conn_id: u64, tx: std::sync::mpsc::Sender<String>) -> u64 {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = inner.next_id;
        inner.next_id += 1;
        inner.by_id.insert(
            id,
            Registration {
                topic: topic.to_string(),
                conn_id,
                tx,
            },
        );
        inner.by_conn.entry(conn_id).or_default().push(id);
        inner
            .by_topic
            .entry(topic.to_string())
            .or_default()
            .push(id);
        id
    }

    /// Drop one subscription by id.  Returns whether it existed.
    pub fn unsubscribe(&self, id: u64) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(reg) = inner.by_id.remove(&id) else {
            return false;
        };
        detach(&mut inner, id, reg.topic, reg.conn_id);
        true
    }

    /// Drop every subscription owned by a connection (teardown path).
    pub fn remove_conn(&self, conn_id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ids = inner.by_conn.remove(&conn_id).unwrap_or_default();
        for id in ids {
            if let Some(reg) = inner.by_id.remove(&id) {
                detach(&mut inner, id, reg.topic, conn_id);
            }
        }
    }

    /// Send one pre-formatted JSONL line to every subscriber of `topic`.
    /// Returns how many deliveries succeeded.  Failed sends (dead writer
    /// threads) unregister that subscription in place.
    pub fn broadcast(&self, topic: &str, line: &str) -> usize {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<u64> = match inner.by_topic.get(topic) {
            Some(ids) => ids.clone(),
            None => return 0,
        };
        let mut sent = 0usize;
        let mut dead = Vec::new();
        for id in &ids {
            match inner.by_id.get(id) {
                Some(reg) if reg.tx.send(line.to_string()).is_ok() => sent += 1,
                _ => dead.push(*id),
            }
        }
        if !dead.is_empty() {
            for id in dead {
                if let Some(reg) = inner.by_id.remove(&id) {
                    detach(&mut inner, id, reg.topic, reg.conn_id);
                }
            }
        }
        sent
    }

    /// Whether at least one live subscription exists for `topic`.
    pub fn has_topic(&self, topic: &str) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.by_topic.get(topic).is_some_and(|ids| !ids.is_empty())
    }
}

/// Remove `id` from the conn/topic indexes (the by_id entry is the caller's
/// job).  Linear scans are fine: subscription counts are tiny by design.
fn detach(inner: &mut BusInner, id: u64, topic: String, conn_id: u64) {
    if let Some(ids) = inner.by_topic.get_mut(&topic) {
        ids.retain(|x| *x != id);
        if ids.is_empty() {
            inner.by_topic.remove(&topic);
        }
    }
    if let Some(ids) = inner.by_conn.get_mut(&conn_id) {
        ids.retain(|x| *x != id);
        if ids.is_empty() {
            inner.by_conn.remove(&conn_id);
        }
    }
}
