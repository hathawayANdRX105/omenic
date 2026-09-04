//! Single-instance daemon ownership via an exclusive file lock.
//!
//! The lock is a `<socket_path>.lock` companion file.  We `flock` it for the
//! lifetime of the daemon; if another process already holds the flock the
//! daemon refuses to start.  On Drop the lock is released (kernel does it on
//! close) and the lock file is best-effort removed.
//!
//! A pid file is written next to the lock so external tools can identify the
//! running daemon.  Stale pid files (no live process at that pid) are removed
//! on startup.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::DaemonError;

/// Companion `<socket>.lock` file used by `flock`.  Acquire on `start`,
/// release on `Drop`.  `pid_path()` is the file storing the daemon PID for
/// out-of-band introspection.
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
    lock_path: PathBuf,
    pid_path: PathBuf,
}

impl InstanceLock {
    /// Try to become the unique daemon instance for `socket_path`.
    ///
    /// On success, returns the lock and writes the current PID to
    /// `<socket>.pid`.  On failure (someone else holds the flock),
    /// returns [`DaemonError::AlreadyRunning`] reading the live PID from
    /// the pid file when possible.
    pub fn acquire(socket_path: &Path) -> Result<Self, DaemonError> {
        let lock_path = lock_path_for(socket_path);
        let pid_path = pid_path_for(socket_path);

        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;

        // Try non-blocking flock.  If another process already owns it, we
        // get Locked and surface a structured "already running" error.
        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let pid = read_pid(&pid_path).unwrap_or(0);
                return Err(DaemonError::AlreadyRunning { pid });
            }
            Err(e) => return Err(DaemonError::Io(e)),
        }

        // Stale pid files are confusing for `pgrep`-style tools — clear
        // before writing the live pid.
        let _ = std::fs::remove_file(&pid_path);
        let mut pid_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&pid_path)?;
        pid_file.write_all(std::process::id().to_string().as_bytes())?;
        pid_file.write_all(b"\n")?;
        pid_file.sync_all()?;

        Ok(InstanceLock {
            file,
            lock_path,
            pid_path,
        })
    }

    /// PID stored in the companion pid file (0 if absent).
    pub fn pid(&self) -> u32 {
        read_pid(&self.pid_path).unwrap_or(0)
    }

    /// Lock file path (for tests / diagnostics).
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Pid file path (for tests / diagnostics).
    pub fn pid_path(&self) -> &Path {
        &self.pid_path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Releasing the lock explicitly is not strictly necessary — closing
        // the file does it — but it gives us a hook to log failures without
        // making Drop noisy.
        let _ = FileExt::unlock(&self.file);
        let _ = std::fs::remove_file(&self.pid_path);
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

fn lock_path_for(socket_path: &Path) -> PathBuf {
    let mut s = socket_path.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

fn pid_path_for(socket_path: &Path) -> PathBuf {
    let mut s = socket_path.as_os_str().to_owned();
    s.push(".pid");
    PathBuf::from(s)
}

fn read_pid(path: &Path) -> Result<u32, std::io::Error> {
    let raw = std::fs::read_to_string(path)?;
    raw.trim().parse::<u32>().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("pid file `{path:?}` not numeric: {e}"),
        )
    })
}
