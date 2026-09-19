//! Integration tests for the ACP out-of-process backend.
//!
//! Each test scripts its scenario through the mock agent binary's env vars
//! and asserts on the observable [`SubagentResult`] — the wire details are
//! the mock's business, not the test's.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use omenic_harness_subagent::{
    AcpPermission, AcpProvider, AcpProviderSpec, SubagentProvider, SubagentResult,
    SubagentStartRequest,
};
use tempfile::{TempDir, tempdir};

/// The mock agent binary, built alongside the tests by the `[[bin]]` target.
const MOCK_BIN: &str = env!("CARGO_BIN_EXE_mock_acp_server");

/// Fast ladders: the tiers are exercised in milliseconds rather than the
/// spec's multi-second production defaults.
const EOF_GRACE: Duration = Duration::from_millis(600);
const KILL_GRACE: Duration = Duration::from_millis(300);

/// A run must settle well inside this, or something in the teardown path is
/// wedged — see [`bounded`].
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// One scenario, one scratch working directory (parallel runs never collide).
fn spec(dir: &TempDir, env: &[(&str, &str)]) -> AcpProviderSpec {
    AcpProviderSpec {
        command: MOCK_BIN.to_string(),
        args: Vec::new(),
        cwd: Some(dir.path().to_path_buf()),
        env: env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        permission: AcpPermission::Allow,
        eof_grace: EOF_GRACE,
        kill_grace: KILL_GRACE,
    }
}

fn request(prompt: &str) -> SubagentStartRequest {
    SubagentStartRequest {
        prompt: prompt.to_string(),
        signal: Arc::new(AtomicBool::new(false)),
        inherits_parent_context: false,
    }
}

fn start(spec: &AcpProviderSpec, prompt: &str) -> omenic_harness_subagent::SubagentRun {
    AcpProvider::new(spec.clone()).start(request(prompt))
}

/// Run `f` on a thread and fail loudly if it does not settle in time. A
/// broken dispose ladder or exit watcher would otherwise hang the whole test
/// binary until the CI timeout — better to name the suspect in the message.
fn bounded<T: Send + 'static>(timeout: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout)
        .expect("the run did not settle in time: dispose ladder or exit watcher is broken")
}

/// Block on the result, then tear the run down.
fn settle_then_dispose(run: omenic_harness_subagent::SubagentRun) -> SubagentResult {
    bounded(SETTLE_TIMEOUT, move || {
        let result = run.result();
        run.dispose();
        result
    })
}

/// Tear the (wedged) run down first — only then does the result arrive.
fn dispose_then_settle(run: omenic_harness_subagent::SubagentRun) -> SubagentResult {
    bounded(SETTLE_TIMEOUT, move || {
        run.dispose();
        run.result()
    })
}

fn expect_completed(result: SubagentResult) -> String {
    match result {
        SubagentResult::Completed { output } => output,
        other => panic!("expected Completed, got {other:?}"),
    }
}

/// 1. The happy path: `MOCK_TEXT` is streamed and collected, and a custom
/// `MOCK_SESSION_ID` is accepted (the turn could not complete otherwise).
#[test]
fn happy_path_streams_text() {
    let dir = tempdir().unwrap();
    let spec = spec(
        &dir,
        &[
            ("MOCK_TEXT", "hello from the agent"),
            ("MOCK_SESSION_ID", "test-session-42"),
        ],
    );
    let result = settle_then_dispose(start(&spec, "Do the thing"));
    assert_eq!(
        result,
        SubagentResult::Completed {
            output: "hello from the agent".to_string()
        }
    );
}

/// 2. `MOCK_ECHO_CWD`: the child streams its own cwd and the cwd announced at
/// `session/new`, so one assertion covers both the spawn and the handshake.
#[test]
fn echo_cwd_streams_working_directory() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_ECHO_CWD", "1")]);
    let result = settle_then_dispose(start(&spec, "Where are we?"));
    let output = expect_completed(result);
    let expected = dir.path().display().to_string();
    assert!(
        output.contains(&expected),
        "output must carry the working directory, got: {output}"
    );
}

/// 3. A stop reason that is not `end_turn` or `cancelled` is a failure —
/// never a silently completed turn (dsh's `acpStopReason`).
#[test]
fn non_terminal_stop_reason_fails() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_STOP", "max_turn_requests")]);
    let result = settle_then_dispose(start(&spec, "Hit the budget"));
    match result {
        SubagentResult::Failed { error } => {
            assert!(
                error.contains("max_turn_requests"),
                "error must name the stop reason, got: {error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// 4. `MOCK_NO_SESSION_ID`: the handshake is malformed, so the run fails and
/// the child is torn down (the worker's startup rollback).
#[test]
fn missing_session_id_fails_and_reaps_child() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_NO_SESSION_ID", "1")]);
    let started = Instant::now();
    let result = settle_then_dispose(start(&spec, "Never starts"));
    assert!(
        matches!(result, SubagentResult::Failed { .. }),
        "a session without an id must fail, got {result:?}"
    );
    // The rollback disposed the child already, so this dispose is the
    // idempotent no-op path — it must not walk the ladder again.
    assert!(
        started.elapsed() < SETTLE_TIMEOUT,
        "the failed startup must not hang"
    );
}

/// 5. `MOCK_PERMISSION` under the default `Allow` policy: the provider
/// approves the first offered option and the turn completes.
#[test]
fn permission_allow_completes_turn() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_PERMISSION", "1")]);
    let result = settle_then_dispose(start(&spec, "Needs approval"));
    assert_eq!(
        result,
        SubagentResult::Completed {
            output: "mock child answer".to_string()
        }
    );
}

/// 6. `MOCK_PERMISSION` under a `Reject` policy: the denied request settles
/// the turn `cancelled`, which the backend reports as an abort.
#[test]
fn permission_reject_aborts_turn() {
    let dir = tempdir().unwrap();
    let mut spec = spec(&dir, &[("MOCK_PERMISSION", "1")]);
    spec.permission = AcpPermission::Reject;
    let result = settle_then_dispose(start(&spec, "Needs approval"));
    assert!(
        matches!(result, SubagentResult::Aborted),
        "a denied permission must abort, got {result:?}"
    );
}

/// 7. `MOCK_HANG` + `MOCK_EOF_FLUSH_MS`: the child never answers and exits
/// `MOCK_EOF_FLUSH_MS` after EOF, so tier 1 reaps it *inside* `eof_grace`. If
/// the cooperative tier were broken the ladder would have to kill, and the
/// elapsed time would reach the grace first.
#[test]
fn hang_is_reaped_within_eof_grace() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_HANG", "1"), ("MOCK_EOF_FLUSH_MS", "200")]);
    let started = Instant::now();
    let result = dispose_then_settle(start(&spec, "Hang forever"));
    assert!(
        matches!(result, SubagentResult::Aborted),
        "a disposed hanging agent must abort, got {result:?}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < EOF_GRACE,
        "tier 1 must reap the child before eof_grace; took {elapsed:?}"
    );
}

/// 8. `MOCK_HANG` + `MOCK_IGNORE_CANCEL`: the child swallows the cancel and
/// outlives stdin EOF — cooperative teardown cannot touch it, so the ladder
/// must exhaust `eof_grace` and then SIGKILL.
#[test]
fn ignore_cancel_escalates_to_sigkill() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_HANG", "1"), ("MOCK_IGNORE_CANCEL", "1")]);
    let started = Instant::now();
    let result = dispose_then_settle(start(&spec, "Hang forever"));
    assert!(
        matches!(result, SubagentResult::Aborted),
        "a disposed hanging agent must abort, got {result:?}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= EOF_GRACE,
        "an unkillable child must exhaust eof_grace before SIGKILL; took {elapsed:?}"
    );
    assert!(
        elapsed < EOF_GRACE + KILL_GRACE + Duration::from_secs(2),
        "SIGKILL must end it promptly; took {elapsed:?}"
    );
}

/// 9. `MOCK_CRASH`: the agent dies mid-turn. No `dispose` is needed — the
/// exit watcher closes the transport, and the run fails instead of hanging.
#[test]
fn crash_fails_without_dispose() {
    let dir = tempdir().unwrap();
    let spec = spec(&dir, &[("MOCK_CRASH", "1")]);
    let result = settle_then_dispose(start(&spec, "Crash on prompt"));
    assert!(
        matches!(result, SubagentResult::Failed { .. }),
        "a crashed agent must fail, not hang, got {result:?}"
    );
}
