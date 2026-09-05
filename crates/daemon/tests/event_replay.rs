use daemon::state::RunLedger;

#[test]
fn cursor_replay_is_exclusive_and_monotonic() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = RunLedger::open_for_socket(&dir.path().join("daemon.sock")).unwrap();

    ledger.start("run-1", "session-1", 10).unwrap();
    ledger.start("run-2", "session-1", 20).unwrap();

    let (all, cursor) = ledger.read_from_cursor(0);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].seq, 1);
    assert_eq!(all[1].seq, 2);
    assert_eq!(cursor, 2);

    let (none, unchanged) = ledger.read_from_cursor(cursor);
    assert!(none.is_empty());
    assert_eq!(unchanged, cursor);

    ledger.start("run-3", "session-1", 30).unwrap();
    let (replayed, next_cursor) = ledger.read_from_cursor(cursor);
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].run_id, "run-3");
    assert_eq!(replayed[0].seq, 3);
    assert_eq!(next_cursor, 3);
}
