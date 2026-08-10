//! Configuration: omp path / data dir / model selection.
//!
//! Implemented in M1.7 (TOML + env fallback).
//!
//! M1.7: config struct + loader.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// Configuration for omenic.
#[derive(Debug, Clone)]
pub struct Config {
    /// Path to the omp binary.
    pub omp_path: PathBuf,
    /// Directory for omenic data storage.
    pub data_dir: PathBuf,
    /// Model name to use.
    pub model: String,
}

/// Errors that can occur during config loading.
#[derive(Debug)]
pub enum ConfigError {
    /// I/O error (e.g., reading config file).
    Io(std::io::Error),
    /// TOML parsing error.
    Toml(toml::de::Error),
    /// Generic parse error with context.
    #[allow(dead_code)] // kept for future manual parsers; not hit by toml path
    Parse(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "I/O error: {}", e),
            ConfigError::Toml(e) => write!(f, "TOML parse error: {}", e),
            ConfigError::Parse(s) => write!(f, "Config parse error: {}", s),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConfigError::Io(e) => Some(e),
            ConfigError::Toml(e) => Some(e),
            ConfigError::Parse(_) => None,
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::Toml(e)
    }
}

impl Config {
    /// Load configuration with priority: env > TOML file > defaults.
    pub fn load() -> Result<Config, ConfigError> {
        // Start with defaults
        let mut config = Config {
            omp_path: PathBuf::from("omp"),
            data_dir: PathBuf::from("./omenic-data"),
            model: String::from("default"),
        };

        // Load from TOML file (./omenic.toml in CWD); missing file is not an error.
        let toml_path = PathBuf::from("./omenic.toml");
        if let Ok(content) = std::fs::read_to_string(&toml_path) {
            let toml_config: TomlConfig = toml::from_str(&content)?;
            config = toml_config.merge_into(config);
        }
        // missing toml is fine (skip); other I/O errors propagate below as needed.

        // Environment variables override everything
        if let Ok(v) = env::var("OMENIC_OMP_PATH") {
            config.omp_path = PathBuf::from(v);
        }
        if let Ok(v) = env::var("OMENIC_DATA_DIR") {
            config.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = env::var("OMENIC_MODEL") {
            config.model = v;
        }

        Ok(config)
    }
}

/// Internal TOML config struct for deserialization (all fields optional).
#[derive(Debug, Default, serde::Deserialize)]
struct TomlConfig {
    omp_path: Option<String>,
    data_dir: Option<String>,
    model: Option<String>,
}

impl TomlConfig {
    fn merge_into(self, mut base: Config) -> Config {
        if let Some(v) = self.omp_path {
            base.omp_path = PathBuf::from(v);
        }
        if let Some(v) = self.data_dir {
            base.data_dir = PathBuf::from(v);
        }
        if let Some(v) = self.model {
            base.model = v;
        }
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    // rustc 1.97 nightly marks env::set_var/remove_var as unsafe (data race risk).
    // Tests must not run in parallel for env mutation; serialize via Mutex.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn clear_env() {
        unsafe {
            env::remove_var("OMENIC_OMP_PATH");
            env::remove_var("OMENIC_DATA_DIR");
            env::remove_var("OMENIC_MODEL");
        }
    }
    fn set_env(omp: Option<&str>, data: Option<&str>, model: Option<&str>) {
        unsafe {
            if let Some(v) = omp {
                env::set_var("OMENIC_OMP_PATH", v);
            }
            if let Some(v) = data {
                env::set_var("OMENIC_DATA_DIR", v);
            }
            if let Some(v) = model {
                env::set_var("OMENIC_MODEL", v);
            }
        }
    }

    // Restore original CWD + clean omenic.toml on drop.
    struct DirGuard {
        orig: PathBuf,
        tmp: PathBuf,
    }
    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.orig);
            let _ = fs::remove_file(self.tmp.join("omenic.toml"));
        }
    }

    // Move CWD into a unique temp subdir so ./omenic.toml resolution is isolated.
    fn tmp_dir(id: &str) -> DirGuard {
        clear_env();
        let orig = env::current_dir().unwrap();
        let tmp = env::temp_dir().join(format!("omenic-test-{}-{}", id, std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let _ = env::set_current_dir(&tmp);
        DirGuard { orig, tmp }
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn default_no_file_no_env() {
        let _g = lock();
        let _d = tmp_dir("default");
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("omp"));
        assert_eq!(config.data_dir, PathBuf::from("./omenic-data"));
        assert_eq!(config.model, "default");
    }

    #[test]
    fn env_override() {
        let _g = lock();
        let _d = tmp_dir("env1");
        set_env(None, None, Some("claude"));
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("omp"));
        assert_eq!(config.data_dir, PathBuf::from("./omenic-data"));
        assert_eq!(config.model, "claude");
    }

    #[test]
    fn all_env_override() {
        let _g = lock();
        let _d = tmp_dir("envall");
        set_env(
            Some("/custom/omp"),
            Some("/custom/data"),
            Some("custom-model"),
        );
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("/custom/omp"));
        assert_eq!(config.data_dir, PathBuf::from("/custom/data"));
        assert_eq!(config.model, "custom-model");
    }

    #[test]
    fn toml_file_load() {
        let _g = lock();
        let _d = tmp_dir("tomlfull");
        fs::write(
            "./omenic.toml",
            "omp_path = \"/usr/local/bin/omp\"\ndata_dir = \"/var/lib/omenic\"\nmodel = \"claude-opus-4-6\"\n",
        )
        .unwrap();
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("/usr/local/bin/omp"));
        assert_eq!(config.data_dir, PathBuf::from("/var/lib/omenic"));
        assert_eq!(config.model, "claude-opus-4-6");
    }

    #[test]
    fn toml_partial() {
        let _g = lock();
        let _d = tmp_dir("tomlpart");
        fs::write(
            "./omenic.toml",
            "omp_path = \"/toml/omp\"\nmodel = \"toml-model\"\n",
        )
        .unwrap();
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("/toml/omp"));
        assert_eq!(config.data_dir, PathBuf::from("./omenic-data"));
        assert_eq!(config.model, "toml-model");
    }

    #[test]
    fn toml_plus_env() {
        let _g = lock();
        let _d = tmp_dir("tomlenv");
        set_env(Some("/env/omp"), None, Some("env-model"));
        fs::write(
            "./omenic.toml",
            "omp_path = \"/toml/omp\"\ndata_dir = \"/toml/data\"\nmodel = \"toml-model\"\n",
        )
        .unwrap();
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("/env/omp"));
        assert_eq!(config.data_dir, PathBuf::from("/toml/data"));
        assert_eq!(config.model, "env-model");
    }

    #[test]
    fn missing_toml_ignored() {
        let _g = lock();
        let _d = tmp_dir("missing");
        // No omenic.toml present; load() must skip (NotFound) and use defaults.
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("omp"));
        assert_eq!(config.data_dir, PathBuf::from("./omenic-data"));
        assert_eq!(config.model, "default");
    }
}
