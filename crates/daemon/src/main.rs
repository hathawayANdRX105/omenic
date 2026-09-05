//! Headless omenic daemon binary entrypoint.
//!
//! This module provides the main entry point for the daemon binary,
//! following the daemon config pattern from server.rs.
//!
//! Signal handling:
//! - SIGINT (Ctrl-C) and SIGTERM trigger graceful shutdown via Daemon::shutdown()
//! - The Drop implementation handles cleanup if signals are not caught
//!
//! This avoids using systemd/Windows service integrations, keeping the
//! daemon portable and embeddable.

use config::Config;
use ctrlc;
use daemon::{Daemon, DaemonConfig, DaemonError};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Main entry point for the omenic daemon binary.
///
/// This function:
/// 1. Creates a DaemonConfig using Config::daemon_socket_path and Config::session_db_path
/// 2. Starts the daemon
/// 3. Blocks on the daemon until shutdown is triggered
/// 4. Cleans up on exit
fn main() -> Result<(), DaemonError> {
    let config = Config::load().map_err(DaemonError::Config)?;

    let cfg = DaemonConfig::from_config(&config)?;

    println!("Starting omenic daemon...");
    let mut daemon = Daemon::start(cfg)?;

    println!(
        "Daemon started on socket: {:?}",
        daemon.socket_addr().path()
    );
    println!("Daemon PID: {}", daemon.pid());

    // Shared flag to track if shutdown was requested via signal
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let shutdown_flag = Arc::clone(&shutdown_requested);

    // Set up signal handlers for SIGINT and SIGTERM
    ctrlc::set_handler(move || {
        shutdown_flag.store(true, Ordering::SeqCst);
    })
    .expect("Failed to set signal handler");

    println!("Daemon running. Press Ctrl-C to stop.");

    // Stop on either a process signal or a daemon.shutdown request.
    while !shutdown_requested.load(Ordering::SeqCst) && !daemon.is_shutdown_requested() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!("Shutdown requested, stopping daemon...");
    daemon.shutdown();
    drop(daemon);
    println!("Daemon stopped cleanly.");

    Ok(())
}
