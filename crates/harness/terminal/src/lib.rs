//! Persistent pty-backed terminal sessions.
//!
//! A `run_bash` call runs one command and dies with it: `cd` does not survive
//! to the next call, an interactive program cannot be driven, and a shell that
//! holds state (env, aliases, a REPL) is impossible. This crate keeps a real
//! pty session alive between tool calls, so the model can do
//! `terminal_create` → `terminal_write("cd /tmp")` → `terminal_write("pwd")`
//! and see `/tmp`.
//!
//! # Session lifetime
//!
//! Sessions live in a [`TerminalRegistry`] held by the daemon, not in the tool
//! call. Each session owns:
//!
//! * the pty master (read/write/resize),
//! * a **reader thread** that drains the pty master into an in-memory buffer.
//!
//! The reader thread is what makes `read` non-destructive and non-blocking:
//! the tool returns whatever has accumulated since the last read and never
//! stalls waiting for the shell to produce output. Without it a `read` on an
//! idle shell would block forever, which is exactly the failure mode this
//! crate exists to avoid.
//!
//! # Reading is a drain, not a screen
//!
//! [`TerminalRegistry::read`] returns the bytes captured since the previous
//! read and clears the buffer. It does *not* emulate a terminal: there is no
//! screen, no cursor, no alternate-buffer handling. Control sequences are
//! passed through verbatim. That is the honest level of fidelity for this
//! layer — anything smarter belongs in a terminal emulator in front of it.
//!
//! Two consequences worth knowing before writing a caller:
//!
//! * **A pty echoes its input, and counting occurrences is not a fix.** Writing
//!   `echo hi\n` puts `hi` on the read side twice: once in the echoed command
//!   line, once as the command's output. The tempting workaround — "wait for
//!   two occurrences" — is *unreliable*, because the pty delivers bytes with no
//!   framing: the echoed line can itself arrive split across reads, and the
//!   marker is then counted from the echo twice before the command has produced
//!   anything. The caller returns early holding the echo and never sees the
//!   result.
//!
//!   The robust fix is to remove the ambiguity rather than count around it:
//!   have the shell assemble the sentinel from pieces so the literal never
//!   appears in the command text you wrote.
//!
//!   ```text
//!   printf '\n__DONE_%s__\n' OK      # literal "__DONE_OK__" is not in this line
//!   ```
//!
//!   Any occurrence of `__DONE_OK__` on the read side is then genuine output,
//!   and one occurrence is enough.
//! * **A full-screen TUI** (`vim`, `htop`) produces a stream of escape codes
//!   rather than a picture of the screen.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub use portable_pty::ExitStatus;

// -----------------------------------------------------------------------------
// Identifiers
// -----------------------------------------------------------------------------

/// Opaque terminal session identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    pub fn new(id: impl Into<String>) -> Self {
        TerminalId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TerminalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("unknown terminal: {0}")]
    Unknown(TerminalId),
    /// The pty layer failed. Carries the underlying message because
    /// `portable_pty::Error` is `anyhow::Error` and does not implement
    /// `std::error::Error` for `#[from]` to work usefully across it.
    #[error("pty: {0}")]
    Pty(String),
    #[error("terminal {0} has exited")]
    Exited(TerminalId),
    /// The session table is at its ceiling.
    #[error("terminal registry refused the session: {0}")]
    Refused(String),
}

impl From<anyhow::Error> for TerminalError {
    fn from(e: anyhow::Error) -> Self {
        TerminalError::Pty(e.to_string())
    }
}

impl From<std::io::Error> for TerminalError {
    fn from(e: std::io::Error) -> Self {
        TerminalError::Pty(e.to_string())
    }
}

// -----------------------------------------------------------------------------
// Session
// -----------------------------------------------------------------------------

/// Default pty geometry. 80x24 is the historical terminal default and what
/// most shells assume before a resize arrives.
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// How much output one session may buffer before the oldest bytes are dropped.
///
/// A reader thread on a chatty program (`yes`, a build log) would otherwise
/// grow the buffer without bound while the model is busy elsewhere. Dropping
/// the *oldest* bytes is the right end to lose: the recent output is what a
/// `read` is usually after. The truncation is reported in the read result so
/// the model is never silently lied to about having seen everything.
pub const MAX_BUFFER_BYTES: usize = 1 << 20; // 1 MiB

/// Shared state between the reader thread and the registry's public methods.
struct SessionInner {
    /// Output captured since the last drain, plus a flag for whether the
    /// buffer overflowed and lost its head.
    buffer: Mutex<Buffer>,
    /// Signalled on every append and on reader-thread exit, so a blocking
    /// `read` can wait for output instead of polling.
    appended: Condvar,
    /// Set once the reader thread has drained the last of the output.
    eof: AtomicBool,
    /// Set by `kill`/`close`, so the reader thread stops trying to drain.
    closed: AtomicBool,
}

#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    /// Bytes dropped from the head while the model was not reading.
    dropped: u64,
}

struct Session {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// `Child` is only needed for `wait`/`try_wait`; kept separate from the
    /// killer so a blocked `wait` never holds the lock `kill` needs.
    child: Mutex<Box<dyn Child + Send + Sync>>,
    /// Immutable description, captured at spawn time for `list`/`status`.
    meta: SessionMeta,
    inner: Arc<SessionInner>,
}

/// The spawn-time description of a session.
///
/// Stored rather than derived because the pty cannot be asked what program it
/// runs or what size it was *created* with after a resize.
struct SessionMeta {
    shell: String,
    cwd: String,
    /// Size at creation, for `list`. `resize` does not update this: the field
    /// documents the spawn geometry, and the live size is queryable from the
    /// master if a caller ever needs it.
    cols: u16,
    rows: u16,
}

impl Session {
    /// Append reader-thread output, dropping the oldest bytes on overflow.
    fn push_output(inner: &SessionInner, chunk: &[u8]) {
        let mut buf = inner.buffer.lock().unwrap();
        buf.bytes.extend_from_slice(chunk);
        if buf.bytes.len() > MAX_BUFFER_BYTES {
            let overflow = buf.bytes.len() - MAX_BUFFER_BYTES;
            buf.bytes.drain(..overflow);
            buf.dropped += overflow as u64;
        }
        drop(buf);
        inner.appended.notify_all();
    }
}

// -----------------------------------------------------------------------------
// Public snapshot types
// -----------------------------------------------------------------------------

/// What a `read` call returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutput {
    /// Output captured since the previous read, verbatim (escape codes
    /// included).
    pub bytes: Vec<u8>,
    /// How many older bytes were dropped because the buffer overflowed before
    /// this read. Non-zero means the caller missed output.
    pub dropped: u64,
    /// Whether the shell has exited and no more output is coming.
    pub exited: bool,
}

impl TerminalOutput {
    /// The captured output as a lossy UTF-8 string, for handing to a model.
    ///
    /// Lossy on purpose: a pty emits arbitrary bytes, and refusing to decode
    /// a half-written multi-byte character would be worse than a replacement
    /// character.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    /// Whether this read produced nothing at all.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// One row of [`TerminalRegistry::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSummary {
    pub id: TerminalId,
    /// The shell command line this session runs.
    pub shell: String,
    /// Working directory the session started in.
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    /// Whether the shell process has exited.
    pub exited: bool,
    /// Exit code, once known.
    pub exit_code: Option<u32>,
}

// -----------------------------------------------------------------------------
// Registry
// -----------------------------------------------------------------------------

/// Holds live pty sessions for the process lifetime.
pub struct TerminalRegistry {
    sessions: Mutex<HashMap<TerminalId, Arc<Session>>>,
    seq: AtomicU64,
    max_sessions: AtomicU64,
}

impl Default for TerminalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalRegistry {
    /// Default cap on concurrently live sessions.
    ///
    /// Each session costs a pty plus a reader thread, so an unbounded
    /// `terminal_create` loop is a resource leak. Terminal sessions are also
    /// far more expensive than jobs, so the ceiling is lower.
    pub const MAX_SESSIONS: usize = 16;

    pub fn new() -> Self {
        TerminalRegistry {
            sessions: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(0),
            max_sessions: AtomicU64::new(Self::MAX_SESSIONS as u64),
        }
    }

    /// Spawn a shell in a new pty and start draining it.
    ///
    /// `shell` is a command line, split on whitespace, not a single
    /// executable path — `terminal_create` binds it to the model's input, and
    /// requiring the model to encode `bash --norc` as a single argument would
    /// be pointless friction. There is no shell interpolation of the command
    /// itself; the *session* is a shell, so the model can send `cd x && y` as
    /// session input.
    pub fn create(
        &self,
        shell: &str,
        cwd: &str,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalId, TerminalError> {
        let mut argv = shell.split_whitespace();
        let program = argv
            .next()
            .ok_or_else(|| TerminalError::Refused("empty shell command".into()))?;

        let mut cmd = CommandBuilder::new(program);
        for arg in argv {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);

        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let child = pair.slave.spawn_command(cmd)?;
        // The slave handle must be dropped promptly: holding it open keeps the
        // pty from ever reporting EOF when the shell exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let killer = child.clone_killer();

        let inner = Arc::new(SessionInner {
            buffer: Mutex::new(Buffer::default()),
            appended: Condvar::new(),
            eof: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });

        // Reader thread: the pty master is drained here for the whole session
        // life. `read` never touches the master, so a `terminal_read` on an
        // idle shell returns promptly instead of blocking on the pty.
        let reader_inner = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("omenic-pty-reader".into())
            .spawn(move || {
                let mut reader = reader;
                let mut chunk = [0u8; 8192];
                loop {
                    if reader_inner.closed.load(Ordering::Relaxed) {
                        break;
                    }
                    match reader.read(&mut chunk) {
                        // EOF: the shell exited and the pty closed.
                        Ok(0) => break,
                        Ok(n) => Session::push_output(&reader_inner, &chunk[..n]),
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        // EIO on Linux is the normal way a pty reports "the
                        // child is gone"; anything else is also terminal for
                        // this reader, so both end the loop the same way.
                        Err(_) => break,
                    }
                }
                reader_inner.eof.store(true, Ordering::Relaxed);
                reader_inner.appended.notify_all();
            })
            .map_err(|e| TerminalError::Pty(e.to_string()))?;

        let id;
        {
            let mut sessions = self.sessions.lock().unwrap();
            let cap = self.max_sessions.load(Ordering::Relaxed) as usize;
            if sessions.len() >= cap {
                return Err(TerminalError::Refused(format!(
                    "{} sessions already open (ceiling {cap}); close one first",
                    sessions.len()
                )));
            }
            id = TerminalId(format!(
                "term-{}",
                self.seq.fetch_add(1, Ordering::Relaxed) + 1
            ));
            sessions.insert(
                id.clone(),
                Arc::new(Session {
                    master: Mutex::new(pair.master),
                    writer: Mutex::new(writer),
                    killer: Mutex::new(killer),
                    child: Mutex::new(child),
                    meta: SessionMeta {
                        shell: shell.to_string(),
                        cwd: cwd.to_string(),
                        cols,
                        rows,
                    },
                    inner,
                }),
            );
        }
        Ok(id)
    }

    /// Look up a session, or report it unknown.
    fn get(&self, id: &TerminalId) -> Result<Arc<Session>, TerminalError> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| TerminalError::Unknown(id.clone()))
    }

    /// Send input to the session's shell.
    ///
    /// No newline is appended: the model sends `"ls\n"` when it wants the
    /// command run, and a bare write is how it drives a program that reads
    /// without a line discipline.
    pub fn write(&self, id: &TerminalId, data: &str) -> Result<(), TerminalError> {
        let session = self.get(id)?;
        if session.inner.eof.load(Ordering::Relaxed) {
            return Err(TerminalError::Exited(id.clone()));
        }
        let mut writer = session.writer.lock().unwrap();
        writer.write_all(data.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    /// Drain output captured since the last read.
    ///
    /// `timeout_ms: None` returns immediately with whatever is buffered (it
    /// does **not** block for output). `Some(ms)` waits up to `ms` for
    /// *something* to arrive, which is how a caller waits for a command to
    /// finish without polling.
    pub fn read(
        &self,
        id: &TerminalId,
        timeout_ms: Option<u64>,
    ) -> Result<TerminalOutput, TerminalError> {
        let session = self.get(id)?;
        let inner = &session.inner;

        if let Some(ms) = timeout_ms {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            let mut buf = inner.buffer.lock().unwrap();
            while buf.bytes.is_empty() && !inner.eof.load(Ordering::Relaxed) {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let (next, _) = inner.appended.wait_timeout(buf, remaining).unwrap();
                buf = next;
            }
        }

        let (bytes, dropped) = {
            let mut buf = inner.buffer.lock().unwrap();
            let bytes = std::mem::take(&mut buf.bytes);
            let dropped = std::mem::replace(&mut buf.dropped, 0);
            (bytes, dropped)
        };
        Ok(TerminalOutput {
            bytes,
            dropped,
            exited: inner.eof.load(Ordering::Relaxed),
        })
    }

    /// Resize the pty. The kernel delivers `SIGWINCH` to the foreground
    /// process group, so a full-screen program re-layouts.
    pub fn resize(&self, id: &TerminalId, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let session = self.get(id)?;
        let master = session.master.lock().unwrap();
        master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Signal the shell to terminate, leaving the record in place.
    ///
    /// The record is kept so a later `read` can still drain the output the
    /// process produced on its way out. [`close`](Self::close) is what removes
    /// it.
    pub fn kill(&self, id: &TerminalId) -> Result<(), TerminalError> {
        let session = self.get(id)?;
        let mut killer = session.killer.lock().unwrap();
        killer.kill().map_err(TerminalError::from)
    }

    /// Terminate the shell (if still alive) and drop the session.
    pub fn close(&self, id: &TerminalId) -> Result<(), TerminalError> {
        let session = self.get(id)?;
        // Signal the reader to stop before killing, so it does not spend its
        // last moments appending output nobody will read.
        session.inner.closed.store(true, Ordering::Relaxed);
        {
            let mut killer = session.killer.lock().unwrap();
            // A session whose shell already exited reports an error from
            // `kill` (ESRCH); that is success for `close`'s purposes.
            let _ = killer.kill();
        }
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }

    /// Summary of one session.
    pub fn status(&self, id: &TerminalId) -> Result<TerminalSummary, TerminalError> {
        let session = self.get(id)?;
        Ok(Self::summarize(id, &session))
    }

    /// Build a summary from a session already looked up.
    ///
    /// Split out of [`status`](Self::status) because `list` holds the session
    /// table lock while it walks the table: `std::sync::Mutex` is not
    /// reentrant, so `list` calling `status` (which calls `get`, which locks
    /// the table) would deadlock. This takes the `Arc` directly.
    fn summarize(id: &TerminalId, session: &Session) -> TerminalSummary {
        let exit_code = {
            let mut child = session.child.lock().unwrap();
            child.try_wait().ok().flatten().map(|s| s.exit_code())
        };
        TerminalSummary {
            id: id.clone(),
            shell: session.meta.shell.clone(),
            cwd: session.meta.cwd.clone(),
            cols: session.meta.cols,
            rows: session.meta.rows,
            exited: session.inner.eof.load(Ordering::Relaxed) || exit_code.is_some(),
            exit_code,
        }
    }

    /// Every live session.
    ///
    /// Ordered by id's numeric suffix (creation order) rather than by hash,
    /// so repeated listings are stable.
    pub fn list(&self) -> Vec<TerminalSummary> {
        let sessions = self.sessions.lock().unwrap();
        let mut ids: Vec<TerminalId> = sessions.keys().cloned().collect();
        ids.sort_by_key(seq_of);
        ids.iter()
            // The guard is held for the whole walk, so every id still has a
            // session; the `get` cannot come back empty.
            .filter_map(|id| sessions.get(id).map(|s| Self::summarize(id, s)))
            .collect()
    }

    /// Number of live sessions. Test helper.
    pub fn len(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Override the session ceiling. Test-only in practice, but not gated on a
    /// compile-time test attribute: `tests/` link the crate as a dependency and
    /// would not see such an item.
    pub fn set_max_sessions(&self, max: usize) {
        self.max_sessions.store(max as u64, Ordering::Relaxed);
    }
}

/// Numeric suffix of `term-<n>`, for a stable listing order.
fn seq_of(id: &TerminalId) -> u64 {
    id.as_str()
        .rsplit_once('-')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(0)
}
