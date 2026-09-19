//! Configuration: omp path / data dir / model selection.
//!
//! Implemented in M1.7 (TOML + env fallback).
//!
//! M1.7: config struct + loader.

use std::collections::HashSet;
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
    /// Fallback LLM providers tried in order after the primary `[llm]`
    /// provider fails before emitting any content. Empty = no waterfall.
    pub llm_fallbacks: Vec<LlmFallbackConfig>,
    /// External MCP servers to spawn for extra tools. Empty by default —
    /// MCP is opt-in and nothing is spawned unless the user lists a server.
    pub mcp_servers: Vec<McpServerConfig>,
    /// Out-of-process subagent providers (`[[subagent.providers]]`): spawned
    /// ACP child agents the daemon can delegate runs to. Empty by default —
    /// subagents are opt-in, same as `mcp_servers`.
    pub subagent_providers: Vec<SubagentProviderConfig>,

    /// Persistent local memory. Off by default: nothing is written to disk
    /// until the user opts in via `[memory] enabled = true`.
    pub memory_enabled: bool,
    /// Override for the memory directory; `None` = `data_dir/memory`.
    pub memory_dir: Option<PathBuf>,
    /// Working directory the daemon's orbit engine treats as the session
    /// root: `AGENTS.md` workspace-instruction discovery walks up this
    /// directory's ancestor chain. Defaults to the daemon startup directory
    /// (`.oi/config.toml` `[daemon] cwd`, or `OMENIC_CWD`).
    pub cwd: PathBuf,
    /// Cap on LLM round-trips per run for the orbit engine. `None` = the
    /// loop's own default (`[daemon] max_turns`, or `OMENIC_MAX_TURNS`).
    pub max_turns: Option<usize>,
}

/// One external MCP server: a stdio child process (`command`), or a running
/// HTTP endpoint (`url`) for the streamable-HTTP transport.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct McpServerConfig {
    /// Short handle used to namespace this server's tool names.
    pub name: String,
    /// Executable to spawn. Omit for HTTP servers (use `url` instead).
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments passed to `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables for the child.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// HTTP endpoint for the streamable-HTTP transport.
    #[serde(default)]
    pub url: Option<String>,
    /// Per-call timeout in ms. `None` = crate default (`MCP_TIMEOUT`).
    #[serde(default)]
    pub tool_call_timeout_ms: Option<u64>,
    /// Reconnect policy. `None` = default (500ms → 30s, 10 attempts).
    #[serde(default)]
    pub reconnect: Option<McpReconnectConfig>,
    /// Working directory for the stdio child process. `None` = inherit the
    /// daemon/CLI process cwd.
    #[serde(default)]
    pub cwd: Option<String>,
    /// When true, a server that fails to start/handshake aborts the whole MCP
    /// bring-up instead of being skipped. Default false (skip + log).
    #[serde(default)]
    pub fail_on_startup_error: Option<bool>,
}

/// Per-server reconnect/backoff tuning. Absent fields fall back to the
/// defaults in the mcp crate (`ReconnectPolicy::default`).
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct McpReconnectConfig {
    #[serde(default)]
    pub initial_delay_ms: Option<u64>,
    #[serde(default)]
    pub max_delay_ms: Option<u64>,
    #[serde(default)]
    pub max_attempts: Option<u32>,
}

/// Permission policy for ACP child requests: how the client auto-answers
/// the child agent's permission prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentPermission {
    Allow,
    Reject,
}

impl Default for SubagentPermission {
    fn default() -> Self {
        Self::Reject
    }
}

/// One out-of-process subagent provider (`[[subagent.providers]]`): a spawned
/// ACP child agent the daemon can delegate runs to. The transport is ACP for
/// now; the spawn fields (`command`/`args`/`env`/`cwd`) are kept generic so a
/// future transport reuses them without a config migration.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct SubagentProviderConfig {
    /// Registry name; must not collide with the built-in `fork`.
    pub name: String,
    /// Executable to spawn (the child ACP agent). Required.
    pub command: String,
    /// Arguments passed to `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra env for the child, merged over the scrubbed parent env.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Child cwd. `None` = inherit the daemon session cwd.
    #[serde(default)]
    pub cwd: Option<String>,
    /// How the client auto-answers the child's permission prompts.
    #[serde(default)]
    pub permission: SubagentPermission,
    /// SIGTERM→SIGKILL grace on dispose, ms. `None` = provider default (3000).
    #[serde(default)]
    pub dispose_grace_ms: Option<u64>,
    /// stdin-EOF quiesce window on dispose, ms. `None` = provider default (6000).
    #[serde(default)]
    pub dispose_eof_grace_ms: Option<u64>,
}

/// One fallback LLM provider (`[[llm.fallbacks]]`), tried in listed order
/// after the primary `[llm]` provider fails without emitting content.
/// `max_tokens` absent = inherit nothing (the provider's request carries
/// no `max_tokens`); set it explicitly to bound the fallback's output.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct LlmFallbackConfig {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
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
            llm_fallbacks: Vec::new(),
            mcp_servers: Vec::new(),
            subagent_providers: Vec::new(),
            memory_enabled: false,
            memory_dir: None,
            cwd: Self::default_cwd(),
            max_turns: None,
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
        config
            .memory_dir
            .get_or_insert_with(|| config.data_dir.join("memory"));
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
        if let Ok(v) = env::var("OMENIC_CWD") {
            config.cwd = PathBuf::from(v);
        }
        if let Ok(v) = env::var("OMENIC_MAX_TURNS") {
            config.max_turns = v.parse().ok();
        }
        config.validate()?;
        Ok(config)
    }

    /// Daemon session working directory: the orbit engine searches this
    /// directory's ancestor chain for `AGENTS.md`. Defaults to the process
    /// working directory — the daemon startup dir — never to a silent
    /// `None`, so a workspace without an explicit `cwd` still gets instruction
    /// discovery from where it was launched.
    fn default_cwd() -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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

        // memory_dir: when enabled, the nearest existing ancestor must be a directory.
        // (When disabled, a stale path is allowed.)
        if self.memory_enabled
            && let Some(dir) = self.memory_dir.as_deref()
            && let Some(p) = dir.ancestors().find(|a| a.exists())
            && !p.is_dir()
        {
            return Err(ConfigError::Invalid {
                field: "memory_dir",
                message: format!("'{}' exists but is not a directory", p.display()),
            });
        }

        // mcp servers: a listed server must be startable — an entry with no
        // name or no command can only fail later at spawn time.
        // ponytail: uniqueness inside the same loop — no second pass needed.
        let mut seen = HashSet::new();
        for s in &self.mcp_servers {
            if s.name.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "mcp.servers.name",
                    message: "must not be empty".to_string(),
                });
            }
            // A server is startable if it names a command or a url; an entry
            // with neither can only fail later at spawn/connect time.
            let has_command = s
                .command
                .as_deref()
                .map(str::trim)
                .is_some_and(|c| !c.is_empty());
            if !has_command && s.url.as_deref().map(str::trim).is_none_or(str::is_empty) {
                return Err(ConfigError::Invalid {
                    field: "mcp.servers",
                    message: format!("server '{}' has neither command nor url", s.name),
                });
            }
            // Ambiguous transport: the mcp crate prefers `url` (streamable
            // HTTP) and silently ignores `command` when both are set. Warn —
            // don't reject — so a legacy config keeps loading while the
            // unused `command` stops being a silent surprise.
            let trimmed_url = s.url.as_deref().map(str::trim);
            let has_url = trimmed_url.is_some_and(|u| !u.is_empty());
            if has_command && has_url {
                eprintln!(
                    "warn: mcp server `{}` has both `url` and `command`; the HTTP transport (url) takes precedence, `command` is ignored",
                    s.name
                );
            }
            // A non-http(s) scheme cannot be dialed by the HTTP transport;
            // warn here so the eventual connect failure points back at the
            // config line. Still not a rejection: validate stays `Ok`.
            if let Some(url) = trimmed_url.filter(|u| !u.is_empty())
                && !url.starts_with("http://")
                && !url.starts_with("https://")
            {
                eprintln!(
                    "warn: mcp server `{}` url `{url}` does not start with http(s)://; connection will likely fail",
                    s.name
                );
            }
            // Checked last so a duplicate error only names an otherwise-valid server.
            if !seen.insert(s.name.clone()) {
                return Err(ConfigError::Invalid {
                    field: "mcp.servers.name",
                    message: format!("duplicate server name '{}'", s.name),
                });
            }
        }

        // subagent providers: a listed provider must be startable, and must
        // not shadow `fork` — the built-in in-process provider. A user config
        // that redefines `fork` would silently route runs to the wrong place.
        for (i, p) in self.subagent_providers.iter().enumerate() {
            if p.name.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "subagent.providers",
                    message: format!("entry {i}: name must not be empty"),
                });
            }
            if p.name == "fork" {
                return Err(ConfigError::Invalid {
                    field: "subagent.providers",
                    message: format!(
                        "entry {i}: name 'fork' is the built-in in-process provider and cannot be overridden"
                    ),
                });
            }
            if p.command.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "subagent.providers",
                    message: format!("entry {i}: an out-of-process provider requires a command"),
                });
            }
        }

        // cwd: when it exists it must be a directory — instruction discovery
        // walks its ancestor chain, and a file would never hold AGENTS.md.
        if self.cwd.exists() && !self.cwd.is_dir() {
            return Err(ConfigError::Invalid {
                field: "cwd",
                message: format!("'{}' exists but is not a directory", self.cwd.display()),
            });
        }
        // max_turns: 0 would end every run before its first LLM round-trip.
        if let Some(n) = self.max_turns
            && n == 0
        {
            return Err(ConfigError::Invalid {
                field: "max_turns",
                message: "must be at least 1".to_string(),
            });
        }

        // llm fallbacks: every field is `Option` and inherits from the
        // primary provider, so an entry that overrides *nothing* replays the
        // same provider verbatim — a no-op retry rather than a fallback.
        // Warn (never reject): inheritance is legal, this only spots the
        // entry that forgot to say what it changes.
        for (idx, f) in self.llm_fallbacks.iter().enumerate() {
            if f.base_url.is_none()
                && f.api_key.is_none()
                && f.model.is_none()
                && f.max_tokens.is_none()
            {
                eprintln!(
                    "warn: [[llm.fallbacks]] entry #{idx} overrides nothing; it retries the primary provider unchanged"
                );
            }
        }

        Ok(())
    }

    /// Resolve the absolute path to the session database file.
    ///
    /// Priority:
    /// 1. `OMENIC_SESSION_DB` env var (verbatim).
    /// 2. Platform-specific config directory + `omenic/sessions.db`.
    ///
    /// Returns `ConfigError::Invalid` when the platform-default lookup needs
    /// an environment variable that is missing (e.g. `XDG_CONFIG_HOME` set
    /// to empty, or `HOME`/`APPDATA` unset on Unix/macOS/Windows).
    pub fn session_db_path(&self) -> Result<PathBuf, ConfigError> {
        match env::var_os("OMENIC_SESSION_DB") {
            Some(v) if !v.is_empty() => Ok(PathBuf::from(v)),
            _ => Ok(platform_config_dir()?.join("omenic").join("sessions.db")),
        }
    }

    /// Resolve the absolute path to the daemon Unix-domain / named-pipe socket.
    ///
    /// Priority:
    /// 1. `OMENIC_DAEMON_SOCKET` env var (verbatim).
    /// 2. Platform-specific config directory + `omenic/daemon.sock`.
    ///
    /// Same error semantics as [`Self::session_db_path`].
    pub fn daemon_socket_path(&self) -> Result<PathBuf, ConfigError> {
        match env::var_os("OMENIC_DAEMON_SOCKET") {
            Some(v) if !v.is_empty() => Ok(PathBuf::from(v)),
            _ => Ok(platform_config_dir()?.join("omenic").join("daemon.sock")),
        }
    }
}

/// Resolve the platform-specific user config directory root.
///
/// Unix/Linux: `$XDG_CONFIG_HOME` if non-empty, else `$HOME/.config`.
/// macOS:     `$HOME/Library/Application Support`.
/// Windows:   `%APPDATA%` (verbatim).
///
/// Returns `ConfigError::Invalid` if the underlying env var is missing or
/// empty (no silent fallback to the current working directory).
fn platform_config_dir() -> Result<PathBuf, ConfigError> {
    #[cfg(target_family = "unix")]
    {
        if let Some(v) = env::var_os("XDG_CONFIG_HOME")
            && !v.is_empty()
        {
            return Ok(PathBuf::from(v));
        }
        match env::var_os("HOME") {
            Some(v) if !v.is_empty() => Ok(PathBuf::from(v).join(".config")),
            _ => Err(ConfigError::Invalid {
                field: "platform_config_dir",
                message: "neither XDG_CONFIG_HOME nor HOME is set".to_string(),
            }),
        }
    }
    #[cfg(target_family = "windows")]
    {
        match env::var_os("APPDATA") {
            Some(v) if !v.is_empty() => Ok(PathBuf::from(v)),
            _ => Err(ConfigError::Invalid {
                field: "platform_config_dir",
                message: "APPDATA is not set".to_string(),
            }),
        }
    }
    #[cfg(not(any(target_family = "unix", target_family = "windows")))]
    {
        // ponytail: no spec'd target here; surface a clear error rather than silently fall back.
        Err(ConfigError::Invalid {
            field: "platform_config_dir",
            message: "unsupported target family for platform config directory".to_string(),
        })
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
    #[serde(default)]
    mcp: McpToml,
    #[serde(default)]
    memory: MemoryToml,
    #[serde(default)]
    daemon: DaemonToml,
    #[serde(default)]
    subagent: SubagentToml,
}

#[derive(Debug, Default, serde::Deserialize)]
struct MemoryToml {
    enabled: Option<bool>,
    dir: Option<String>,
}

/// `[daemon]` TOML section: orbit-engine session knobs.
#[derive(Debug, Default, serde::Deserialize)]
struct DaemonToml {
    /// Session working directory (`AGENTS.md` discovery root).
    cwd: Option<String>,
    /// Cap on LLM round-trips per run.
    max_turns: Option<u64>,
}

/// `[llm]` TOML section for direct LLM credentials. `fallbacks` is a list
/// of `[[llm.fallbacks]]` tables (waterfall, in listed order).
#[derive(Debug, Default, serde::Deserialize)]
struct LlmToml {
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    max_tokens: Option<u32>,
    #[serde(default)]
    fallbacks: Vec<LlmFallbackConfig>,
}

/// `[mcp]` TOML section. `servers` is a list of `[[mcp.servers]]` tables.
#[derive(Debug, Default, serde::Deserialize)]
struct McpToml {
    #[serde(default)]
    servers: Vec<McpServerConfig>,
}

/// `[subagent]` TOML section. `providers` is a list of
/// `[[subagent.providers]]` tables.
#[derive(Debug, Default, serde::Deserialize)]
struct SubagentToml {
    #[serde(default)]
    providers: Vec<SubagentProviderConfig>,
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
        // Same empty-does-not-override semantics as `mcp.servers`.
        if !self.llm.fallbacks.is_empty() {
            base.llm_fallbacks = self.llm.fallbacks;
        }
        if !self.mcp.servers.is_empty() {
            // Normalize the transport fields here, at the merge boundary.
            // `validate` trims only to *test* a value, so a padded
            // `command = " npx "` would pass there and still be handed to
            // `Command::new` with its spaces — which then fails to spawn.
            base.mcp_servers = self
                .mcp
                .servers
                .into_iter()
                .map(|mut s| {
                    s.name = s.name.trim().to_string();
                    s.command = s.command.map(|c| c.trim().to_string());
                    s.url = s.url.map(|u| u.trim().to_string());
                    s
                })
                .collect();
        }
        // Same empty-does-not-override + merge-boundary trim as
        // `mcp.servers` above: `validate` only *tests* the trimmed value, so
        // a padded `command = " /bin/echo "` would pass and still spawn with
        // its spaces.
        if !self.subagent.providers.is_empty() {
            base.subagent_providers = self
                .subagent
                .providers
                .into_iter()
                .map(|mut p| {
                    p.name = p.name.trim().to_string();
                    p.command = p.command.trim().to_string();
                    p.cwd = p.cwd.map(|c| c.trim().to_string());
                    p
                })
                .collect();
        }
        if let Some(v) = self.memory.enabled {
            base.memory_enabled = v;
        }
        if let Some(v) = self.memory.dir {
            base.memory_dir = Some(PathBuf::from(v));
        }
        if let Some(v) = self.daemon.cwd {
            base.cwd = PathBuf::from(v);
        }
        if let Some(v) = self.daemon.max_turns {
            // `as usize` would silently truncate on a 32-bit host; the TOML
            // field is u64 because TOML integers are. usize::try_from keeps a
            // too-large value out of the config instead of wrapping it.
            base.max_turns = usize::try_from(v).ok();
        }
        base
    }
}
