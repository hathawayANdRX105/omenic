//! CLI command layer.
//!
//! Subcommands: task add/done/status, plan, run, steer, abort.
//! Implemented in M1.8 (task) / M1.9 (plan) / M2 (run/steer/abort).
//!
//! Parsing is intentionally hand-rolled over `std::env::args`: the surface is
//! small and stable, and skipping a parser dependency keeps the binary slim.

use std::process::ExitCode;

use crate::config::Config;
use crate::store::Store;
use crate::task::{Task, TaskKind, TaskStatus};

/// Entry point: parse argv and dispatch to a subcommand.
///
/// Returns the process exit code so tests can exercise routing without
/// calling `std::process::exit`.
pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("omenic: {msg}");
            ExitCode::from(2)
        }
    }
}

/// Dispatch `args` (everything after argv[0]) to the matching subcommand.
///
/// Returns `Ok(exit_code)` on handled commands and `Err(msg)` for usage
/// errors (printed on stderr, exit 2).
fn dispatch(args: &[String]) -> Result<u8, String> {
    let Some(cmd) = args.first() else {
        return Err(usage());
    };
    match cmd.as_str() {
        "task" => task_cmd(&args[1..]),
        // plan lands in M1.9; run/steer/abort land in M2.
        "plan" | "run" | "steer" | "abort" => {
            Err(format!("command `{cmd}` is not implemented yet"))
        }
        "help" | "-h" | "--help" => {
            print!("{}", usage());
            Ok(0)
        }
        other => Err(format!("unknown command `{other}`\n\n{}", usage())),
    }
}

fn usage() -> String {
    "usage: omenic <command> [args]\n\n\
     commands:\n\
       task add <title> [-p <parent-id>]   create a task (repeat title for multiple)\n\
       task done <id>                      mark a task done\n\
       task status <id>                    show a task's state\n\
       help                                show this help\n"
        .to_string()
}

/// `task` subcommand group.
fn task_cmd(args: &[String]) -> Result<u8, String> {
    let Some(sub) = args.first() else {
        return Err("usage: omenic task <add|done|status> ...".to_string());
    };
    // Validate before touching config/store so unknown commands fail fast
    // and stay testable without environment setup.
    match sub.as_str() {
        "add" | "done" | "status" => {}
        other => {
            return Err(format!(
                "unknown task command `{other}`\n\nusage: omenic task <add|done|status> ..."
            ));
        }
    }
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    match sub.as_str() {
        "add" => task_add(&store, &args[1..]),
        "done" => task_done(&store, &args[1..]),
        "status" => task_status(&store, &args[1..]),
        _ => unreachable!("validated above"),
    }
}

/// Parse `-p <id>` out of the tail of `args`; returns (titles, parent).
fn parse_add_args(args: &[String]) -> Result<(Vec<String>, Option<String>), String> {
    let mut titles = Vec::new();
    let mut parent = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--parent" => {
                let Some(id) = args.get(i + 1) else {
                    return Err("`-p` requires a parent id".to_string());
                };
                parent = Some(id.clone());
                i += 2;
            }
            t => {
                titles.push(t.to_string());
                i += 1;
            }
        }
    }
    if titles.is_empty() {
        return Err("usage: omenic task add <title> [-p <parent-id>]".to_string());
    }
    Ok((titles, parent))
}

fn now_iso() -> String {
    // ISO-8601-ish UTC timestamp; seconds precision is plenty for MVP.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since epoch → civil date via a small conversion.
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn task_add(store: &Store, args: &[String]) -> Result<u8, String> {
    let (titles, parent) = parse_add_args(args)?;
    for title in &titles {
        let now = now_iso();
        let task = Task {
            id: title.clone(),
            title: title.clone(),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            parent: parent.clone(),
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        store
            .append(&task)
            .map_err(|e| format!("store error: {e}"))?;
        println!("created {id}", id = task.id);
    }
    Ok(0)
}

fn task_done(store: &Store, args: &[String]) -> Result<u8, String> {
    let Some(id) = args.first() else {
        return Err("usage: omenic task done <id>".to_string());
    };
    let Some(mut task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    if task.status == TaskStatus::Done {
        eprintln!("task already done: {id}");
        return Ok(1);
    }
    task.status = TaskStatus::Done;
    task.updated_at = now_iso();
    store
        .append(&task)
        .map_err(|e| format!("store error: {e}"))?;
    println!("done {id}");
    Ok(0)
}

fn task_status(store: &Store, args: &[String]) -> Result<u8, String> {
    let Some(id) = args.first() else {
        return Err("usage: omenic task status <id>".to_string());
    };
    let Some(task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    println!("id:         {}", task.id);
    println!("title:      {}", task.title);
    println!("status:     {:?}", task.status);
    match &task.parent {
        Some(p) => println!("parent:     {p}"),
        None => println!("parent:     -"),
    }
    if task.deps.is_empty() {
        println!("deps:       -");
    } else {
        println!("deps:       {}", task.deps.join(", "));
    }
    println!("created_at: {}", task.created_at);
    println!("updated_at: {}", task.updated_at);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests: store writes go to distinct temp dirs, but stdout is
    // process-global and some tests capture it.
    static LOCK: Mutex<()> = Mutex::new(());

    fn tmp_store(tag: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(&dir)
    }

    #[test]
    fn add_creates_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("add1");
        task_add(&store, &["write design doc".to_string()]).unwrap();
        let t = store.load_task("write design doc").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Open);
        assert_eq!(t.kind, TaskKind::Task);
        assert_eq!(t.parent, None);
        assert_eq!(t.deps, Vec::<String>::new());
    }

    #[test]
    fn add_multiple_titles() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addmulti");
        let args = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        task_add(&store, &args).unwrap();
        assert_eq!(store.load_all().unwrap().len(), 3);
    }

    #[test]
    fn add_with_parent() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addparent");
        let args = vec![
            "child".to_string(),
            "-p".to_string(),
            "schemav1".to_string(),
        ];
        task_add(&store, &args).unwrap();
        let t = store.load_task("child").unwrap().unwrap();
        assert_eq!(t.parent.as_deref(), Some("schemav1"));
    }

    #[test]
    fn done_updates_status_and_timestamp() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("done");
        task_add(&store, &["t1".to_string()]).unwrap();
        let before = store.load_task("t1").unwrap().unwrap().updated_at;
        task_done(&store, &["t1".to_string()]).unwrap();
        let after = store.load_task("t1").unwrap().unwrap();
        assert_eq!(after.status, TaskStatus::Done);
        // updated_at must not regress; same-second writes may be equal.
        assert!(after.updated_at >= before);
    }

    #[test]
    fn done_missing_task_exits_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("donemissing");
        let code = task_done(&store, &["nope".to_string()]).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn status_missing_task_exits_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("statusmissing");
        let code = task_status(&store, &["nope".to_string()]).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn add_without_title_errors() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addnoargs");
        let r = task_add(&store, &[]);
        assert!(r.is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = task_cmd(&["bogus".to_string()]);
        assert!(r.is_err());
    }

    #[test]
    fn now_iso_format() {
        let s = now_iso();
        assert_eq!(s.len(), 20); // 2026-08-10T12:34:56Z
        assert!(s.ends_with('Z'));
        assert!(s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.as_bytes()[10] == b'T');
    }
}
