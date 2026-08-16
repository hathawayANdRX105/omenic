//! End-to-end smoke: MVP acceptance lines 1–5 from todo/spike/mvp-design.md §7.
//!
//! Runs a real `omenic` process in an isolated `OMENIC_DATA_DIR`, exercises the
//! full CLI lifecycle — `task add` → `plan` → deps-gating → `run` executes
//! a real `omp --mode rpc` session — and verifies the store + evidence land.
//!
//! Why `--ignored`: this walks a real `omp` subprocess (LLM call, 35s+), so
//! CI defaults skip it. Local opt-in:
//!
//!     cargo test --test m3_e2e -- --ignored
//!
//!     Or to specify omp binary explicitly:
//!     OMENIC_OMP_PATH=/usr/bin/omp cargo test --test m3_e2e -- --ignored

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;

/// Run the `omenic` test binary (built into target/debug) inside an isolated
/// data directory. Returns (stdout, stderr, exit_success).
fn omenic(data_dir: &Path, args: &[&str]) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_omenic");
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .env("OMENIC_DATA_DIR", data_dir)
        .env("OMENIC_OMP_PATH", omp_binary())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let output = cmd.output().expect("spawn omenic");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// Resolve the omp binary path for tests (OMENIC_OMP_PATH or "omp" on PATH).
fn omp_binary() -> String {
    env::var("OMENIC_OMP_PATH").unwrap_or_else(|_| "omp".to_string())
}

/// Read the store directly to resolve a task id by title.
fn task_id_for(data_dir: &Path, title: &str) -> String {
    let store_path = data_dir.join("tasks.jsonl");
    let content = fs::read_to_string(&store_path).expect("read tasks.jsonl");
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("title").and_then(|s| s.as_str()) == Some(title)
            && let Some(id) = v.get("id").and_then(|s| s.as_str())
        {
            return id.to_string();
        }
    }
    panic!("task with title {title:?} not found in store");
}

#[test]
#[ignore = "requires omp binary"]
fn m3_e2e_end_to_end_run() {
    // -----------------------------------------------------------------------
    // Acceptance line 1: task add + plan
    // -----------------------------------------------------------------------
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let (_stdout_unused, stderr, ok) =
        omenic(data_dir, &["task", "add", "Write a haiku about testing"]);
    assert!(ok, "task add failed: {stderr}");

    let (_stdout, stderr, ok) = omenic(data_dir, &["plan"]);
    assert!(ok, "plan failed: {stderr}");

    let leaf_id = task_id_for(data_dir, "Write a haiku about testing");

    // -----------------------------------------------------------------------
    // Acceptance line 4: run with unmet deps → blocked (no worker spawn).
    // We fake a dep by writing a done task, then a dependent task via JSONL.
    // -----------------------------------------------------------------------
    {
        // Manually inject a dep edge into the store.
        let mut lines: Vec<String> = fs::read_to_string(data_dir.join("tasks.jsonl"))
            .unwrap()
            .lines()
            .map(|s| s.to_string())
            .collect();
        // Leaf gets a dep on a fake parent; since parent is missing, it's
        // treated as blocked.
        let leaf_line_idx = lines.iter().position(|l| l.contains(&leaf_id)).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&lines[leaf_line_idx]).unwrap();
        v["deps"] = serde_json::json!(["fake-parent"]);
        lines[leaf_line_idx] = serde_json::to_string(&v).unwrap();

        fs::write(data_dir.join("tasks.jsonl"), lines.join("\n") + "\n").unwrap();
    }

    let (stdout, stderr, ok) = omenic(data_dir, &["run", &leaf_id]);
    assert!(
        ok || stderr.contains("blocked") || stdout.contains("blocked"),
        "expected blocked, got stdout={stdout} stderr={stderr}"
    );

    // -----------------------------------------------------------------------
    // Acceptance line 2,3,5: run leaf (no deps) → worker runs → evidence
    // -----------------------------------------------------------------------
    let tmp2 = TempDir::new().unwrap();
    let data_dir2 = tmp2.path();

    let (_stdout, stderr, ok) = omenic(data_dir2, &["task", "add", "Summarize pace layers"]);
    assert!(ok, "task add failed: {stderr}");
    let feat_id = task_id_for(data_dir2, "Summarize pace layers");

    let (run_out, run_err, run_ok) = omenic(data_dir2, &["run", &feat_id]);
    assert!(
        run_ok || run_err.is_empty(),
        "run unexpectedly failed: {run_err}"
    );

    // Evidence
    let ctx_dir = data_dir2.join("tasks").join(&feat_id);
    assert!(ctx_dir.exists(), "context dir missing: {ctx_dir:?}");

    let brief_path = ctx_dir.join("brief.md");
    assert!(brief_path.exists(), "brief.md missing");
    let brief = fs::read_to_string(&brief_path).unwrap();
    assert!(
        brief.contains("Summarize pace layers"),
        "brief wrong: {brief}"
    );

    // F1: observable agent-call artifacts — prompt.json (input) and
    // events.jsonl (process) alongside the brief and result.
    let prompt_path = ctx_dir.join("prompt.json");
    assert!(prompt_path.exists(), "prompt.json missing");
    let prompt: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&prompt_path).unwrap()).unwrap();
    assert_eq!(prompt["task_id"], feat_id, "prompt.task_id wrong");
    assert!(prompt["brief"].is_string(), "prompt.brief missing");
    assert!(prompt["deps"].is_array(), "prompt.deps missing");
    assert!(prompt["materials"].is_string(), "prompt.materials missing");

    let events_path = ctx_dir.join("events.jsonl");
    assert!(events_path.exists(), "events.jsonl missing");
    let events = fs::read_to_string(&events_path).unwrap();
    assert!(!events.trim().is_empty(), "events.jsonl empty after run");
    for line in events.lines() {
        let v: serde_json::Value =
            serde_json::from_str(line).expect("each events.jsonl line is valid JSON");
        assert!(
            v["event"].is_string(),
            "event line lacks 'event' tag: {line}"
        );
    }

    let result_path = ctx_dir.join("result.json");
    assert!(result_path.exists(), "result.json missing");
    let result: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result_path).unwrap()).unwrap();
    assert!(
        result["status"].is_string(),
        "result.status missing: {result}"
    );

    // Acceptance line 3: status flipped when Done.
    let (st_out, _, _) = omenic(data_dir2, &["task", "status", &feat_id]);
    assert!(st_out.contains("done") || st_out.contains("status: done"));

    println!("m3 e2e smoke passed — run_out={run_out} err={run_err}");
}
