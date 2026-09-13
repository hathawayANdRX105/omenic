//! Socket abstraction.  Unix-only today; Windows is a stub that surfaces
//! `UnsupportedPlatform` so the rest of the daemon can stay platform-neutral.

use std::os::unix::fs::FileTypeExt;

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use crate::DaemonError;

/// Address for a daemon socket.  On Unix this is the filesystem path of the
/// Unix-domain socket; on Windows it is currently unsupported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketAddr {
    path: std::path::PathBuf,
}

impl SocketAddr {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        SocketAddr { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(target_family = "unix")]
pub mod platform {
    //! Unix-domain-socket implementation.
    //!
    //! `Listener` binds + cleans stale sockets + accepts; `Connection`
    //! reads/writes newline-delimited JSON frames.

    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::{Duration, Instant};

    use super::*;

    /// Bound, listening Unix-domain socket.
    pub struct Listener {
        inner: UnixListener,
        path: std::path::PathBuf,
    }

    impl Listener {
        /// Bind to `path`.  Removes any stale socket file left over from a
        /// crashed daemon so we don't fail with `AddrInUse`.
        pub fn bind(path: &Path) -> Result<Self, DaemonError> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Stale socket cleanup.  We only unlink if it is a socket file;
            // a regular file at this path is treated as user error.
            match std::fs::metadata(path) {
                Ok(meta) if meta.file_type().is_socket() => {
                    let _ = std::fs::remove_file(path);
                }
                Ok(_) => {
                    return Err(DaemonError::Io(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("socket path `{path:?}` exists and is not a socket"),
                    )));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(DaemonError::Io(e)),
            }

            let inner = UnixListener::bind(path)?;
            Ok(Listener {
                inner,
                path: path.to_path_buf(),
            })
        }

        /// `accept()` with a deadline — used by the accept loop so the
        /// shutdown flag gets observed promptly.
        ///
        /// Returns `Ok(None)` on timeout; the caller decides what to do.
        pub fn accept_timeout(&self, dur: Duration) -> Result<Option<Connection>, DaemonError> {
            self.inner.set_nonblocking(true)?;
            let start = Instant::now();
            let conn = loop {
                match self.inner.accept() {
                    Ok((stream, _addr)) => break Some(stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if start.elapsed() >= dur {
                            break None;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => {
                        let _ = self.inner.set_nonblocking(false);
                        return Err(DaemonError::Io(e));
                    }
                }
            };
            self.inner.set_nonblocking(false)?;
            Ok(conn.map(Connection::new))
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Best-effort socket cleanup.  If unlink fails (e.g. another
            // process raced and bound the path), we just leave it — a
            // future start will surface the error.
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// One accepted connection.  Owned exclusively by the accept loop;
    /// closes on Drop.
    pub struct Connection {
        reader: BufReader<UnixStream>,
        writer: BufWriter<UnixStream>,
    }

    impl Connection {
        pub(crate) fn new(stream: UnixStream) -> Self {
            // We can't easily split a `UnixStream` — clone the fd by
            // duplicating it and put one half on the reader, the other on
            // the writer.
            let read_half = stream.try_clone().expect("clone unix stream");
            Connection {
                reader: BufReader::new(read_half),
                writer: BufWriter::new(stream),
            }
        }

        /// Read one newline-delimited JSON frame as raw text.  Returns
        /// `Ok(None)` on EOF.
        pub fn read_frame(&mut self) -> Result<Option<String>, DaemonError> {
            let mut buf = Vec::with_capacity(1024);
            let n = self.reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                return Ok(None);
            }
            // Strip the trailing newline for the caller's convenience.
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
        }

        /// Write one newline-delimited JSON frame.
        pub fn write_frame(&mut self, line: &str) -> Result<(), DaemonError> {
            self.writer.write_all(line.as_bytes())?;
            self.writer.write_all(b"\n")?;
            self.writer.flush()?;
            Ok(())
        }
    }
}

#[cfg(not(target_family = "unix"))]
pub mod platform {
    use super::*;

    /// Stub: not implemented on non-Unix yet.
    pub struct Listener;

    impl Listener {
        pub fn bind(_path: &Path) -> Result<Self, DaemonError> {
            Err(DaemonError::UnsupportedPlatform)
        }
    }

    pub struct Connection;

    impl Connection {
        pub fn read_frame(&mut self) -> Result<Option<String>, DaemonError> {
            Err(DaemonError::UnsupportedPlatform)
        }
        pub fn write_frame(&mut self, _line: &str) -> Result<(), DaemonError> {
            Err(DaemonError::UnsupportedPlatform)
        }
    }
}

pub use platform::{Connection, Listener};
