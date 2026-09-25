//! G8 — run lifecycle: a run closes when the turn actually ends, not when
//! the prompt is acknowledged.
//!
//! In orbit mode `WorkerHandle::prompt` returns `{"started":true}` the moment
//! the message is queued to the engine's serial run thread; the turn keeps
//! running and its events (ending in `AgentEnd`) are pumped afterwards.
//! Before G8, dispatch finished the ledger run and wrote `TurnEnd` right
//! after that ack, so:
//!
//! * `in_flight_runs` was permanently 0 — every run looked done on arrival;
//! * the session turn-log entry was balanced immediately, so the half-open
//!   shape startup crash repair (`interrupted_run_closers`) exists to fix
//!   could never occur in a live log;
//! * the orbit/ack value `{"started":true}` was reported as the run's
//!   terminal result.
//!
//! The close now happens in the event pump, on `AgentEnd`. These tests pin
//! the three invariants that move is built on.

use daemon::state::{RunLedger, agent_end_status, aggregate_stats};
use session::TurnRecord;
use tempfile::tempdir;

/// `agent_end_status` maps the wire stop reason onto the ledger's own status
/// vocabulary. Budget caps are clean stops; an empty reason (legacy frame or
/// an omp-compat peer that never sends one) means the caller said nothing, so
/// it degrades to "ok" alongside every other reason we do not recognise.
#[test]
fn agent_end_status_maps_stop_reasons() {
    assert_eq!(agent_end_status("aborted"), "aborted");
    assert_eq!(agent_end_status("error"), "failed");
    assert_eq!(agent_end_status("end_turn"), "ok");
    assert_eq!(agent_end_status("max_tokens"), "ok");
    assert_eq!(agent_end_status("max_turns"), "ok");
    // Empty = the wire frame said nothing; degrade to ok rather than invent a
    // failure.
    assert_eq!(agent_end_status(""), "ok");
    // A reason a future orbit adds is still a clean stop by default.
    assert_eq!(agent_end_status("stop_reason_we_dont_know_yet"), "ok");
}

/// The turn-log shape a live orbit run leaves behind while it is still
/// running — and the shape a `kill -9` between ack and `AgentEnd` leaves
/// persisted: one `TurnStart`, no `TurnEnd`.
///
/// Before G8 this log could not occur in production at all: dispatch wrote
/// the `TurnEnd` on the ack, balancing the start away. Crash repair therefore
/// had nothing real to repair outside of session/tests. It does now.
#[test]
fn interrupted_run_closers_now_finds_half_open_runs() {
    let log = vec![TurnRecord::TurnStart {
        run_id: "r1".into(),
        ts_ms: 1_000,
    }];

    let closers = session::interrupted_run_closers(&log);

    assert_eq!(closers.len(), 1, "exactly one closer for the open run");
    match &closers[0] {
        TurnRecord::TurnEnd {
            run_id,
            status,
            ts_ms,
        } => {
            assert_eq!(run_id, "r1", "the closer names the open run");
            assert_eq!(status, session::ABORTED, "an interrupted run aborted");
            assert_eq!(*ts_ms, 1_000, "closers reuse the last log timestamp");
        }
        other => panic!("expected a TurnEnd closer, got {other:?}"),
    }
}

/// A run that started but has not ended is genuinely in flight — the honest
/// state of an orbit run between prompt-ack and `AgentEnd`. Receiving the end
/// closes it, which is exactly what the pump now does.
#[test]
fn in_flight_run_stays_open_until_agent_end() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("daemon.sock");
    let ledger = RunLedger::open_for_socket(&socket).expect("open ledger");

    // Prompt was acked; AgentEnd has not arrived yet.
    ledger
        .start("r-live", "s1", 1_000)
        .expect("ledger start (prompt ack)");
    let open = aggregate_stats(&ledger.list(), "all", 2_000);
    assert_eq!(
        open.in_flight_runs, 1,
        "a run whose AgentEnd has not landed is in flight"
    );
    assert_eq!(open.ok_runs, 0, "in-flight runs are not counted as ok");

    // The pump-side close: AgentEnd arrives for this run.
    ledger
        .finish("r-live", 2_500, agent_end_status("end_turn"))
        .expect("ledger finish (AgentEnd)");
    let closed = aggregate_stats(&ledger.list(), "all", 3_000);
    assert_eq!(
        closed.in_flight_runs, 0,
        "AgentEnd closes the run and it is no longer in flight"
    );
    assert_eq!(closed.ok_runs, 1, "an end_turn close counts as ok");
}

/// `finish` must be idempotent at the ledger level: the caller's "not already
/// finished" check and the write itself are two separate lock acquisitions, so
/// a repeated `AgentEnd` that wins the gap between them must not overwrite the
/// first close or append a second `TurnEnd`. The start/end balance is exactly
/// what `interrupted_run_closers` walks at startup.
#[test]
fn finish_twice_keeps_the_first_close() {
    let dir = tempdir().expect("temp dir");
    let socket = dir.path().join("daemon.sock");
    let ledger = RunLedger::open_for_socket(&socket).expect("open ledger");
    ledger.start("r-double", "s1", 1_000).expect("ledger start");
    ledger
        .finish("r-double", 2_000, "ok")
        .expect("first finish");
    // A late duplicate close for the same run.
    ledger
        .finish("r-double", 9_999, "failed")
        .expect("second finish is not an error, just a no-op");

    let run = ledger.get("r-double").expect("run exists");
    assert_eq!(
        run.finished_at_ms,
        Some(2_000),
        "the first close must survive a duplicate"
    );
    assert_eq!(
        run.status.as_deref(),
        Some("ok"),
        "the first status must survive a duplicate: {:?}",
        run.status
    );
}
