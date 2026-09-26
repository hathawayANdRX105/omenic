//! CLI command layer.
//!
//! Subcommands: task add/done/status/update/delete/list/show, plan, run,
//! steer, abort, ready, blocked, compact, init, dep.
//! Parsing uses clap (derive API); `--json` selects machine-readable output.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use config::Config;
use store::store::Store;

mod commands;

use commands::*;
// clap CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "oi", about = "Task-driven agent orchestrator")]
struct Cli {
    /// Output JSON instead of human-readable text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Task management commands
    Task {
        #[command(subcommand)]
        sub: TaskCmd,
    },
    /// Show task details (alias for `task show`)
    Show { id: String },
    /// Render the task tree / Graphviz DOT / board view
    Plan {
        /// Output Graphviz DOT format
        #[arg(long)]
        dot: bool,
        #[command(subcommand)]
        sub: Option<PlanSub>,
    },
    /// Execute a task via a worker session
    Run { id: String },
    /// Send a steering message to a running task
    Steer { id: String, message: Vec<String> },
    /// Abort a running task and reopen it
    Abort { id: String },
    /// List tasks ready to work on
    Ready,
    /// List tasks blocked by unmet dependencies
    Blocked,
    /// Compact the store (latest-per-id, drop tombstones)
    Compact,
    /// Initialize an omenic workspace
    Init,
    /// Manage task dependencies
    Dep {
        #[command(subcommand)]
        sub: DepCmd,
    },
    /// Task board: tasks partitioned by status/readiness
    Board,
    /// Built-in orchestration templates
    Template {
        #[command(subcommand)]
        sub: TemplateCmd,
    },
    /// Spec tables (规范表) for GitHub artifacts
    Spec {
        #[command(subcommand)]
        sub: SpecCmd,
    },
    /// Render task tree as PR Construction plan
    Pr {
        #[command(subcommand)]
        sub: PrCmd,
    },
    /// Read-only parallel subagent exploration (opt-in)
    Subagent {
        #[command(subcommand)]
        sub: SubagentCmd,
    },
    /// Session store queries (daemon-backed)
    Session {
        #[command(subcommand)]
        sub: SessionCmd,
    },
    /// Manage the local daemon.
    Daemon {
        #[command(subcommand)]
        sub: DaemonCmd,
    },
    /// Boot/bundle profiles: prewritten `.oi/config.toml` starting points.
    Profile {
        #[command(subcommand)]
        sub: ProfileCmd,
    },
    /// Terminal chat front-end (T1: linear minimal viable)
    Tui(TuiCmd),
}

/// Arguments of `oi tui` (route §3 T1: `--tui auto|enhanced|linear`).
#[derive(clap::Args)]
struct TuiCmd {
    /// Rendering mode; T1 always renders linear regardless of the gate
    #[arg(long = "tui", value_enum, default_value = "auto")]
    mode: tui::TuiMode,
    /// Resume an existing session by id (missing → exit code 2)
    #[arg(long)]
    session: Option<String>,
    /// Continue the most recently active session
    #[arg(long)]
    resume: bool,
    /// Disable colors (same effect as NO_COLOR)
    #[arg(long)]
    no_color: bool,
    /// Suppress non-essential animation in the enhanced shell
    #[arg(long)]
    reduced_motion: bool,
}

/// Sub-views of `cli profile`.
#[derive(Subcommand)]
enum ProfileCmd {
    /// List embedded profiles (boot / bundle).
    List,
    /// Write a profile's config into `.oi/config.toml` (never overwrites).
    Apply { name: String },
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Start the daemon if it is not already running (spawns the sibling
    /// `daemon` binary; override its location with OMENIC_DAEMON_PATH).
    Start,
    /// Show daemon liveness and process information.
    Status,
    /// Ask the daemon to shut down cleanly.
    Stop,
}

/// Sub-views of `cli subagent`.
#[derive(Subcommand)]
enum SubagentCmd {
    /// Run one or more read-only subagents in parallel
    Run {
        /// One or more prompts; each spawns its own subagent.
        #[arg(long = "prompt", value_name = "PROMPT")]
        prompts: Vec<String>,
        /// Per-subagent loop turn cap (default 10).
        #[arg(long, default_value_t = subagent::config::MAX_TURNS_DEFAULT as u32)]
        max_turns: u32,
    },
}

/// Sub-views of `cli pr`.
#[derive(Subcommand)]
enum PrCmd {
    /// Render a task subtree as Construction plan checkboxes
    Render { id: String },
}

/// Sub-views of `cli plan`.
#[derive(Subcommand)]
enum PlanSub {
    /// Partitioned board view (ready / blocked / in_progress / done)
    Board,
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Create a task
    Add {
        /// Task title(s); multiple positional args create multiple tasks
        title: Vec<String>,
        /// Parent task id
        #[arg(short = 'p', long)]
        parent: Option<String>,
        /// Comma-separated dependency ids
        #[arg(long)]
        deps: Option<String>,
        /// Acceptance criteria text
        #[arg(long)]
        acceptance: Option<String>,
        /// Priority 0-4 (0 highest)
        #[arg(long)]
        priority: Option<u8>,
        /// Task kind (milestone|feature|bug|task|chore|spike|decision)
        #[arg(long)]
        kind: Option<String>,
    },
    /// Mark a task done
    Done { id: String },
    /// Show a task's state
    Status { id: String },
    /// Update task fields
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        deps: Option<String>,
        #[arg(long)]
        acceptance: Option<String>,
        #[arg(long)]
        priority: Option<u8>,
        #[arg(long)]
        kind: Option<String>,
        /// Reset the failed-attempt counter (#47 retry budget)
        #[arg(long)]
        attempts: Option<u32>,
    },
    /// Delete an isolated task
    Delete { id: String },
    /// List tasks (optionally filtered)
    List {
        /// Filter by status (open|in_progress|failed|done, comma-separated)
        #[arg(long)]
        status: Option<String>,
        /// Filter by kind (comma-separated)
        #[arg(long)]
        kind: Option<String>,
        /// Filter by parent id (`none` for root tasks)
        #[arg(long)]
        parent: Option<String>,
    },
    /// Show task details + computed relationships
    Show { id: String },
}

/// Session commands backed by the omenic daemon. Reads and writes go
/// through the running daemon's `SessionDb`; the CLI is a thin client.
#[derive(Subcommand)]
enum SessionCmd {
    /// List sessions matching a query (substring of id/title)
    List {
        /// Substring to match against session id / title
        query: String,
        /// Maximum number of rows to return (default 50)
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Show one session by id
    Get {
        /// Session id
        id: String,
    },
    /// Substring search across message text (optionally scoped to one session)
    Search {
        /// Substring to match against message text
        query: String,
        /// Restrict the search to one session id
        #[arg(long = "scope")]
        scope: Option<String>,
        /// Maximum number of messages to return (default 50)
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Delete a session by id (cascades to its messages)
    Delete {
        /// Session id
        id: String,
    },
    /// Agent-facing dispatcher — same args shape as the `session_query`
    /// ToolDef so the CLI and the worker call the same code path.
    Query {
        /// Kind: list | get | search | delete
        #[arg(long)]
        kind: String,
        /// Substring for list / search kinds
        #[arg(long)]
        query: Option<String>,
        /// Session id for get / delete / search kinds
        #[arg(long = "session-id")]
        session_id: Option<String>,
        /// Maximum rows (default 50)
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Attach to a session: show its summary and recent messages
    Attach {
        /// Session id
        id: String,
        /// Maximum messages to show (default 50)
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Resume a session: send a follow-up prompt to the daemon worker
    Resume {
        /// Session id
        id: String,
        /// Message to send to the worker
        message: String,
    },
}

#[derive(Subcommand)]
enum DepCmd {
    /// Add a dependency edge
    Add { id: String, dep_id: String },
    /// Remove a dependency edge
    Remove { id: String, dep_id: String },
}

#[derive(Subcommand)]
enum TemplateCmd {
    /// List built-in templates
    List,
    /// Apply a template: create topic task + ordered step chain
    Apply {
        /// Template name (dev | plan)
        name: String,
        /// Topic title; becomes the parent task id
        topic: String,
        /// Parent task id for the topic (e.g. a milestone)
        #[arg(short = 'p', long)]
        parent: Option<String>,
    },
}

#[derive(Subcommand)]
enum SpecCmd {
    /// List spec tables
    List,
    /// Generate a blank spec table skeleton
    New {
        /// Spec kind (issue | epic | pr | review)
        kind: String,
        /// Document title (becomes the `#` heading)
        #[arg(long)]
        title: Option<String>,
        /// Write to file instead of stdout
        #[arg(short = 'o', long)]
        output: Option<String>,
    },
    /// Validate a filled spec document against the kind's rules
    Check {
        /// Spec kind to validate against
        kind: String,
        /// Markdown file to check
        file: String,
    },
    /// Print a spec document (agent-facing view)
    View { file: String },
}

/// Entry point: parse argv via clap and dispatch to a subcommand.
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("omenic: {msg}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(cli: Cli) -> Result<u8, String> {
    let json = cli.json;
    match cli.command {
        // No subcommand: point at the TUI first, then the main commands.
        None => {
            println!(
                "omenic: no subcommand. Available: oi tui / oi init / oi task add / oi web / oi daemon ..."
            );
            eprintln!("(run `oi tui` for the terminal UI, or `oi --help` for the full list)");
            Ok(0)
        }
        Some(command) => dispatch_sub(command, json),
    }
}

/// Dispatch a subcommand to the matching implementation function.
fn dispatch_sub(command: Command, json: bool) -> Result<u8, String> {
    match command {
        Command::Task { sub } => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            let store = Store::new(&config.data_dir);
            match sub {
                TaskCmd::Add {
                    title,
                    parent,
                    deps,
                    acceptance,
                    priority,
                    kind,
                } => task_add(
                    &store, &title, parent, deps, acceptance, priority, kind, json,
                ),
                TaskCmd::Done { id } => task_done(&store, &id, json),
                TaskCmd::Status { id } => task_status(&store, &id, json),
                TaskCmd::Update {
                    id,
                    title,
                    description,
                    status,
                    deps,
                    acceptance,
                    priority,
                    kind,
                    attempts,
                } => task_update(
                    &store,
                    &id,
                    title,
                    description,
                    status,
                    deps,
                    acceptance,
                    priority,
                    kind,
                    attempts,
                    json,
                ),
                TaskCmd::Delete { id } => task_delete(&store, &id, json),
                TaskCmd::List {
                    status,
                    kind,
                    parent,
                } => task_list(&store, status, kind, parent, json),
                TaskCmd::Show { id } => task_show(&store, &id, json),
            }
        }
        Command::Show { id } => show_cmd(&id, json),
        Command::Plan { dot, sub } => match sub {
            Some(PlanSub::Board) => board_cmd(json),
            None => plan_cmd(dot, json),
        },
        Command::Run { id } => run_cmd(&id),
        Command::Steer { id, message } => steer_cmd(&id, &message, json),
        Command::Abort { id } => abort_cmd(&id, json),
        Command::Ready => ready_cmd(json),
        Command::Blocked => blocked_cmd(json),
        Command::Compact => compact_cmd(json),
        Command::Init => init_cmd(json),
        Command::Profile { sub } => match sub {
            ProfileCmd::List => profile_list_cmd(json),
            ProfileCmd::Apply { name } => {
                let dir = std::env::current_dir().map_err(|e| format!("cwd error: {e}"))?;
                profile_apply_cmd_at(&dir, &name, json)
            }
        },
        Command::Dep { sub } => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            let store = Store::new(&config.data_dir);
            match sub {
                DepCmd::Add { id, dep_id } => dep_add(&store, &id, &dep_id, json),
                DepCmd::Remove { id, dep_id } => dep_remove(&store, &id, &dep_id, json),
            }
        }
        Command::Board => board_cmd(json),
        Command::Template { sub } => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            let store = Store::new(&config.data_dir);
            match sub {
                TemplateCmd::List => template_list_cmd(json),
                TemplateCmd::Apply {
                    name,
                    topic,
                    parent,
                } => template_apply_cmd(&store, &name, &topic, parent, json),
            }
        }
        Command::Spec { sub } => match sub {
            SpecCmd::List => spec_list_cmd(json),
            SpecCmd::New {
                kind,
                title,
                output,
            } => spec_new_cmd(&kind, title, output, json),
            SpecCmd::Check { kind, file } => spec_check_cmd(&kind, &file, json),
            SpecCmd::View { file } => spec_view_cmd(&file),
        },
        Command::Pr { sub } => match sub {
            PrCmd::Render { id } => pr_render_cmd(&id, json),
        },
        Command::Subagent { sub } => match sub {
            SubagentCmd::Run { prompts, max_turns } => {
                subagent_run_cmd(&prompts, max_turns as usize)
            }
        },
        Command::Session { sub } => session_cmd_dispatch(sub, json),
        Command::Daemon { sub } => daemon_cmd_dispatch(sub, json),
        Command::Tui(args) => {
            // T14 autostart: probe / spawn / wait-ready before the render
            // loop, so a cold start hands the TUI a daemon that already
            // answers pings (the first prompt must not be silently dropped).
            // Path resolution is the same helper `oi daemon start` uses.
            if let Err(reason) = resolve_daemon_bin().and_then(|bin| {
                tui::autostart::ensure_daemon_running(&bin).map_err(|e| e.to_string())
            }) {
                // Autostart could not produce a live daemon: the exact
                // condition `tui::run` reports as DaemonUnreachable, so it
                // keeps that exit code (3) — one stderr line, never a fake
                // start into a TUI that cannot talk to anything.
                eprintln!("omenic tui: {reason}");
                return Ok(3);
            }
            let opts = tui::TuiOptions {
                mode: args.mode,
                session: args.session,
                resume: args.resume,
                no_color: args.no_color,
                reduced_motion: args.reduced_motion,
            };
            match tui::run(opts) {
                Ok(()) => Ok(0),
                Err(err) => {
                    // One line to stderr; the variant maps the exit code
                    // (session 2 / daemon unreachable 3 / other 1, route §5).
                    eprintln!("omenic tui: {err}");
                    Ok(match err {
                        tui::TuiError::SessionNotFound(_) => 2,
                        tui::TuiError::DaemonUnreachable(_) => 3,
                        _ => 1,
                    })
                }
            }
        }
    }
}

#[cfg(any(test))]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use store::{Task, TaskKind, TaskStatus};

    // Serialize tests: store writes go to distinct temp dirs, but stdout is
    // process-global and some tests capture it.
    static LOCK: Mutex<()> = Mutex::new(());

    fn tmp_store(tag: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(&dir)
    }

    #[cfg_attr(test, test)]
    fn add_creates_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("add1");
        task_add(
            &store,
            &["write design doc".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("write design doc").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Open);
        assert_eq!(t.kind, TaskKind::Task);
        assert_eq!(t.parent, None);
        assert_eq!(t.deps, Vec::<String>::new());
    }

    #[cfg_attr(test, test)]
    fn add_multiple_titles() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addmulti");
        let args = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        task_add(&store, &args, None, None, None, None, None, false).unwrap();
        assert_eq!(store.load_all().unwrap().len(), 3);
    }

    #[cfg_attr(test, test)]
    fn add_with_parent() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addparent");
        task_add(
            &store,
            &["child".to_string()],
            Some("schemav1".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("child").unwrap().unwrap();
        assert_eq!(t.parent.as_deref(), Some("schemav1"));
    }

    #[cfg_attr(test, test)]
    fn done_updates_status_and_timestamp() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("done");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let before = store.load_task("t1").unwrap().unwrap().updated_at;
        task_done(&store, "t1", false).unwrap();
        let after = store.load_task("t1").unwrap().unwrap();
        assert_eq!(after.status, TaskStatus::Done);
        // updated_at must not regress; same-second writes may be equal.
        assert!(after.updated_at >= before);
    }

    #[cfg_attr(test, test)]
    fn done_missing_task_exits_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("donemissing");
        let code = task_done(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }

    #[cfg_attr(test, test)]
    fn status_missing_task_exits_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("statusmissing");
        let code = task_status(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }

    #[cfg_attr(test, test)]
    fn add_without_title_errors() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addnoargs");
        let r = task_add(&store, &[], None, None, None, None, None, false);
        assert!(r.is_err());
    }

    #[cfg_attr(test, test)]
    fn unknown_subcommand_errors() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = Cli::try_parse_from(["oi", "task", "bogus"]);
        assert!(r.is_err());
    }

    #[cfg_attr(test, test)]
    fn now_iso_format() {
        let s = store::now_iso();
        assert_eq!(s.len(), 20); // 2026-08-10T12:34:56Z
        assert!(s.ends_with('Z'));
        assert!(s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.as_bytes()[10] == b'T');
    }
    fn mk_task(id: &str, parent: Option<&str>, status: TaskStatus) -> Task {
        let now = "2026-08-10T00:00:00Z".to_string();
        Task {
            id: id.to_string(),
            title: id.to_string(),
            kind: TaskKind::Task,
            status,
            attempts: 0,
            priority: 2,
            parent: parent.map(|p| p.to_string()),
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[cfg_attr(test, test)]
    fn plan_nested_tree_indentation() {
        let tasks = vec![
            mk_task("dev-shell", None, TaskStatus::Open),
            mk_task("scheme-workflow-01", Some("dev-shell"), TaskStatus::Open),
            mk_task("imp-cli", Some("scheme-workflow-01"), TaskStatus::Open),
            mk_task("imp-rpc", Some("scheme-workflow-01"), TaskStatus::Open),
            mk_task("scheme-workflow-02", Some("dev-shell"), TaskStatus::Done),
        ];
        let expected = "\
○ dev-shell ● P2 dev-shell
├─ ○ scheme-workflow-01 ● P2 scheme-workflow-01
│  ├─ ○ imp-cli ● P2 imp-cli
│  └─ ○ imp-rpc ● P2 imp-rpc
└─ ✓ scheme-workflow-02 ● P2 scheme-workflow-02
Status: ○ open  ◐ in_progress  ✗ failed  ● blocked  ✓ done
";
        assert_eq!(render_plan(&tasks), expected);
    }

    #[cfg_attr(test, test)]
    fn plan_empty_store() {
        assert_eq!(render_plan(&[]), "(no tasks)\n");
    }

    #[cfg_attr(test, test)]
    fn plan_dangling_parent_is_root() {
        // parent id that doesn't exist in the store → treated as a root
        let tasks = vec![
            mk_task("a", Some("ghost"), TaskStatus::Open),
            mk_task("b", None, TaskStatus::Open),
        ];
        let out = render_plan(&tasks);
        assert!(out.starts_with("○ a ● P2 a\n"));
        assert!(out.contains("○ b ● P2 b"));
    }

    #[cfg_attr(test, test)]
    fn plan_cycle_does_not_hang() {
        let tasks = vec![
            mk_task("a", Some("b"), TaskStatus::Open),
            mk_task("b", Some("a"), TaskStatus::Open),
        ];
        let out = render_plan(&tasks);
        // Both appear exactly once; renderer terminates.
        assert_eq!(out.matches("○ a ● P2 a").count(), 1);
        assert_eq!(out.matches("○ b ● P2 b").count(), 1);
    }

    #[cfg_attr(test, test)]
    fn plan_status_rendering() {
        let tasks = vec![
            mk_task("t-open", None, TaskStatus::Open),
            mk_task("t-ip", None, TaskStatus::InProgress),
            mk_task("t-done", None, TaskStatus::Done),
        ];
        let out = render_plan(&tasks);
        assert!(out.contains("○ t-open ● P2 t-open"));
        assert!(out.contains("◐ t-ip ● P2 t-ip"));
        assert!(out.contains("✓ t-done ● P2 t-done"));
    }

    #[cfg_attr(test, test)]
    fn run_command_parse_errors_on_missing_id() {
        let r = Cli::try_parse_from(["oi", "run"]).is_err();
        assert!(r); // needs <task-id>
    }

    #[cfg_attr(test, test)]
    fn steer_command_parse_and_note() {
        // non-empty message required after id; bare steer errors
        // steer with no message: clap allows it (empty vec), but steer_cmd should error
        let r = Cli::try_parse_from(["oi", "steer", "t-1"]);
        assert!(r.is_ok()); // parsing succeeds; steer_cmd handles empty message
        // with msg should hit the handle (but not fail dispatch parse)
        let r = Cli::try_parse_from(["oi", "steer", "t-1", "keep chipping"]);
        assert!(r.is_ok());
    }

    #[cfg_attr(test, test)]
    fn abort_command_parse_needs_id() {
        let r = Cli::try_parse_from(["oi", "abort"]).is_err();
        assert!(r);
    }

    // --- #50: unknown flags rejected, not swallowed as title ---
    #[cfg_attr(test, test)]
    fn add_unknown_flag_rejected() {
        let r = Cli::try_parse_from(["oi", "task", "add", "-t", "foo"]);
        assert!(r.is_err(), "unknown flag -t must be rejected");
    }

    #[cfg_attr(test, test)]
    fn add_unknown_long_flag_rejected() {
        let r = Cli::try_parse_from(["oi", "task", "add", "--bogus", "x"]);
        assert!(r.is_err());
    }

    // --- #42: --deps and --acceptance ---

    #[cfg_attr(test, test)]
    fn add_with_deps() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("deps42");
        // Create prerequisite tasks first
        task_add(
            &store,
            &["prereq-a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["prereq-b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // Create dependent task
        task_add(
            &store,
            &["dependent".to_string()],
            None,
            Some("prereq-a,prereq-b".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("dependent").unwrap().unwrap();
        assert_eq!(t.deps, vec!["prereq-a", "prereq-b"]);
    }

    #[cfg_attr(test, test)]
    fn add_with_acceptance() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("acc42");
        task_add(
            &store,
            &["task-x".to_string()],
            None,
            None,
            Some("all tests pass".to_string()),
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("task-x").unwrap().unwrap();
        assert_eq!(t.acceptance, "all tests pass");
    }

    #[cfg_attr(test, test)]
    fn add_deps_nonexistent_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("deps404");
        let r = task_add(
            &store,
            &["t1".to_string()],
            None,
            Some("ghost".to_string()),
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("does not exist"));
    }

    #[cfg_attr(test, test)]
    fn add_self_dep_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("selfdep");
        let r = task_add(
            &store,
            &["s1".to_string()],
            None,
            Some("s1".to_string()),
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot depend on itself"));
    }

    #[cfg_attr(test, test)]
    fn add_deps_cycle_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("cycle42");
        // a exists, b depends on a
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            Some("a".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // re-create a with deps=[b] → a→b→a cycle
        let r = task_add(
            &store,
            &["a".to_string()],
            None,
            Some("b".to_string()),
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cycle"));
    }

    #[cfg_attr(test, test)]
    fn add_deps_and_acceptance_combined() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("combo42");
        task_add(
            &store,
            &["p1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("parent-epic".to_string()),
            Some("p1".to_string()),
            Some("done when p1 is verified".to_string()),
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("child").unwrap().unwrap();
        assert_eq!(t.deps, vec!["p1"]);
        assert_eq!(t.acceptance, "done when p1 is verified");
        assert_eq!(t.parent.as_deref(), Some("parent-epic"));
    }

    // --- #179: dep add / dep remove ---

    fn dep_store(tag: &str) -> (Store, [String; 3]) {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store(tag);
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["t2".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["t3".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let ids = ["t1".to_string(), "t2".to_string(), "t3".to_string()];
        (store, ids)
    }

    #[cfg_attr(test, test)]
    fn dep_add_creates_edge() {
        let (store, ids) = dep_store("depadd1");
        let r = dep_add(&store, "t1", "t2", false);
        assert!(r.is_ok());
        let t = store.load_task(&ids[0]).unwrap().unwrap();
        assert_eq!(t.deps, vec!["t2"]);
    }

    #[cfg_attr(test, test)]
    fn dep_add_includes_dep_in_status() {
        let (store, _ids) = dep_store("depadd_status");
        dep_add(&store, "t1", "t2", false).unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert!(t.deps.contains(&"t2".to_string()));
    }

    #[cfg_attr(test, test)]
    fn dep_add_self_dep_rejected() {
        let (store, _ids) = dep_store("depself");
        let r = dep_add(&store, "t1", "t1", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot depend on itself"));
    }

    #[cfg_attr(test, test)]
    fn dep_add_duplicate_rejected() {
        let (store, _ids) = dep_store("depdup");
        dep_add(&store, "t1", "t2", false).unwrap();
        let r = dep_add(&store, "t1", "t2", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("already exists"));
    }

    #[cfg_attr(test, test)]
    fn dep_add_nonexistent_dep_rejected() {
        let (store, _ids) = dep_store("dep404");
        let r = dep_add(&store, "t1", "ghost", false);
        // dep_id doesn't exist → eprintln + Ok(1)
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), 1);
    }

    #[cfg_attr(test, test)]
    fn dep_add_nonexistent_task_rejected() {
        let (store, _ids) = dep_store("dep404task");
        let r = dep_add(&store, "ghost", "t2", false);
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), 1);
    }

    #[cfg_attr(test, test)]
    fn dep_add_cycle_rejected() {
        let (store, _ids) = dep_store("depcycle");
        // t1 → t2
        dep_add(&store, "t1", "t2", false).unwrap();
        // t2 → t3
        dep_add(&store, "t2", "t3", false).unwrap();
        // t3 → t1 would create cycle: t1→t2→t3→t1
        let r = dep_add(&store, "t3", "t1", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cycle"));
    }

    #[cfg_attr(test, test)]
    fn dep_add_keeps_deps_sorted() {
        let (store, _ids) = dep_store("depsort");
        dep_add(&store, "t1", "t3", false).unwrap();
        dep_add(&store, "t1", "t2", false).unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.deps, vec!["t2", "t3"]);
    }

    #[cfg_attr(test, test)]
    fn dep_add_updates_timestamp() {
        let (store, _ids) = dep_store("dep_ts");
        let before = store.load_task("t1").unwrap().unwrap().updated_at;
        dep_add(&store, "t1", "t2", false).unwrap();
        let after = store.load_task("t1").unwrap().unwrap();
        assert!(after.updated_at >= before);
    }

    #[cfg_attr(test, test)]
    fn dep_remove_deletes_edge() {
        let (store, _ids) = dep_store("depremove1");
        dep_add(&store, "t1", "t2", false).unwrap();
        let r = dep_remove(&store, "t1", "t2", false);
        assert!(r.is_ok());
        let t = store.load_task("t1").unwrap().unwrap();
        assert!(!t.deps.contains(&"t2".to_string()));
        assert!(t.deps.is_empty());
    }

    #[cfg_attr(test, test)]
    fn dep_remove_not_a_dep_rejected() {
        let (store, _ids) = dep_store("depremove_missing");
        dep_add(&store, "t1", "t2", false).unwrap();
        // t3 was never a dep of t1
        let r = dep_remove(&store, "t1", "t3", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not found"));
    }

    #[cfg_attr(test, test)]
    fn dep_remove_missing_task_rejected() {
        let (store, _ids) = dep_store("depremove_404");
        let r = dep_remove(&store, "ghost", "t2", false);
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), 1);
    }

    #[cfg_attr(test, test)]
    fn dep_remove_preserves_other_deps() {
        let (store, _ids) = dep_store("depremove_keep");
        dep_add(&store, "t1", "t2", false).unwrap();
        dep_add(&store, "t1", "t3", false).unwrap();
        dep_remove(&store, "t1", "t2", false).unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.deps, vec!["t3"]);
    }

    #[cfg_attr(test, test)]
    fn dep_cmd_unknown_sub_rejected() {
        let r = Cli::try_parse_from(["oi", "dep", "bogus"]);
        assert!(r.is_err());
    }

    #[cfg_attr(test, test)]
    fn dep_cmd_no_sub_rejected() {
        let r = Cli::try_parse_from(["oi", "dep"]);
        assert!(r.is_err());
    }

    #[cfg_attr(test, test)]
    fn dep_add_wrong_arg_count_rejected() {
        let r = Cli::try_parse_from(["oi", "dep", "add", "t1"]);
        assert!(r.is_err());
    }

    #[cfg_attr(test, test)]
    fn dep_remove_wrong_arg_count_rejected() {
        let r = Cli::try_parse_from(["oi", "dep", "remove", "t1"]);
        assert!(r.is_err());
    }

    // --- #178: task update / delete / list / show ---

    #[cfg_attr(test, test)]
    fn update_status() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_status");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "t1",
            None,
            None,
            Some("in_progress".to_string()),
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
    }

    #[cfg_attr(test, test)]
    fn update_title() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_title");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "t1",
            Some("new title".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.title, "new title");
    }

    #[cfg_attr(test, test)]
    fn update_acceptance() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_acc");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "t1",
            None,
            None,
            None,
            None,
            Some("all pass".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.acceptance, "all pass");
    }

    #[cfg_attr(test, test)]
    fn update_deps() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_deps");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "b",
            None,
            None,
            None,
            Some("a".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("b").unwrap().unwrap();
        assert_eq!(t.deps, vec!["a"]);
    }

    #[cfg_attr(test, test)]
    fn update_missing_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_404");
        let code = task_update(
            &store, "nope", None, None, None, None, None, None, None, None, false,
        )
        .unwrap();
        assert_eq!(code, 1);
    }

    #[cfg_attr(test, test)]
    fn update_invalid_status() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_badstatus");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_update(
            &store,
            "t1",
            None,
            None,
            Some("bogus".to_string()),
            None,
            None,
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("invalid status"));
    }

    #[cfg_attr(test, test)]
    fn update_missing_task_returns_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_noargs");
        let r = task_update(
            &store,
            "nonexistent",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        );
        assert_eq!(r.unwrap(), 1);
    }

    #[cfg_attr(test, test)]
    fn update_self_dep_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_selfdep");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_update(
            &store,
            "t1",
            None,
            None,
            None,
            Some("t1".to_string()),
            None,
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot depend on itself"));
    }

    #[cfg_attr(test, test)]
    fn update_deps_nonexistent_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_dep404");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_update(
            &store,
            "t1",
            None,
            None,
            None,
            Some("ghost".to_string()),
            None,
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("does not exist"));
    }

    #[cfg_attr(test, test)]
    fn delete_isolated_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_isolated");
        task_add(
            &store,
            &["orphan".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_delete(&store, "orphan", false).unwrap();
        assert!(store.load_task("orphan").unwrap().is_none());
    }

    #[cfg_attr(test, test)]
    fn delete_with_child_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_child");
        task_add(
            &store,
            &["parent".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("parent".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_delete(&store, "parent", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot delete"));
    }

    #[cfg_attr(test, test)]
    fn delete_with_dependent_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_dep");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        dep_add(&store, "b", "a", false).unwrap();
        let r = task_delete(&store, "a", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot delete"));
    }

    #[cfg_attr(test, test)]
    fn delete_missing_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_404");
        let code = task_delete(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }

    #[cfg_attr(test, test)]
    fn list_no_filter() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_all");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_list(&store, None, None, None, false);
        assert!(r.is_ok());
    }

    #[cfg_attr(test, test)]
    fn list_by_status() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_status");
        task_add(
            &store,
            &["open1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["open2".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["done1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_done(&store, "done1", false).unwrap();
        let r = task_list(&store, Some("done".to_string()), None, None, false);
        assert!(r.is_ok());
        // Verify only done tasks in store match
        let done: Vec<Task> = store
            .load_all()
            .unwrap()
            .into_iter()
            .filter(|t| t.status == TaskStatus::Done)
            .collect();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id, "done1");
    }

    #[cfg_attr(test, test)]
    fn list_by_parent() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_parent");
        task_add(
            &store,
            &["root".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("root".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_list(&store, None, None, Some("none".to_string()), false);
        assert!(r.is_ok());
        let roots: Vec<Task> = store
            .load_all()
            .unwrap()
            .into_iter()
            .filter(|t| t.parent.is_none())
            .collect();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, "root");
    }

    #[cfg_attr(test, test)]
    fn list_empty_store() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_empty");
        let r = task_list(&store, None, None, None, false);
        assert!(r.is_ok());
    }

    #[cfg_attr(test, test)]
    fn show_task_detail() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_detail");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_show(&store, "t1", false);
        assert!(r.is_ok());
    }

    #[cfg_attr(test, test)]
    fn show_task_with_children_and_deps() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_rels");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        dep_add(&store, "b", "a", false).unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("a".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_show(&store, "a", false);
        assert!(r.is_ok());
        let all = store.load_all().unwrap();
        let children = store::graph::children_of(&all, "a");
        let dependents = store::graph::dependents(&all, "a");
        assert_eq!(children, vec!["child"]);
        assert_eq!(dependents, vec!["b"]);
    }

    #[cfg_attr(test, test)]
    fn show_missing_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_404");
        let code = task_show(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }
    #[cfg_attr(test, test)]
    fn show_cmd_top_level_no_arg_errors() {
        // No arg → usage error before Config::load is reached.
        let r = Cli::try_parse_from(["oi", "show"]);
        assert!(r.is_err());
    }

    #[cfg_attr(test, test)]
    fn show_cmd_top_level_alias() {
        // show_cmd calls Config::load which needs env; test core logic via task_show instead.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_alias");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_show(&store, "t1", false);
        assert!(r.is_ok());
    }
    fn store_file(tag: &str) -> String {
        let path = std::env::temp_dir()
            .join(format!("omenic-cli-test-{tag}-{}", std::process::id()))
            .join("tasks.jsonl");
        std::fs::read_to_string(&path).unwrap_or_default()
    }

    fn mk_task_titled(id: &str, title: &str) -> Task {
        let now = store::now_iso();
        Task {
            id: id.to_string(),
            title: title.to_string(),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            attempts: 0,
            priority: 2,
            parent: None,
            deps: Vec::new(),
            description: String::new(),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[cfg_attr(test, test)]
    fn compact_reduces_duplicate_ids() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_dup");
        assert!(store.append(&mk_task_titled("t1", "v1")).is_ok());
        assert!(store.append(&mk_task_titled("t1", "v2")).is_ok());
        assert!(store.append(&mk_task_titled("t1", "v3")).is_ok());
        assert!(store.append(&mk_task_titled("t1", "v4")).is_ok());
        // load_all already dedupes (latest-wins).
        let before = store.load_all().unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].title, "v4");
        // File still holds all 4 lines physically.
        assert_eq!(store_file("compact_dup").lines().count(), 4);
        assert!(store.compact().is_ok());
        // After compact: 1 logical task, latest title, 1 physical line.
        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].title, "v4");
        assert_eq!(store_file("compact_dup").lines().count(), 1);
    }

    #[cfg_attr(test, test)]
    fn compact_drops_tombstones() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_tomb");
        task_add(
            &store,
            &["alpha".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["beta".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_delete(&store, "beta", false).unwrap();
        // File: 2 task lines + 1 tombstone line.
        assert_eq!(store_file("compact_tomb").lines().count(), 3);
        assert!(store_file("compact_tomb").contains("tombstone"));
        assert_eq!(store.load_all().unwrap().len(), 1);
        assert!(store.compact().is_ok());
        // After compact: only alpha remains; no tombstone line in file.
        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "alpha");
        let file = store_file("compact_tomb");
        assert!(!file.contains("tombstone"));
        assert_eq!(file.lines().count(), 1);
    }

    #[cfg_attr(test, test)]
    fn compact_empty_store() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_empty");
        assert!(store.load_all().unwrap().is_empty());
        assert!(store.compact().is_ok());
        assert!(store.load_all().unwrap().is_empty());
    }

    #[cfg_attr(test, test)]
    fn compact_preserves_data() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_data");
        task_add(
            &store,
            &["base".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // second task depends on base, with acceptance text.
        task_add(
            &store,
            &["child".to_string()],
            None,
            Some("base".to_string()),
            Some("all tests green".to_string()),
            None,
            None,
            false,
        )
        .unwrap();
        let before = store.load_all().unwrap();
        assert_eq!(before.len(), 2);
        assert!(store.compact().is_ok());
        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 2);
        let child = after.iter().find(|t| t.id == "child").unwrap();
        assert_eq!(child.deps, vec!["base".to_string()]);
        assert_eq!(child.acceptance, "all tests green");
    }

    // --- #183: cli init ---

    #[cfg_attr(test, test)]
    fn init_creates_data_dir_and_config() {
        let dir = std::env::temp_dir().join(format!(
            "omenic-cli-test-init-create-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        init_cmd_at(&dir, false).unwrap();
        assert!(dir.join(".oi").is_dir());
        assert!(dir.join(".oi/config.toml").is_file());
        assert!(dir.join(".oi/specs").is_dir());
        let content = std::fs::read_to_string(dir.join(".oi/config.toml")).unwrap();
        assert!(content.contains("omp_path = \"omp\""));
        assert!(content.contains("data_dir = \"./.oi\""));
        assert!(content.contains("model = \"default\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg_attr(test, test)]
    fn init_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-init-idem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        init_cmd_at(&dir, false).unwrap();
        let toml = std::fs::read_to_string(dir.join(".oi/config.toml")).unwrap();
        init_cmd_at(&dir, false).unwrap();
        // Not overwritten.
        assert_eq!(
            toml,
            std::fs::read_to_string(dir.join(".oi/config.toml")).unwrap()
        );
        assert!(dir.join(".oi").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg_attr(test, test)]
    fn init_partial_existing() {
        let dir = std::env::temp_dir().join(format!(
            "omenic-cli-test-init-partial-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".oi")).unwrap();
        init_cmd_at(&dir, false).unwrap();
        // config.toml + specs created, .oi already there.
        assert!(dir.join(".oi/config.toml").is_file());
        assert!(dir.join(".oi/specs").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- #181: cli ready / cli blocked ---

    /// Helper: build a store with tasks from a spec of (id, priority, deps).
    fn make_ready_store(tag: &str, specs: &[(&str, u8, &[&str])]) -> Store {
        let store = tmp_store(tag);
        for &(id, prio, deps) in specs {
            let deps_opt = if deps.is_empty() {
                None
            } else {
                Some(deps.join(","))
            };
            task_add(
                &store,
                &[id.to_string()],
                None,
                deps_opt,
                None,
                Some(prio),
                None,
                false,
            )
            .unwrap();
        }
        store
    }

    #[cfg_attr(test, test)]
    fn ready_shows_only_unblocked_open() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = make_ready_store(
            "ready1",
            &[("A", 2, &[]), ("B", 2, &["A"]), ("C", 2, &["A"])],
        );
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let ready: Vec<&str> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && store::graph::is_ready(&map, &t.id))
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ready, vec!["A"]);
    }

    #[cfg_attr(test, test)]
    fn ready_after_done_shows_unblocked() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = make_ready_store(
            "ready2",
            &[("A", 2, &[]), ("B", 2, &["A"]), ("C", 2, &["A"])],
        );
        task_done(&store, "A", false).unwrap();
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let mut ready: Vec<&str> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && store::graph::is_ready(&map, &t.id))
            .map(|t| t.id.as_str())
            .collect();
        ready.sort();
        assert_eq!(ready, vec!["B", "C"]);
    }

    #[cfg_attr(test, test)]
    fn ready_empty() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("ready_empty");
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let ready: Vec<&Task> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && store::graph::is_ready(&map, &t.id))
            .collect();
        assert!(ready.is_empty());
        // The command prints "(no ready tasks)" when empty.
    }

    #[cfg_attr(test, test)]
    fn blocked_shows_blocked_with_reasons() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = make_ready_store("blocked1", &[("A", 2, &[]), ("B", 2, &["A"])]);
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        // B is blocked by A (A is open, not done).
        let blocked: Vec<(String, Vec<String>)> = all
            .iter()
            .filter_map(|t| {
                if t.status == TaskStatus::Done {
                    return None;
                }
                let unmet: Vec<String> = t
                    .deps
                    .iter()
                    .filter(|dep| map.get(*dep).is_none_or(|d| d.status != TaskStatus::Done))
                    .cloned()
                    .collect();
                if unmet.is_empty() {
                    None
                } else {
                    Some((t.id.clone(), unmet))
                }
            })
            .collect();
        // B should be blocked by A. A has no deps → not blocked.
        let b = blocked.iter().find(|(id, _)| id == "B").unwrap();
        assert_eq!(b.1, vec!["A".to_string()]);
        assert!(!blocked.iter().any(|(id, _)| id == "A"));
    }

    #[cfg_attr(test, test)]
    fn blocked_empty() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A has no deps → not blocked. All ready → "(no blocked tasks)".
        let store = make_ready_store("blocked_empty", &[("A", 2, &[])]);
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let blocked: Vec<&Task> = all
            .iter()
            .filter_map(|t| {
                if t.status == TaskStatus::Done {
                    return None;
                }
                let unmet = t
                    .deps
                    .iter()
                    .filter(|dep| map.get(*dep).is_none_or(|d| d.status != TaskStatus::Done));
                if unmet.count() == 0 { None } else { Some(t) }
            })
            .collect();
        assert!(blocked.is_empty());
    }

    #[cfg_attr(test, test)]
    fn ready_sorted_by_priority() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Three root tasks with different priorities.
        let store = make_ready_store(
            "ready_sort",
            &[("low", 2, &[]), ("high", 0, &[]), ("mid", 1, &[])],
        );
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let mut ready: Vec<&Task> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && store::graph::is_ready(&map, &t.id))
            .collect();
        ready.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));
        let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
        // P0 (high) before P1 (mid) before P2 (low).
        assert_eq!(ids, vec!["high", "mid", "low"]);
    }

    // --- #184: plan --dot + done suggest-next ---

    #[cfg_attr(test, test)]
    fn plan_dot_outputs_valid_dot() {
        let mut a = mk_task("A", None, TaskStatus::Open);
        a.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Open);
        b.deps = vec!["A".to_string()];
        let tasks = vec![a, b];
        let dot = render_dot(&tasks);
        assert!(dot.starts_with("digraph omenic {\n"));
        assert!(dot.contains("\"A\""));
        assert!(dot.contains("\"B\""));
        // B depends on A → edge A -> B
        assert!(dot.contains("\"A\" -> \"B\";"));
        assert!(dot.ends_with("}\n"));
    }

    #[cfg_attr(test, test)]
    fn plan_dot_empty() {
        assert_eq!(render_dot(&[]), "digraph omenic {\n}\n");
    }

    #[cfg_attr(test, test)]
    fn plan_dot_colors_by_status() {
        let tasks = vec![
            mk_task("t-open", None, TaskStatus::Open),
            mk_task("t-ip", None, TaskStatus::InProgress),
            mk_task("t-done", None, TaskStatus::Done),
        ];
        let dot = render_dot(&tasks);
        assert!(dot.contains("fillcolor=\"#e8f4fd\""), "open color");
        assert!(dot.contains("fillcolor=\"#fff3cd\""), "in_progress color");
        assert!(dot.contains("fillcolor=\"#d4edda\""), "done color");
    }

    #[cfg_attr(test, test)]
    fn plan_dot_parent_child_dotted_edge() {
        let tasks = vec![
            mk_task("parent", None, TaskStatus::Open),
            mk_task("child", Some("parent"), TaskStatus::Open),
        ];
        let dot = render_dot(&tasks);
        assert!(
            dot.contains("\"parent\" -> \"child\" [style=dotted, arrowhead=none]"),
            "parent edge styled dotted"
        );
    }

    // suggest_next is tested via the pure helper to avoid stdout capture.

    #[cfg_attr(test, test)]
    fn done_suggest_next_shows_unblocked() {
        let mut a = mk_task("A", None, TaskStatus::Done);
        a.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Open);
        b.deps = vec!["A".to_string()];
        let mut c = mk_task("C", None, TaskStatus::Open);
        c.deps = vec!["A".to_string()];
        let ready = suggest_next(&[a, b, c], "A");
        assert!(ready.contains(&"B".to_string()));
        assert!(ready.contains(&"C".to_string()));
        assert_eq!(ready.len(), 2);
    }

    #[cfg_attr(test, test)]
    fn done_suggest_next_none() {
        let a = mk_task("A", None, TaskStatus::Done);
        let b = mk_task("B", None, TaskStatus::Open); // B does NOT depend on A
        let ready = suggest_next(&[a, b], "A");
        assert!(ready.is_empty());
    }

    #[cfg_attr(test, test)]
    fn done_suggest_next_partial_deps_not_ready() {
        // B deps=[A, X]; A done but X not → B still blocked.
        let mut a = mk_task("A", None, TaskStatus::Done);
        a.deps = vec![];
        let mut x = mk_task("X", None, TaskStatus::Open);
        x.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Open);
        b.deps = vec!["A".to_string(), "X".to_string()];
        let ready = suggest_next(&[a, x, b], "A");
        assert!(ready.is_empty());
    }

    #[cfg_attr(test, test)]
    fn done_suggest_next_skips_done_tasks() {
        // B already Done and deps=[A] → not suggested (it's not Open).
        let mut a = mk_task("A", None, TaskStatus::Done);
        a.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Done);
        b.deps = vec!["A".to_string()];
        let ready = suggest_next(&[a, b], "A");
        assert!(ready.is_empty());
    }

    #[cfg_attr(test, test)]
    fn done_suggest_next_via_store_integration() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("suggest184");
        task_add(
            &store,
            &["A".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["B".to_string()],
            None,
            Some("A".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // Mark A done → B should be newly ready.
        task_done(&store, "A", false).unwrap();
        let all = store.load_all().unwrap();
        let ready = suggest_next(&all, "A");
        assert!(ready.contains(&"B".to_string()));
    }

    #[cfg_attr(test, test)]
    fn parse_status_accepts_failed() {
        assert_eq!(parse_status("failed"), Ok(TaskStatus::Failed));
        assert!(parse_status("bogus").is_err());
    }

    #[cfg_attr(test, test)]
    fn update_resets_attempts() {
        // #47: the documented escape hatch for an exhausted retry budget.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("attempts_reset");
        store.append(&mk_task_titled("t1", "t1")).unwrap();
        task_update(
            &store,
            "t1",
            None,
            None,
            Some("failed".into()),
            None,
            None,
            None,
            None,
            Some(3),
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Failed);
        assert_eq!(t.attempts, 3);
        task_update(
            &store,
            "t1",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(0),
            false,
        )
        .unwrap();
        assert_eq!(store.load_task("t1").unwrap().unwrap().attempts, 0);
    }

    #[cfg_attr(test, test)]
    fn persist_failed_attempt_bumps_counter_and_evidence() {
        // #47: a failed run flips to Failed, counts the attempt and leaves
        // the reason + timestamp in attempts.jsonl.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-persist-f-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(&dir);
        let pre = Task {
            attempts: 2,
            ..mk_task_titled("t1", "t1")
        };
        store.append(&pre).unwrap();

        let outcome = daemon::runner::RunOutcome {
            status: daemon::runner::RunStatus::Failed,
            summary: "read_event error: 中文 \"boom\" 💥".into(),
            events_seen: 7,
        };
        let updated = persist_run_outcome(&store, &dir, &pre, &outcome).unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
        assert_eq!(updated.attempts, 3);

        let stored = store.load_task("t1").unwrap().unwrap();
        assert_eq!(stored.attempts, 3);

        let rec: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("tasks/t1/attempts.jsonl")).unwrap(),
        )
        .unwrap();
        assert_eq!(rec["attempt"], 3);
        assert_eq!(rec["outcome"], "failed");
        assert_eq!(rec["reason"], "read_event error: 中文 \"boom\" 💥");
        assert_eq!(rec["events_seen"], 7);
        assert_eq!(rec["ts"].as_str().unwrap().len(), 20);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg_attr(test, test)]
    fn persist_done_attempt_keeps_failure_count() {
        // #47: success does not count as a failure; the record still lands
        // in evidence.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-persist-d-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(&dir);
        let pre = mk_task_titled("t2", "t2");
        store.append(&pre).unwrap();

        let outcome = daemon::runner::RunOutcome {
            status: daemon::runner::RunStatus::Done,
            summary: "all good".into(),
            events_seen: 3,
        };
        let updated = persist_run_outcome(&store, &dir, &pre, &outcome).unwrap();
        assert_eq!(updated.status, TaskStatus::Done);
        assert_eq!(updated.attempts, 0);
        assert_eq!(
            store.load_task("t2").unwrap().unwrap().status,
            TaskStatus::Done
        );

        let rec: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("tasks/t2/attempts.jsonl")).unwrap(),
        )
        .unwrap();
        assert_eq!(rec["attempt"], 1);
        assert_eq!(rec["outcome"], "done");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
