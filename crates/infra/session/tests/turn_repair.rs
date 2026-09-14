//! R2 2.4 — crash repair of a half-finished run log.
//!
//! Repair must close every `TurnStart` that never got a `TurnEnd` with an
//! `aborted` `TurnEnd`, and it must be *lossless*: the original records
//! survive untouched (prefix-preserved through the JSONL round-trip), and
//! re-repairing a repaired log changes nothing.

use session::{ABORTED, TurnRecord, decode_turn_log, encode_turn_log, interrupted_run_closers};

fn start(run: &str, ts: i64) -> TurnRecord {
    TurnRecord::TurnStart {
        run_id: run.into(),
        ts_ms: ts,
    }
}

fn end(run: &str, ts: i64, status: &str) -> TurnRecord {
    TurnRecord::TurnEnd {
        run_id: run.into(),
        ts_ms: ts,
        status: status.into(),
    }
}

#[test]
fn balanced_and_empty_logs_yield_no_closers() {
    assert!(interrupted_run_closers(&[]).is_empty());
    let balanced = [
        start("a", 1),
        end("a", 2, "ok"),
        start("b", 3),
        end("b", 4, "ok"),
    ];
    assert!(interrupted_run_closers(&balanced).is_empty());
}

#[test]
fn half_run_gets_aborted_closer_reusing_last_time() {
    let log = [start("a", 1), end("a", 2, "ok"), start("b", 3)];
    let closers = interrupted_run_closers(&log);
    assert_eq!(closers, vec![end("b", 3, ABORTED)]);
}

#[test]
fn multiple_open_runs_close_in_start_order() {
    let log = [start("late", 5), start("early", 2)];
    let closers = interrupted_run_closers(&log);
    // Sorted by start time; timestamps reuse the last real record (dsh rule).
    assert_eq!(
        closers,
        vec![end("early", 2, ABORTED), end("late", 2, ABORTED)]
    );
}

#[test]
fn round_trip_is_lossless_and_repair_idempotent() {
    let original = [start("a", 1), end("a", 2, "ok"), start("b", 3)];
    let mut repaired = original.to_vec();
    repaired.extend(interrupted_run_closers(&original));

    // Full JSONL round-trip: nothing lost, nothing reordered.
    let text = encode_turn_log(&repaired);
    let back = decode_turn_log(&text).expect("re-decode repaired log");
    assert_eq!(back, repaired);
    // The untouched messages/runs are still the log prefix ("不丢消息").
    assert_eq!(&back[..original.len()], &original[..]);

    // A repaired log is balanced: repairing again appends nothing.
    assert!(interrupted_run_closers(&back).is_empty());
}

#[test]
fn wire_tags_match_frozen_agent_event_vocabulary() {
    assert_eq!(
        serde_json::to_value(start("r", 9)).unwrap(),
        serde_json::json!({ "type": "turn_start", "run_id": "r", "ts_ms": 9 })
    );
    assert_eq!(
        serde_json::to_value(end("r", 10, ABORTED)).unwrap(),
        serde_json::json!({ "type": "turn_end", "run_id": "r", "ts_ms": 10, "status": "aborted" })
    );
}

#[test]
fn malformed_line_refuses_the_whole_log() {
    let text = "{\"type\":\"turn_start\",\"run_id\":\"a\",\"ts_ms\":1}\nnot json\n";
    let err = decode_turn_log(text).unwrap_err();
    assert!(matches!(
        err,
        session::SessionError::MalformedTurnLog { line: 2, .. }
    ));
}
