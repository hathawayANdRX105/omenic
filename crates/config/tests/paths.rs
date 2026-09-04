//! Integration tests for the platform runtime path helpers on `Config`.
//!
//! Exercises `Config::session_db_path` / `Config::daemon_socket_path`:
//!
//! 1. env override (when set, override wins; empty is treated as unset)
//! 2. platform default (XDG / HOME on Unix, APPDATA on Windows)
//! 3. clear error when the underlying env var is missing
//!
//! All tests must run serially because they mutate process env via
//! `unsafe { env::set_var / remove_var }`. The `ENV_LOCK` mutex guards
//! the whole binary so cargo's default test parallelism stays safe.

use std::env;
use std::path::PathBuf;
use std::sync::Mutex;

use config::Config;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Remove every env var the helpers read. Call this both at start and end
/// of each test so a panic mid-test never leaks state to siblings.
fn clear_runtime_env() {
    unsafe {
        env::remove_var("OMENIC_SESSION_DB");
        env::remove_var("OMENIC_DAEMON_SOCKET");
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("HOME");
        env::remove_var("APPDATA");
    }
}

/// Minimal `Config` — the path helpers don't read any field.
fn fixture_config() -> Config {
    Config {
        omp_path: PathBuf::from("omp"),
        data_dir: PathBuf::from("./.oi"),
        model: String::from("default"),
        llm_api_key: None,
        llm_base_url: None,
        llm_model: None,
        llm_max_tokens: None,
        mcp_servers: Vec::new(),
        memory_enabled: false,
        memory_dir: None,
    }
}

#[cfg(target_family = "unix")]
fn set_platform_dir(home_like: &str) {
    unsafe {
        env::remove_var("XDG_CONFIG_HOME");
        env::set_var("HOME", home_like);
    }
}

#[cfg(target_family = "windows")]
fn set_platform_dir(appdata: &str) {
    unsafe {
        env::set_var("APPDATA", appdata);
    }
}

/// Env override wins for `session_db_path`.
#[test]
fn session_db_env_override_wins() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    unsafe {
        env::set_var("OMENIC_SESSION_DB", "/tmp/custom-sessions.db");
    }
    let got = fixture_config()
        .session_db_path()
        .expect("OMENIC_SESSION_DB set → must succeed");
    assert_eq!(got, PathBuf::from("/tmp/custom-sessions.db"));
    clear_runtime_env();
}

/// Empty `OMENIC_SESSION_DB` is treated as unset (falls through to default).
#[test]
fn session_db_empty_env_falls_through() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    unsafe {
        env::set_var("OMENIC_SESSION_DB", "");
    }
    set_platform_dir(if cfg!(target_family = "unix") {
        "/tmp/home-empty-env"
    } else {
        r"C:\Users\fixture\AppData\Roaming"
    });
    let got = fixture_config()
        .session_db_path()
        .expect("platform env set → must succeed");
    let expected = if cfg!(target_family = "unix") {
        PathBuf::from("/tmp/home-empty-env/.config/omenic/sessions.db")
    } else {
        PathBuf::from(r"C:\Users\fixture\AppData\Roaming\omenic\sessions.db")
    };
    assert_eq!(got, expected);
    clear_runtime_env();
}

/// Platform default for `session_db_path` when only the platform env is set.
#[test]
fn session_db_platform_default() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    #[cfg(target_family = "unix")]
    {
        unsafe {
            env::set_var("XDG_CONFIG_HOME", "/etc/xdg-default-test");
        }
        let got = fixture_config()
            .session_db_path()
            .expect("XDG_CONFIG_HOME set → must succeed");
        assert_eq!(
            got,
            PathBuf::from("/etc/xdg-default-test/omenic/sessions.db")
        );
    }
    #[cfg(target_family = "windows")]
    {
        unsafe {
            env::set_var("APPDATA", r"C:\Users\fixture\AppData\Roaming");
        }
        let got = fixture_config()
            .session_db_path()
            .expect("APPDATA set → must succeed");
        assert_eq!(
            got,
            PathBuf::from(r"C:\Users\fixture\AppData\Roaming\omenic\sessions.db")
        );
    }
    clear_runtime_env();
}

/// XDG wins over HOME on Unix.
#[cfg(target_family = "unix")]
#[test]
fn session_db_xdg_overrides_home() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    unsafe {
        env::set_var("XDG_CONFIG_HOME", "/explicit/xdg");
        env::set_var("HOME", "/should/be/ignored");
    }
    let got = fixture_config()
        .session_db_path()
        .expect("both set → must succeed");
    assert_eq!(got, PathBuf::from("/explicit/xdg/omenic/sessions.db"));
    clear_runtime_env();
}

/// Empty `XDG_CONFIG_HOME` falls through to `$HOME/.config`.
#[cfg(target_family = "unix")]
#[test]
fn session_db_empty_xdg_falls_back_to_home() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    unsafe {
        env::set_var("XDG_CONFIG_HOME", "");
        env::set_var("HOME", "/home/empty-xdg");
    }
    let got = fixture_config()
        .session_db_path()
        .expect("HOME set → must succeed");
    assert_eq!(
        got,
        PathBuf::from("/home/empty-xdg/.config/omenic/sessions.db")
    );
    clear_runtime_env();
}

/// Missing both `XDG_CONFIG_HOME` and `HOME` on Unix surfaces a clear error.
#[cfg(target_family = "unix")]
#[test]
fn session_db_missing_unix_env_returns_error() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    // Belt-and-braces: clear HOME too (some CI inherits it).
    unsafe {
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("HOME");
    }
    let err = fixture_config()
        .session_db_path()
        .expect_err("missing XDG_CONFIG_HOME & HOME must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("XDG_CONFIG_HOME") || msg.contains("HOME"),
        "error message should name the missing var, got: {msg}"
    );
    clear_runtime_env();
}

/// Daemon socket env override wins.
#[test]
fn daemon_socket_env_override_wins() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    unsafe {
        env::set_var("OMENIC_DAEMON_SOCKET", "/tmp/custom.sock");
    }
    let got = fixture_config()
        .daemon_socket_path()
        .expect("OMENIC_DAEMON_SOCKET set → must succeed");
    assert_eq!(got, PathBuf::from("/tmp/custom.sock"));
    clear_runtime_env();
}

/// Daemon socket falls through to the platform default when env is unset.
#[test]
fn daemon_socket_platform_default() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    set_platform_dir(if cfg!(target_family = "unix") {
        "/home/sock-default"
    } else {
        r"C:\Users\fixture\AppData\Roaming"
    });
    let got = fixture_config()
        .daemon_socket_path()
        .expect("platform env set → must succeed");
    let expected = if cfg!(target_family = "unix") {
        PathBuf::from("/home/sock-default/.config/omenic/daemon.sock")
    } else {
        PathBuf::from(r"C:\Users\fixture\AppData\Roaming\omenic\daemon.sock")
    };
    assert_eq!(got, expected);
    clear_runtime_env();
}

/// Each helper honours its own env var — overriding one does NOT change the other.
#[test]
fn session_db_and_socket_env_vars_are_independent() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_runtime_env();
    set_platform_dir(if cfg!(target_family = "unix") {
        "/home/indep"
    } else {
        r"C:\indep"
    });
    let cfg = fixture_config();

    // Override only the DB; socket must follow the platform default.
    unsafe {
        env::set_var("OMENIC_SESSION_DB", "/var/omenic/sessions.db");
    }
    let db = cfg.session_db_path().unwrap();
    let sock = cfg.daemon_socket_path().unwrap();
    assert_eq!(db, PathBuf::from("/var/omenic/sessions.db"));
    let expected_sock = if cfg!(target_family = "unix") {
        PathBuf::from("/home/indep/.config/omenic/daemon.sock")
    } else {
        PathBuf::from(r"C:\indep\omenic\daemon.sock")
    };
    assert_eq!(sock, expected_sock);
    clear_runtime_env();
}
