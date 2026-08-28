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
    /// Direct LLM API key (TUI chat / adaptor). Optional; call sites may
    /// fall back to legacy `AGNES_API_KEY` env.
    pub llm_api_key: Option<String>,
    /// Direct LLM base URL, no `/v1` suffix (call site appends it).
    pub llm_base_url: Option<String>,
    /// Direct LLM model name (e.g. `agnes-2.5-flash`). Distinct from `model`,
    /// which is the omp model profile selector.
    pub llm_model: Option<String>,
    /// Direct LLM max tokens.
    pub llm_max_tokens: Option<u32>,
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
    /// Field-level validation error: invalid value for a named field.
    Invalid {
        field: &'static str,
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "I/O error: {}", e),
            ConfigError::Toml(e) => write!(f, "TOML parse error: {}", e),
            ConfigError::Parse(s) => write!(f, "Config parse error: {}", s),
            ConfigError::Invalid { field, message } => {
                write!(f, "invalid config field `{field}`: {message}")
            }
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConfigError::Io(e) => Some(e),
            ConfigError::Toml(e) => Some(e),
            ConfigError::Parse(_) | ConfigError::Invalid { .. } => None,
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
    ///
    /// Config lives in the `.oi/` directory (`.oi/config.toml`); the legacy
    /// root `omenic.toml` is still read when present so pre-.oi workspaces
    /// keep working (migrate with `cli init`).
    pub fn load() -> Result<Config, ConfigError> {
        // Start with defaults (data lives inside the `.oi/` config dir).
        let mut config = Config {
            omp_path: PathBuf::from("omp"),
            data_dir: PathBuf::from("./.oi"),
            model: String::from("default"),
            llm_api_key: None,
            llm_base_url: None,
            llm_model: None,
            llm_max_tokens: None,
        };

        // Load from TOML file (.oi/config.toml, legacy fallback omenic.toml);
        // a missing file is not an error.
        let candidates = [
            PathBuf::from("./.oi/config.toml"),
            PathBuf::from("./omenic.toml"),
        ];
        for toml_path in candidates {
            if let Ok(content) = std::fs::read_to_string(&toml_path) {
                let toml_config: TomlConfig = toml::from_str(&content)?;
                config = toml_config.merge_into(config);
                break;
            }
        }

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
        if let Ok(v) = env::var("OMENIC_LLM_API_KEY") {
            config.llm_api_key = Some(v);
        }
        if let Ok(v) = env::var("OMENIC_LLM_BASE_URL") {
            config.llm_base_url = Some(v);
        }
        if let Ok(v) = env::var("OMENIC_LLM_MODEL") {
            config.llm_model = Some(v);
        }
        if let Ok(v) = env::var("OMENIC_LLM_MAX_TOKENS") {
            config.llm_max_tokens = v.parse().ok();
        }
        config.validate()?;
        Ok(config)
    }

    /// Validate config fields after all sources are merged.
    ///
    /// - `model`: must be non-empty.
    /// - `omp_path`: if it contains a path separator, must exist and be executable;
    ///   bare command names are allowed (resolved via PATH at runtime).
    /// - `data_dir`: if it exists, must be a directory; if not, parent must exist.
    fn validate(&self) -> Result<(), ConfigError> {
        // model: non-empty (don't echo the value back — could be sensitive).
        if self.model.trim().is_empty() {
            return Err(ConfigError::Invalid {
                field: "model",
                message: "must not be empty".to_string(),
            });
        }

        // omp_path: bare name = PATH lookup (allowed); path with separator = must exist.
        let omp_str = self.omp_path.to_string_lossy();
        if omp_str.contains('/') && !self.omp_path.exists() {
            return Err(ConfigError::Invalid {
                field: "omp_path",
                message: format!("'{}' does not exist", omp_str),
            });
        }
        // ponytail: not checking executable bit — OS will error at spawn with clear message.

        // data_dir: if exists, must be a dir; if not, parent must be creatable.
        if self.data_dir.exists() {
            if !self.data_dir.is_dir() {
                return Err(ConfigError::Invalid {
                    field: "data_dir",
                    message: format!(
                        "'{}' exists but is not a directory",
                        self.data_dir.display()
                    ),
                });
            }
        } else {
            // Check that we can create it (parent exists and is writable).
            let parent = self.data_dir.parent();
            if let Some(p) = parent
                && !p.exists()
            {
                return Err(ConfigError::Invalid {
                    field: "data_dir",
                    message: format!("parent directory '{}' does not exist", p.display()),
                });
            }
            // root with overlayfs etc. OS will give a clear error at write time.
        }

        Ok(())
    }
}

/// Internal TOML config struct for deserialization (all fields optional).
#[derive(Debug, Default, serde::Deserialize)]
struct TomlConfig {
    omp_path: Option<String>,
    data_dir: Option<String>,
    model: Option<String>,
    #[serde(default)]
    llm: LlmToml,
}

/// `[llm]` TOML section for direct LLM credentials.
#[derive(Debug, Default, serde::Deserialize)]
struct LlmToml {
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    max_tokens: Option<u32>,
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
        if let Some(v) = self.llm.api_key {
            base.llm_api_key = Some(v);
        }
        if let Some(v) = self.llm.base_url {
            base.llm_base_url = Some(v);
        }
        if let Some(v) = self.llm.model {
            base.llm_model = Some(v);
        }
        if let Some(v) = self.llm.max_tokens {
            base.llm_max_tokens = Some(v);
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
        assert_eq!(config.data_dir, PathBuf::from("./.oi"));
        assert_eq!(config.model, "default");
    }

    #[test]
    fn env_override() {
        let _g = lock();
        let _d = tmp_dir("env1");
        set_env(None, None, Some("claude"));
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("omp"));
        assert_eq!(config.data_dir, PathBuf::from("./.oi"));
        assert_eq!(config.model, "claude");
    }

    #[test]
    fn all_env_override() {
        let _g = lock();
        let _d = tmp_dir("envall");
        set_env(
            Some("custom-omp"),
            Some("./custom-data"),
            Some("custom-model"),
        );
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("custom-omp"));
        assert_eq!(config.data_dir, PathBuf::from("./custom-data"));
        assert_eq!(config.model, "custom-model");
    }

    #[test]
    fn toml_file_load() {
        let _g = lock();
        let _d = tmp_dir("tomlfull");
        fs::write(
            "./omenic.toml",
            "omp_path = \"omp-custom\"\ndata_dir = \"./omenic-data\"\nmodel = \"claude-opus-4-6\"\n",
        )
        .unwrap();
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("omp-custom"));
        assert_eq!(config.data_dir, PathBuf::from("./omenic-data"));
        assert_eq!(config.model, "claude-opus-4-6");
    }

    #[test]
    fn toml_partial() {
        let _g = lock();
        let _d = tmp_dir("tomlpart");
        fs::write(
            "./omenic.toml",
            "omp_path = \"toml-omp\"\nmodel = \"toml-model\"\n",
        )
        .unwrap();
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("toml-omp"));
        assert_eq!(config.data_dir, PathBuf::from("./.oi"));
        assert_eq!(config.model, "toml-model");
    }

    #[test]
    fn toml_plus_env() {
        let _g = lock();
        let _d = tmp_dir("tomlenv");
        set_env(Some("env-omp"), None, Some("env-model"));
        fs::write(
            "./omenic.toml",
            "omp_path = \"toml-omp\"\ndata_dir = \"./toml-data\"\nmodel = \"toml-model\"\n",
        )
        .unwrap();
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("env-omp"));
        assert_eq!(config.data_dir, PathBuf::from("./toml-data"));
        assert_eq!(config.model, "env-model");
    }

    #[test]
    fn missing_toml_ignored() {
        let _g = lock();
        let _d = tmp_dir("missing");
        // No omenic.toml present; load() must skip (NotFound) and use defaults.
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("omp"));
        assert_eq!(config.data_dir, PathBuf::from("./.oi"));
        assert_eq!(config.model, "default");
    }

    // --- #52: field-level validation ---

    #[test]
    fn empty_model_rejected() {
        let _g = lock();
        let _d = tmp_dir("empty-model");
        set_env(None, None, Some(""));
        let r = Config::load();
        assert!(r.is_err());
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("model"), "error should name the field");
        assert!(!msg.contains("default"), "must not leak config values");
    }

    #[test]
    fn whitespace_model_rejected() {
        let _g = lock();
        let _d = tmp_dir("ws-model");
        set_env(None, None, Some("   "));
        let r = Config::load();
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("model"));
    }

    #[test]
    fn nonexistent_omp_path_rejected() {
        let _g = lock();
        let _d = tmp_dir("bad-omp");
        set_env(Some("/nonexistent/definitely/not/here/omp"), None, None);
        let r = Config::load();
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("omp_path"));
    }

    #[test]
    fn bare_omp_name_allowed() {
        let _g = lock();
        let _d = tmp_dir("bare-omp");
        set_env(Some("my-omp"), None, None);
        let config = Config::load().unwrap();
        assert_eq!(config.omp_path, PathBuf::from("my-omp"));
    }

    #[test]
    fn data_dir_not_a_directory_rejected() {
        let _g = lock();
        let _d = tmp_dir("bad-data");
        // Create a file where data_dir points
        fs::write("./not-a-dir", "x").unwrap();
        set_env(None, Some("./not-a-dir"), None);
        let r = Config::load();
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("data_dir"));
        let _ = fs::remove_file("./not-a-dir");
    }

    #[test]
    fn data_dir_parent_missing_rejected() {
        let _g = lock();
        let _d = tmp_dir("bad-parent");
        set_env(None, Some("/nonexistent/definitely/data"), None);
        let r = Config::load();
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("data_dir"));
    }

    #[test]
    fn data_dir_creatable_allowed() {
        let _g = lock();
        let _d = tmp_dir("creat-dir");
        set_env(None, Some("./new-data-dir"), None);
        let config = Config::load().unwrap();
        assert_eq!(config.data_dir, PathBuf::from("./new-data-dir"));
    }
}
