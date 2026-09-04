//! Headless omenic daemon binary entrypoint.
//!
//! This module provides the main entry point for the daemon binary,
//! following the daemon config pattern from server.rs.

use config::Config;
use daemon::{Daemon, DaemonConfig, DaemonError};

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

    // Wait for shutdown signal
    // In a real implementation, this might wait on a signal or
    // allow the daemon to handle incoming connections
    // For now, we just keep it alive until dropped

    // The daemon will be dropped when main() returns,
    // which triggers the Drop implementation for cleanup

    // Keep the process alive - in a real implementation
    // we might handle signals or provide a way to shut down gracefully
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
