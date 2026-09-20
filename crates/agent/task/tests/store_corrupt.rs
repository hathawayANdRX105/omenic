//! Data-integrity tests for the jsonl load path shared by tasks / todos /
//! goals.
//!
//! `load_records` / `corrupt_or_trim` / `trim_trailing_line` are the single
//! code path all three jsonl files go through: a crash mid-append or an
//! external edit can leave a half-written line, and the store must trim a
//! *trailing* one while refusing to silently swallow a *mid-file* one.
//! `Store::append` only ever writes whole serialized records, so the torn
//! shapes below are written straight into the file.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use task::goal::Goal;
use task::store::{Store, StoreError};
use task::todo::Todo;
use task::{Task, TaskKind, TaskStatus, now_iso};

fn tmp_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path());
    (dir, store)
}

fn task(id: &str) -> Task {
    Task {
        id: id.into(),
        title: id.into(),
        kind: TaskKind::Task,
        status: TaskStatus::Open,
        attempts: 0,
        priority: 2,
        parent: None,
        deps: vec![],
        description: String::new(),
        acceptance: String::new(),
        created_at: now_iso(),
        updated_at: now_iso(),
    }
}

fn ids(tasks: &[Task]) -> Vec<String> {
    tasks.iter().map(|t| t.id.clone()).collect()
}

/// Append raw text (newline-terminated, no lock, no validation) to simulate
/// a crash mid-write or an external edit of the store.
fn corrupt_line(dir: &Path, file: &str, line: &str) {
    let mut f = OpenOptions::new()
        .append(true)
        .open(dir.join(file))
        .expect("open store file for raw append");
    writeln!(f, "{line}").expect("write raw line");
}

/// Write exact bytes (no newline appended) to simulate a crash between
/// `append_line`'s two `write_all` calls: the record bytes landed, the
/// `\n` never did.
fn write_raw(dir: &Path, file: &str, text: &str) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file))
        .expect("open store file for raw append");
    f.write_all(text.as_bytes()).expect("write raw bytes");
}

fn line_count(dir: &Path, file: &str) -> usize {
    std::fs::read_to_string(dir.join(file))
        .expect("read store file")
        .lines()
        .count()
}

#[test]
fn trailing_corrupt_line_is_trimmed_and_earlier_records_survive() {
    // Bug it catches: a crash mid-append leaves the last line half-written;
    // if that were fatal (or dropped every record) the whole task list
    // vanishes after any unclean shutdown instead of losing just the torn
    // line.
    let (dir, store) = tmp_store();
    store.append(&task("task-a")).unwrap();
    store.append(&task("task-b")).unwrap();
    corrupt_line(dir.path(), "tasks.jsonl", r#"{"id":"task-c","title":"half"#);

    let loaded = store.load_all().unwrap();
    assert_eq!(ids(&loaded), vec!["task-a", "task-b"]);

    assert_eq!(
        line_count(dir.path(), "tasks.jsonl"),
        2,
        "the torn line must be trimmed off the file, not just skipped in memory"
    );
    // The trimmed file is the new truth: reloading must not re-report it.
    assert_eq!(ids(&store.load_all().unwrap()), vec!["task-a", "task-b"]);
}

#[test]
fn mid_file_corrupt_line_is_error_with_line_number() {
    // Bug it catches: a torn line in the middle of the file is silently
    // skipped or trimmed away (or reported with an off-by-one line number),
    // so the operator gets no signal that a record was lost — and trimming
    // to "recover" would truncate the good lines after it.
    let (dir, store) = tmp_store();
    store.append(&task("task-a")).unwrap();
    corrupt_line(dir.path(), "tasks.jsonl", r#"{"id":"task-b","title":"torn"#);
    store.append(&task("task-c")).unwrap();

    match store.load_all() {
        Err(StoreError::CorruptLine { line, .. }) => {
            assert_eq!(
                line, 2,
                "line number is 1-based and must point at the torn line"
            )
        }
        other => panic!("expected CorruptLine at line 2, got {other:?}"),
    }
    assert_eq!(
        line_count(dir.path(), "tasks.jsonl"),
        3,
        "a mid-file error must not trim anything"
    );
}

#[test]
fn line_without_id_is_treated_as_corrupt() {
    // Bug it catches: a record with no `id` is accepted (then keyed by
    // nothing — panicking or silently dropping its siblings). An id-less
    // line is indistinguishable from a torn write, so it must take the same
    // trim-or-error path.
    let (dir, store) = tmp_store();
    store.append(&task("task-a")).unwrap();
    corrupt_line(dir.path(), "tasks.jsonl", r#"{"title":"no id"}"#);

    assert_eq!(ids(&store.load_all().unwrap()), vec!["task-a"]);
    assert_eq!(line_count(dir.path(), "tasks.jsonl"), 1);

    // Same shape, but mid-file: an error, not a trim.
    let (dir, store) = tmp_store();
    store.append(&task("task-a")).unwrap();
    corrupt_line(dir.path(), "tasks.jsonl", r#"{"title":"no id"}"#);
    store.append(&task("task-b")).unwrap();

    match store.load_all() {
        Err(StoreError::CorruptLine { line, .. }) => assert_eq!(line, 2),
        other => panic!("expected CorruptLine at line 2, got {other:?}"),
    }
}

#[test]
fn tombstone_removes_record_even_when_latest() {
    // Bug it catches: tombstone lines stop being honored, so a deleted task
    // stays on the board forever (latest-wins keeps resurrecting it), or the
    // tombstone line itself falls through into record parsing and is
    // reported as corruption.
    let (_dir, store) = tmp_store();
    let mut t = task("task-a");
    store.append(&t).unwrap();
    t.status = TaskStatus::InProgress;
    store.append(&t).unwrap();
    store.append_tombstone("task-a").unwrap();

    assert!(
        store.load_all().unwrap().is_empty(),
        "a tombstone as the latest line must delete the record"
    );

    // A tombstone for an id that was never appended is a no-op, not damage.
    store.append(&task("task-b")).unwrap();
    store.append_tombstone("never-existed").unwrap();
    assert_eq!(ids(&store.load_all().unwrap()), vec!["task-b"]);
}

#[test]
fn todos_and_goals_share_the_corrupt_path() {
    // Bug it catches: the generic load path is only correct for tasks — e.g.
    // the trim or tombstone branch hardcodes `tasks.jsonl` or a Task-only
    // shape — so todos and goals silently lose the same protection.
    let (dir, store) = tmp_store();
    store
        .append_todo(&Todo::new("todo-a".into()).unwrap())
        .unwrap();
    store
        .append_todo(&Todo::new("todo-b".into()).unwrap())
        .unwrap();
    corrupt_line(dir.path(), "todos.jsonl", r#"{"id":"todo-c","title":"torn"#);

    let loaded = store.load_todos().unwrap();
    assert_eq!(
        loaded.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        vec!["todo-a", "todo-b"]
    );
    assert_eq!(line_count(dir.path(), "todos.jsonl"), 2);

    let (dir, store) = tmp_store();
    store
        .append_goal(&Goal::new("goal-a".into()).unwrap())
        .unwrap();
    corrupt_line(dir.path(), "goals.jsonl", r#"{"id":"goal-b","title":"torn"#);
    store
        .append_goal(&Goal::new("goal-c".into()).unwrap())
        .unwrap();

    match store.load_goals() {
        Err(StoreError::CorruptLine { line, .. }) => assert_eq!(line, 2),
        other => panic!("expected CorruptLine at line 2, got {other:?}"),
    }
}

#[test]
fn empty_and_missing_files_yield_empty() {
    // Bug it catches: a missing or zero-byte file is reported as an error,
    // so a first-run `task.list` (no files yet) — or the state right after a
    // trim truncated the only line — fails instead of showing an empty
    // board.
    let (dir, store) = tmp_store();
    assert!(store.load_all().unwrap().is_empty());
    assert!(store.load_todos().unwrap().is_empty());
    assert!(store.load_goals().unwrap().is_empty());

    // Zero-byte files: exactly what `trim_trailing_line` leaves behind when
    // the store held a single torn line.
    for name in ["tasks.jsonl", "todos.jsonl", "goals.jsonl"] {
        std::fs::File::create(dir.path().join(name)).unwrap();
    }
    assert!(store.load_all().unwrap().is_empty());
    assert!(store.load_todos().unwrap().is_empty());
    assert!(store.load_goals().unwrap().is_empty());

    // The real chain: one torn line -> trimmed to 0 bytes -> reload.
    corrupt_line(dir.path(), "tasks.jsonl", r#"{"id":"task-a","titl"#);
    assert!(store.load_all().unwrap().is_empty());
    assert!(
        std::fs::read_to_string(dir.path().join("tasks.jsonl"))
            .unwrap()
            .is_empty()
    );
    assert!(store.load_all().unwrap().is_empty());
}

#[test]
fn trailing_corrupt_without_final_newline_keeps_last_complete_line() {
    // Bug it catches: `append_line` writes the record and its `\n` in two
    // separate `write_all` calls, so a crash (or power loss) between them
    // leaves the file with *no* trailing newline. A trim point computed by
    // walking back to the second-to-last newline then eats the last
    // *complete* record together with the torn one — every unclean shutdown
    // silently loses one good task.
    let (dir, store) = tmp_store();
    store.append(&task("task-a")).unwrap();
    store.append(&task("task-b")).unwrap();
    let before = std::fs::read_to_string(dir.path().join("tasks.jsonl")).unwrap();
    write_raw(dir.path(), "tasks.jsonl", r#"{"id":"task-c","title":"half"#);

    assert_eq!(ids(&store.load_all().unwrap()), vec!["task-a", "task-b"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tasks.jsonl")).unwrap(),
        before,
        "the trim must cut at the start of the torn line, not one line earlier"
    );
}

#[test]
fn trim_single_line_file_yields_empty() {
    // Bug it catches: when the store's only line is torn the trim must
    // truncate to zero bytes. Searching for a newline *before* the trailing
    // one has nothing to find, and a wrong fallback leaves a partial line
    // behind (or errors out), so a first-run crash leaves an unreadable
    // store instead of an empty one.
    let torn = r#"{"id":"task-a","title":"half"#;
    for shape in [torn.to_string(), format!("{torn}\n")] {
        let (dir, store) = tmp_store();
        write_raw(dir.path(), "tasks.jsonl", &shape);

        assert!(
            store.load_all().unwrap().is_empty(),
            "shape {shape:?} must trim to an empty board"
        );
        assert!(
            std::fs::read_to_string(dir.path().join("tasks.jsonl"))
                .unwrap()
                .is_empty(),
            "shape {shape:?} must leave a zero-byte file"
        );
    }
}

#[test]
fn trim_then_load_is_stable() {
    // Bug it catches: a cut that lands mid-record (or one record short)
    // leaves the file still ending in a corrupt line, so every subsequent
    // load re-trims and can eat another record — the store never converges.
    let (dir, store) = tmp_store();
    store.append(&task("task-a")).unwrap();
    store.append(&task("task-b")).unwrap();
    write_raw(dir.path(), "tasks.jsonl", r#"{"id":"task-c","title":"half"#);

    let first = ids(&store.load_all().unwrap());
    assert_eq!(first, vec!["task-a", "task-b"]);
    for _ in 0..3 {
        assert_eq!(
            ids(&store.load_all().unwrap()),
            first,
            "reloading a trimmed store must be a no-op"
        );
    }
    assert_eq!(
        line_count(dir.path(), "tasks.jsonl"),
        2,
        "repeated loads must not keep trimming"
    );
}
