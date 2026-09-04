//! Shared local session/message store backed by libSQL (SQLite-compatible).
//!
//! Designed for use by the daemon, CLI, and agent tools. A single
//! [`SessionDb`] owns one connection to one local `*.db` file. Cross-thread
//! safety comes from a `parking_lot::Mutex` around the connection and from
//! running every call through one current-thread Tokio runtime created on `open`.
//!
//! ponytail: one connection + one runtime per `SessionDb`. Cross-thread
//! callers (multiple `SessionDb::open`s to the same file) rely on SQLite WAL
//! + `busy_timeout`; per-connection serialization here is to prevent
//!   re-entrant `block_on` on the current-thread runtime, not to provide
//!   cross-process mutual exclusion.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

// ponytail: only the `core` libsql feature is on; this drops the default
// `serde_json`/`reqwest`/HTTP/WebAssembly backends and the C-bundled SQLite
// is still present. Bump if you need remote/replica/sync features.
use libsql::{Connection, TransactionBehavior};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// Maximum allowed `limit` for list / load / search calls. Hard cap to keep a
/// runaway caller from forcing the process to materialize a huge result set.
pub const MAX_LIMIT: u32 = 1000;

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

/// Errors returned by every `SessionDb` operation.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// libsql returned an error from the underlying SQLite engine.
    #[error("libsql error: {0}")]
    Libsql(#[from] libsql::Error),

    /// I/O error (e.g. creating the parent directory of the database file).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Caller asked for `limit == 0` or `limit > MAX_LIMIT`.
    #[error("invalid limit {0}: must be in 1..={MAX_LIMIT}")]
    InvalidLimit(u32),

    /// Caller passed a role string that is not one of the known
    /// [`SessionRole`] variants.
    #[error("unknown role `{0}` (expected one of user, assistant, system, tool)")]
    UnknownRole(String),

    /// Caller passed an empty/whitespace-only session id or title.
    #[error("invalid session id: must be non-empty")]
    InvalidSessionId,

    /// Caller passed an empty message body.
    #[error("invalid message text: must be non-empty")]
    InvalidMessageText,

    /// Caller passed an empty search query.
    #[error("invalid search query: must be non-empty")]
    InvalidSearchQuery,

    /// Caller passed an empty list-sessions query.
    #[error("invalid list query: must be non-empty")]
    InvalidListQuery,

    /// Tokio runtime could not be created. Stored as a string because
    /// `tokio::runtime::Builder::build` returns `io::Error` rather than a
    /// named error type, and we already wrap plain `io::Error` separately.
    #[error("failed to build tokio runtime: {0}")]
    RuntimeBuild(String),

    /// Asked the database to operate on a path that does not exist on disk.
    #[error("database file `{0}` does not exist")]
    DatabaseMissing(PathBuf),
}

impl SessionError {
    fn invalid_id_if_blank(id: &str) -> Result<(), SessionError> {
        if id.trim().is_empty() {
            Err(SessionError::InvalidSessionId)
        } else {
            Ok(())
        }
    }
}

// -----------------------------------------------------------------------------
// Roles
// -----------------------------------------------------------------------------

/// A message role. Kept small and string-backed so it can round-trip through
/// both the SQL column and serde without an extra enum-mapping layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionRole {
    User,
    Assistant,
    System,
    Tool,
}

impl SessionRole {
    /// Canonical lowercase wire/storage form (`"user"`, `"assistant"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            SessionRole::User => "user",
            SessionRole::Assistant => "assistant",
            SessionRole::System => "system",
            SessionRole::Tool => "tool",
        }
    }

    /// Parse from any case. Returns [`SessionError::UnknownRole`] on miss.
    pub fn parse(s: &str) -> Result<SessionRole, SessionError> {
        match s.to_ascii_lowercase().as_str() {
            "user" => Ok(SessionRole::User),
            "assistant" => Ok(SessionRole::Assistant),
            "system" => Ok(SessionRole::System),
            "tool" => Ok(SessionRole::Tool),
            _ => Err(SessionError::UnknownRole(s.to_string())),
        }
    }
}

impl fmt::Display for SessionRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// -----------------------------------------------------------------------------
// Records
// -----------------------------------------------------------------------------

/// Row from `list_sessions` — the fields the UI needs to render a session
/// list. `message_count` is always populated by list/lookup paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    /// Unix epoch milliseconds. Matches `created_at` / `updated_at` columns.
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message_count: u64,
}

/// One message row, in storage order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMessage {
    pub session_id: String,
    /// Monotonic per-session, starting at 1. Assigned by
    /// [`SessionDb::append_message`] inside a single transaction.
    pub seq: i64,
    pub role: SessionRole,
    pub text: String,
    /// Unix epoch milliseconds — set when the row is appended.
    pub created_at_ms: i64,
}

// -----------------------------------------------------------------------------
// SessionDb
// -----------------------------------------------------------------------------

/// One open session database. Cheap to clone (the inner state is `Arc`-shared).
///
/// `Send + Sync` so any thread can call methods. Internal serialization is a
/// `parking_lot::Mutex` over the single `Connection`; this both serializes
/// reads/writes (libSQL/connection methods are `&self` but SQLite itself has
/// only one writer at a time) and prevents re-entrant `block_on` calls on the
/// current-thread runtime owned here.
#[derive(Clone)]
pub struct SessionDb {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    runtime: tokio::runtime::Runtime,
    conn: Mutex<Connection>,
}

// -----------------------------------------------------------------------------
// Construction
// -----------------------------------------------------------------------------

impl SessionDb {
    /// Open (or create) the database at `path`. Creates the parent directory
    /// if missing, applies schema + PRAGMAs, then hands back a ready
    /// `SessionDb`.
    ///
    /// The same file can be opened from multiple threads / processes — SQLite
    /// WAL + the `busy_timeout` PRAGMA handle cross-process contention. We
    /// set `busy_timeout` and `foreign_keys` before any DDL or `journal_mode`
    /// switch so concurrent `SessionDb::open` calls block on the lock instead
    /// of erroring with SQLITE_BUSY.
    pub fn open(path: impl AsRef<Path>) -> Result<SessionDb, SessionError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
        }

        // Build the long-lived runtime first; use it for everything else.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| SessionError::RuntimeBuild(e.to_string()))?;

        let db = runtime.block_on(libsql::Builder::new_local(&path).build())?;
        let conn = db.connect()?;
        runtime.block_on(init_database(&conn))?;

        Ok(SessionDb {
            inner: Arc::new(Inner {
                path,
                runtime,
                conn: Mutex::new(conn),
            }),
        })
    }

    /// Path the database was opened with.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }
}

/// Number of retry rounds when `apply_schema` or `journal_mode` switching
/// hits a transient lock error. Combined with `busy_timeout`, this is a
/// safety net for the rare case where two callers race past the timeout
/// boundary (e.g. one process is mid-switch-to-WAL while another is mid-DDL).
/// Bounded on purpose: a runaway init that retries indefinitely would mask
/// real I/O or schema problems.
const INIT_LOCK_RETRIES: usize = 5;

/// Set the per-connection knobs that don't need a write lock first so any
/// later DDL or journal-mode switch can block on contention instead of
/// returning SQLITE_BUSY immediately.
async fn apply_safe_pragmas(conn: &Connection) -> Result<(), SessionError> {
    run_with_lock_retry(|| async {
        conn.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;")
            .await?;
        Ok(())
    })
    .await
}
async fn apply_schema(conn: &Connection) -> Result<(), SessionError> {
    let sql = "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS messages (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            role TEXT NOT NULL,
            text TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (session_id, seq),
            FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);";
    run_with_lock_retry(|| async {
        conn.execute_batch(sql).await?;
        Ok(())
    })
    .await
}

async fn apply_pragmas(conn: &Connection) -> Result<(), SessionError> {
    conn.execute_batch("PRAGMA synchronous = NORMAL;").await?;
    Ok(())
}

async fn enable_wal_if_needed(conn: &Connection) -> Result<(), SessionError> {
    // `query` returns a `Rows` future that holds a live prepared statement.
    // We must fully drain it (or `None`) before issuing another statement
    // — otherwise libsql leaves the connection with an open statement,
    // and the next `PRAGMA journal_mode = WAL` rejects it as "from within a
    // transaction".
    let mut rows = conn.query("PRAGMA journal_mode", libsql::params![]).await?;
    let current = loop {
        match rows.next().await? {
            Some(row) => {
                if let Ok(s) = row.get_str(0) {
                    break s.to_string();
                }
            }
            None => break String::new(),
        }
    };
    drop(rows);
    if current.eq_ignore_ascii_case("wal") {
        return Ok(());
    }
    // Switching to WAL requires a write lock; retry on transient lock errors.
    run_with_lock_retry(|| async {
        conn.execute_batch("PRAGMA journal_mode = WAL;").await?;
        Ok(())
    })
    .await
}

async fn init_database(conn: &Connection) -> Result<(), SessionError> {
    apply_safe_pragmas(conn).await?;
    enable_wal_if_needed(conn).await?;
    apply_pragmas(conn).await?;
    apply_schema(conn).await?;
    Ok(())
}

fn is_lock_error(err: &SessionError) -> bool {
    let SessionError::Libsql(libsql::Error::SqliteFailure(code, _)) = err else {
        return false;
    };
    // Both SQLITE_BUSY (5) and SQLITE_LOCKED (6) and their extended codes
    // share the primary code in the low 8 bits.
    matches!(*code & 0xFF, 5 | 6)
}

async fn run_with_lock_retry<F, Fut>(op: F) -> Result<(), SessionError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), SessionError>>,
{
    let mut attempt = 0usize;
    loop {
        match op().await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < INIT_LOCK_RETRIES && is_lock_error(&e) => {
                attempt += 1;
                // No sleep: `busy_timeout` already drives the actual wait
                // inside SQLite; this loop is a hedge against the rare case
                // where the contention window is longer than the timeout.
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

fn now_ms() -> i64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

fn validate_limit(limit: u32) -> Result<u32, SessionError> {
    if limit == 0 || limit > MAX_LIMIT {
        Err(SessionError::InvalidLimit(limit))
    } else {
        Ok(limit)
    }
}

// -----------------------------------------------------------------------------
// API: sessions
// -----------------------------------------------------------------------------

impl SessionDb {
    /// Create a brand-new session, or return the existing row (with its
    /// original timestamps) if `id` is already present.
    ///
    /// `id` and `title` must be non-empty (after trimming).
    pub fn ensure_session(&self, id: &str, title: &str) -> Result<SessionSummary, SessionError> {
        SessionError::invalid_id_if_blank(id)?;
        if title.trim().is_empty() {
            return Err(SessionError::InvalidSessionId);
        }
        let id_owned = id.to_string();
        let title_owned = title.to_string();

        let guard = self.inner.conn.lock();
        // ponytail: scope the guard so the lock is released the instant the
        // async block returns — no explicit drop call needed and no chance
        // of using the connection after a different call gets in.
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            let now = now_ms();
            // INSERT OR IGNORE leaves existing rows alone (so created_at is
            // preserved); we then UPDATE updated_at so the row moves to the
            // top of list_sessions.
            conn.execute(
                "INSERT OR IGNORE INTO sessions (id, title, created_at, updated_at) \
                  VALUES (?1, ?2, ?3, ?3)",
                libsql::params![id_owned.as_str(), title_owned.as_str(), now],
            )
            .await?;
            conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                libsql::params![now, id_owned.as_str()],
            )
            .await?;
            load_session_row(conn, id_owned.as_str())
                .await
                .ok_or_else(|| SessionError::DatabaseMissing(PathBuf::from(id_owned)))
        })
    }

    /// Convenience wrapper: always create. Returns the existing row if the id
    /// was already taken — same as [`Self::ensure_session`] but with a
    /// shorter name for the "I just want to make sure this exists" call site.
    pub fn create_session(&self, id: &str, title: &str) -> Result<SessionSummary, SessionError> {
        self.ensure_session(id, title)
    }

    /// Delete a session and all of its messages (via `ON DELETE CASCADE`).
    /// Returns true if a row was removed.
    pub fn delete_session(&self, id: &str) -> Result<bool, SessionError> {
        SessionError::invalid_id_if_blank(id)?;
        let id_owned = id.to_string();

        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            let n = conn
                .execute(
                    "DELETE FROM sessions WHERE id = ?1",
                    libsql::params![id_owned.as_str()],
                )
                .await?;
            Ok(n > 0)
        })
    }

    /// List sessions whose id or title contains `query` (case-insensitive
    /// `LIKE`), ordered by `updated_at DESC, id ASC`. `limit` is hard-capped
    /// at [`MAX_LIMIT`]; 0 is rejected.
    pub fn list_sessions(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        let limit = validate_limit(limit)?;
        if query.is_empty() {
            return Err(SessionError::InvalidListQuery);
        }
        let like = format!("%{}%", query);

        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            let mut rows = conn
                .query(
                    "SELECT s.id, s.title, s.created_at, s.updated_at, \
                            (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) \
                     FROM sessions s \
                     WHERE s.id LIKE ?1 COLLATE NOCASE OR s.title LIKE ?1 COLLATE NOCASE \
                     ORDER BY s.updated_at DESC, s.id ASC \
                     LIMIT ?2",
                    libsql::params![like, limit],
                )
                .await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(SessionSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at_ms: row.get(2)?,
                    updated_at_ms: row.get(3)?,
                    message_count: row.get::<i64>(4)?.max(0) as u64,
                });
            }
            Ok(out)
        })
    }

    /// Look up a single session by id (returns `None` when missing).
    pub fn session(&self, id: &str) -> Result<Option<SessionSummary>, SessionError> {
        SessionError::invalid_id_if_blank(id)?;
        let id_owned = id.to_string();

        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            Ok(load_session_row(conn, id_owned.as_str()).await)
        })
    }

    /// `true` if no session rows exist. Used by tests / callers that want to
    /// skip a "fresh DB" fast path.
    pub fn is_empty(&self) -> Result<bool, SessionError> {
        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            let mut rows = conn.query("SELECT 1 FROM sessions LIMIT 1", ()).await?;
            Ok(rows.next().await?.is_none())
        })
    }
}

async fn load_session_row(conn: &Connection, id: &str) -> Option<SessionSummary> {
    let mut rows = match conn
        .query(
            "SELECT id, title, created_at, updated_at, \
                    (SELECT COUNT(*) FROM messages WHERE session_id = s.id) \
             FROM sessions s WHERE id = ?1",
            libsql::params![id],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return None,
    };
    let row = rows.next().await.ok().flatten()?;
    Some(SessionSummary {
        id: row.get(0).ok()?,
        title: row.get(1).ok()?,
        created_at_ms: row.get(2).ok()?,
        updated_at_ms: row.get(3).ok()?,
        message_count: row.get::<i64>(4).ok()?.max(0) as u64,
    })
}

// -----------------------------------------------------------------------------
// API: messages
// -----------------------------------------------------------------------------

impl SessionDb {
    /// Append a message to a session inside an `Immediate` transaction.
    /// Returns the assigned `seq` (1-based, monotonic per session) and the
    /// row's `created_at_ms`.
    ///
    /// Creates the session row if it does not yet exist (caller may have
    /// forgotten to `ensure_session` first; the id is used as a placeholder
    /// title).
    pub fn append_message(
        &self,
        session_id: &str,
        role: SessionRole,
        text: &str,
    ) -> Result<(i64, i64), SessionError> {
        SessionError::invalid_id_if_blank(session_id)?;
        if text.is_empty() {
            return Err(SessionError::InvalidMessageText);
        }
        let id_owned = session_id.to_string();
        let text_owned = text.to_string();

        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            let now = now_ms();
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await?;

            // Make sure the session row exists. We never overwrite an
            // existing title — that would clobber user-set data inside the
            // same transaction.
            let mut title_row = tx
                .query(
                    "SELECT title FROM sessions WHERE id = ?1",
                    libsql::params![id_owned.as_str()],
                )
                .await?;
            let have_session = title_row.next().await?.is_some();
            if !have_session {
                tx.execute(
                    "INSERT INTO sessions (id, title, created_at, updated_at) \
                     VALUES (?1, ?1, ?2, ?2)",
                    libsql::params![id_owned.as_str(), now],
                )
                .await?;
            }

            // MAX(seq) + 1, atomically inside the tx.
            let mut seq_row = tx
                .query(
                    "SELECT COALESCE(MAX(seq), 0) FROM messages WHERE session_id = ?1",
                    libsql::params![id_owned.as_str()],
                )
                .await?;
            let next_seq: i64 = seq_row
                .next()
                .await?
                .map(|r| r.get::<i64>(0).unwrap_or(0))
                .unwrap_or(0)
                + 1;

            tx.execute(
                "INSERT INTO messages (session_id, seq, role, text, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                libsql::params![
                    id_owned.as_str(),
                    next_seq,
                    role.as_str(),
                    text_owned.as_str(),
                    now
                ],
            )
            .await?;

            // Bump session.updated_at so it surfaces to the top of list.
            tx.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                libsql::params![now, id_owned.as_str()],
            )
            .await?;

            // commit() consumes `tx`; on any error along the way, the
            // libsql Drop impl rolls back, so the tx is never left open.
            tx.commit().await?;
            Ok((next_seq, now))
        })
    }

    /// Load up to `limit` messages for `session_id`, ordered by `seq ASC`.
    pub fn load_messages(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<SessionMessage>, SessionError> {
        SessionError::invalid_id_if_blank(session_id)?;
        let limit = validate_limit(limit)?;
        let id_owned = session_id.to_string();

        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            let mut rows = conn
                .query(
                    "SELECT session_id, seq, role, text, created_at \
                     FROM messages WHERE session_id = ?1 \
                     ORDER BY seq ASC LIMIT ?2",
                    libsql::params![id_owned.as_str(), limit],
                )
                .await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let role_str: String = row.get(2)?;
                let role = SessionRole::parse(&role_str)?;
                out.push(SessionMessage {
                    session_id: row.get(0)?,
                    seq: row.get(1)?,
                    role,
                    text: row.get(3)?,
                    created_at_ms: row.get(4)?,
                });
            }
            Ok(out)
        })
    }

    /// Substring search across message text, optionally scoped to one
    /// session. Returns matches ordered by `created_at ASC, session_id ASC,
    /// seq ASC` so successive pages of the same time window are stable.
    pub fn search_messages(
        &self,
        query: &str,
        session_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SessionMessage>, SessionError> {
        if query.is_empty() {
            return Err(SessionError::InvalidSearchQuery);
        }
        let limit = validate_limit(limit)?;
        let like = format!("%{}%", query);
        // Validate scoped id up front so the error is reported eagerly.
        let scoped_owned: Option<String> = match session_id {
            Some(s) => {
                SessionError::invalid_id_if_blank(s)?;
                Some(s.to_string())
            }
            None => None,
        };

        let guard = self.inner.conn.lock();
        self.inner.runtime.block_on(async move {
            let conn = &*guard;
            // ponytail: a single `session_id IS NULL OR = ?` keeps the query
            // shape identical whether the caller scoped or not, so we only
            // need one prepared path.
            let mut rows = match scoped_owned {
                Some(sid) => {
                    conn.query(
                        "SELECT session_id, seq, role, text, created_at \
                         FROM messages \
                         WHERE session_id = ?1 AND text LIKE ?2 COLLATE NOCASE \
                         ORDER BY created_at ASC, session_id ASC, seq ASC \
                         LIMIT ?3",
                        libsql::params![sid.as_str(), like, limit],
                    )
                    .await?
                }
                None => {
                    conn.query(
                        "SELECT session_id, seq, role, text, created_at \
                         FROM messages \
                         WHERE text LIKE ?1 COLLATE NOCASE \
                         ORDER BY created_at ASC, session_id ASC, seq ASC \
                         LIMIT ?2",
                        libsql::params![like, limit],
                    )
                    .await?
                }
            };

            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let role_str: String = row.get(2)?;
                let role = SessionRole::parse(&role_str)?;
                out.push(SessionMessage {
                    session_id: row.get(0)?,
                    seq: row.get(1)?,
                    role,
                    text: row.get(3)?,
                    created_at_ms: row.get(4)?,
                });
            }
            Ok(out)
        })
    }
}
