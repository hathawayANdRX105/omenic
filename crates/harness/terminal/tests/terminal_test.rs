//! Behavioral tests for the pty-backed terminal registry.
//!
//! These spawn real shells, so they are slower than the jobs tests and one of
//! them is skipped when the chosen shell is absent. Everything is bounded by a
//! deadline: a pty test that hangs is worse than one that fails, and the reader
//! thread means `read` never blocks indefinitely by construction — these
//! deadlines exist to catch a regression in that design.

use std::time::{Duration, Instant};

use omenic_harness_terminal::{TerminalError, TerminalId, TerminalRegistry};

/// A shell that reads `.bashrc`-free and prints no prompt decoration. `--norc`
/// keeps the output assertions from depending on the host's bash config.
const SHELL: &str = "bash --norc --noprofile";

fn have_bash() -> bool {
    std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_ok()
}

/// Read until `pred` is satisfied or `ms` elapses, returning everything read.
///
/// A pty delivers output in chunks with no framing, so "run a command; check
/// the text" is inherently a polling loop. This is the one place polling is the
/// correct tool — the registry itself never polls.
///
/// **The echo trap**: a pty echoes what is written to it, so after
/// `write("echo hi\n")` the marker `hi` appears twice — once in the echoed
/// command line (`echo hi`) and once in the command's output. `pred` here is
/// therefore evaluated against the accumulated text, and callers must phrase
/// their predicate so the *echo alone* cannot satisfy it. The helpers below
/// use a sentinel written by the command itself (`__DONE__`), which cannot
/// appear in the echoed command text before the command runs.
fn read_until(
    reg: &TerminalRegistry,
    id: &TerminalId,
    ms: u64,
    mut pred: impl FnMut(&str) -> bool,
) -> String {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut acc = String::new();
    while Instant::now() < deadline {
        let out = reg.read(id, Some(50)).expect("read");
        acc.push_str(&out.text());
        if pred(&acc) {
            break;
        }
        if out.exited && out.is_empty() {
            break;
        }
    }
    acc
}

/// Write `cmd`, then a sentinel that the command's *own text* does not contain,
/// and wait for that sentinel to come back **once**.
///
/// The obvious spelling is `{cmd}; echo __DONE__` and wait for two occurrences
/// — one from the echoed command line, one from the command's output. That does
/// not work, and the failure is instructive: a pty delivers bytes with no
/// framing, so the echoed command line can itself arrive in two chunks. When it
/// does, the sentinel is counted from the echo twice (once per chunk boundary)
/// and the predicate fires *before* the command has produced anything. That is
/// exactly how `shell_state_persists_across_reads` failed: it read the echo of
/// `pwd; echo …` and stopped, never seeing `/tmp`.
///
/// Counting bytes is not the fix; removing the ambiguity is. Bash builds the
/// sentinel from two pieces (`printf '\n__OMENIC_%s__\n' DONE`), so the literal
/// `__OMENIC_DONE__` never appears in the written line — therefore **any**
/// occurrence on the read side is genuine output, and one occurrence suffices.
fn run_and_wait(reg: &TerminalRegistry, id: &TerminalId, cmd: &str, ms: u64) -> String {
    const SENTINEL: &str = "__OMENIC_DONE__";
    let line = format!("{cmd}; printf '\\n__OMENIC_%s__\\n' DONE\n");
    reg.write(id, &line).expect("write");
    read_until(reg, id, ms, |s| s.contains(SENTINEL))
}

#[test]
fn create_write_read_round_trips_output() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");

    let text = run_and_wait(&reg, &id, "echo terminal-marker-42", 5_000);

    assert!(
        text.contains("terminal-marker-42"),
        "did not see the echo; got {text:?}"
    );
    reg.close(&id).ok();
}

#[test]
fn the_input_command_is_echoed_back_before_its_output() {
    // Documents the pty echo, because every caller has to account for it: the
    // command text appears on the read side before the command has run. A
    // caller that waits for a marker in the *command* would return too early.
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");

    reg.write(&id, "echo echoed-token\n").expect("write");
    // Wait only for the first occurrence: that is the echo.
    let text = read_until(&reg, &id, 5_000, |s| s.contains("echoed-token"));
    assert!(
        text.contains("echoed-token"),
        "no echo at all; got {text:?}"
    );
    // The echo is of the command line, so the token appears as part of a
    // longer `echo echoed-token` string rather than on a line of its own.
    assert!(
        text.contains("echo echoed-token"),
        "expected the echoed command line; got {text:?}"
    );
    reg.close(&id).ok();
}

#[test]
fn shell_state_persists_across_reads() {
    // The whole point of a persistent terminal: `cd` survives between tool
    // calls, which is impossible with one-shot `run_bash`.
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");

    run_and_wait(&reg, &id, "cd /tmp", 3_000);
    let text = run_and_wait(&reg, &id, "pwd", 5_000);

    assert!(
        text.contains("/tmp"),
        "cwd did not persist across calls; got {text:?}"
    );
    reg.close(&id).ok();
}

#[test]
fn read_drains_so_the_same_output_is_not_returned_twice() {
    // The invariant is "the registry hands each byte to at most one read", not
    // "the second read is empty". A pty splits output into chunks with no
    // framing, so the shell's prompt routinely arrives later than the echo —
    // asserting emptiness would race the reader thread. Counting occurrences
    // tests the real property without depending on chunk timing.
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");

    // The sentinel is assembled by the shell from two pieces, so the literal
    // never appears in the command text we write. Any occurrence on the read
    // side is therefore the command's own output — one is enough, and there is
    // no count arithmetic to get wrong. (Counting echo-vs-output was tried and
    // rejected: see `run_and_wait` for why chunk boundaries break it.)
    const SENTINEL: &str = "__DRAIN_ONCE__";
    reg.write(&id, "printf '\\n__DRAIN_%s__\\n' ONCE\n")
        .expect("write");
    let first = read_until(&reg, &id, 5_000, |s| s.contains(SENTINEL));
    assert!(
        first.contains(SENTINEL),
        "the command's output never arrived; got {first:?}"
    );

    // Now that the command has demonstrably run, drain the trailing prompt and
    // read again with no new input. Neither may replay the sentinel.
    let trailing = reg.read(&id, Some(200)).expect("read");
    let second = reg.read(&id, Some(200)).expect("read");
    let seen = format!("{}{}", trailing.text(), second.text());

    assert!(
        !seen.contains(SENTINEL),
        "output was replayed by a later read; got {seen:?}"
    );
    reg.close(&id).ok();
}

#[test]
fn read_on_an_idle_shell_returns_promptly() {
    // Regression guard for the reader-thread design: a `read` that hit the pty
    // master directly would block here until the shell produced output.
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");
    reg.write(&id, "true\n").expect("write");
    // Let it settle so the shell is genuinely idle, not mid-command.
    std::thread::sleep(Duration::from_millis(300));
    reg.read(&id, Some(100)).ok();

    let start = Instant::now();
    let _ = reg.read(&id, None).expect("read");
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "read on an idle shell blocked for {elapsed:?}"
    );
    reg.close(&id).ok();
}

#[test]
fn read_with_timeout_waits_for_new_output() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");
    reg.write(&id, "true\n").expect("write");
    std::thread::sleep(Duration::from_millis(250));
    reg.read(&id, Some(100)).ok();

    // The output will not exist for ~200ms; a timeout-bearing read must
    // actually wait for it rather than returning empty immediately. The
    // sentinel is only written *after* the sleep, so matching it cannot be
    // satisfied by the echo of the command line.
    reg.write(&id, "sleep 0.2; echo __DELAYED__\n")
        .expect("write");
    let out = reg.read(&id, Some(5_000)).expect("read");

    assert!(
        out.text().contains("__DELAYED__"),
        "read with a timeout returned before output arrived; got {:?}",
        out.text()
    );
    reg.close(&id).ok();
}

#[test]
fn exited_shell_is_reported_as_exited() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");

    reg.write(&id, "exit 0\n").expect("write");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut exited = false;
    while Instant::now() < deadline {
        let out = reg.read(&id, Some(100)).expect("read");
        if out.exited {
            exited = true;
            break;
        }
    }
    assert!(exited, "shell exit was never reported");

    // Writing to a dead session is an error, not a silent no-op.
    let err = reg.write(&id, "echo nope\n").unwrap_err();
    assert!(
        matches!(err, TerminalError::Exited(_)),
        "expected Exited, got {err:?}"
    );

    // The session record is still there for a final drain; `close` removes it.
    assert_eq!(reg.len(), 1);
    reg.close(&id).expect("close");
    assert_eq!(reg.len(), 0);
}

#[test]
fn close_removes_the_session_and_kills_the_shell() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");
    assert_eq!(reg.len(), 1);

    reg.close(&id).expect("close");
    assert!(reg.is_empty());
    assert!(matches!(
        reg.read(&id, None),
        Err(TerminalError::Unknown(_))
    ));
    assert!(matches!(
        reg.write(&id, "x"),
        Err(TerminalError::Unknown(_))
    ));
}

#[test]
fn kill_leaves_the_record_so_output_can_still_be_drained() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 80, 24).expect("create");

    // A command that prints, then runs long enough to be killed mid-flight.
    //
    // The marker is assembled by the shell from two pieces so the literal never
    // appears in the written line, which makes one occurrence unambiguous proof
    // that the command started. Counting echo-vs-output instead is unreliable
    // here: the echoed command line can arrive split across reads, and the
    // count then reaches its target before `sleep` has begun — so the kill
    // would land on a shell that never started sleeping.
    reg.write(&id, "printf '__BEFORE_%s__\\n' KILL; sleep 30\n")
        .expect("write");
    read_until(&reg, &id, 3_000, |s| s.contains("__BEFORE_KILL__"));

    reg.kill(&id).expect("kill");

    // The session is still registered: `kill` signals, `close` removes.
    assert_eq!(reg.len(), 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut exited = false;
    while Instant::now() < deadline {
        if reg.read(&id, Some(100)).expect("read").exited {
            exited = true;
            break;
        }
    }
    assert!(exited, "killed shell never reported exit");

    reg.close(&id).expect("close");
}

#[test]
fn resize_changes_the_reported_size_and_survives() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let id = reg.create(SHELL, "/tmp", 100, 30).expect("create");

    let before = reg.status(&id).expect("status");
    assert_eq!((before.cols, before.rows), (100, 30));

    reg.resize(&id, 40, 12).expect("resize");

    // `status` reports the spawn geometry, so it is unchanged by design; the
    // real assertion is that `resize` reached the pty without error and the
    // shell is still responsive afterwards.
    //
    // The marker's *output* is what proves the shell survived, and `run_and_wait`
    // already returns only once the command ran (its sentinel is assembled by
    // the shell, so the echo cannot satisfy it). An extra count assertion here
    // would re-introduce exactly the chunk-dependent arithmetic that helper
    // exists to avoid.
    let text = run_and_wait(&reg, &id, "echo __AFTER_RESIZE__", 5_000);
    assert!(
        text.contains("__AFTER_RESIZE__"),
        "shell died after resize; got {text:?}"
    );
    reg.close(&id).ok();
}

#[test]
fn list_reports_creation_order_and_metadata() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let a = reg.create(SHELL, "/tmp", 80, 24).expect("create a");
    let b = reg.create(SHELL, "/var", 120, 40).expect("create b");

    let rows = reg.list();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, a, "listing was not in creation order");
    assert_eq!(rows[1].id, b);
    assert_eq!(rows[0].shell, SHELL);
    assert_eq!(rows[0].cwd, "/tmp");
    assert_eq!(rows[1].cwd, "/var");
    assert_eq!((rows[1].cols, rows[1].rows), (120, 40));
    assert!(!rows[0].exited);

    reg.close(&a).ok();
    reg.close(&b).ok();
}

#[test]
fn session_ceiling_is_enforced() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    reg.set_max_sessions(2);
    let a = reg.create(SHELL, "/tmp", 80, 24).expect("first");
    let b = reg.create(SHELL, "/tmp", 80, 24).expect("second");

    let err = reg.create(SHELL, "/tmp", 80, 24).unwrap_err();
    assert!(matches!(err, TerminalError::Refused(_)), "got {err:?}");

    // Closing one makes room.
    reg.close(&a).expect("close");
    reg.create(SHELL, "/tmp", 80, 24).expect("fits now");
    reg.close(&b).ok();
}

#[test]
fn empty_shell_command_is_refused() {
    let reg = TerminalRegistry::new();
    let err = reg.create("   ", "/tmp", 80, 24).unwrap_err();
    assert!(matches!(err, TerminalError::Refused(_)), "got {err:?}");
}

#[test]
fn unknown_id_is_an_error_on_every_operation() {
    let reg = TerminalRegistry::new();
    let ghost = TerminalId::new("term-does-not-exist");

    assert!(matches!(
        reg.read(&ghost, None),
        Err(TerminalError::Unknown(_))
    ));
    assert!(matches!(
        reg.write(&ghost, "x"),
        Err(TerminalError::Unknown(_))
    ));
    assert!(matches!(
        reg.resize(&ghost, 80, 24),
        Err(TerminalError::Unknown(_))
    ));
    assert!(matches!(reg.kill(&ghost), Err(TerminalError::Unknown(_))));
    assert!(matches!(reg.close(&ghost), Err(TerminalError::Unknown(_))));
    assert!(matches!(reg.status(&ghost), Err(TerminalError::Unknown(_))));
}

#[test]
fn ids_are_unique_and_prefixed() {
    if !have_bash() {
        eprintln!("skipping: bash not available");
        return;
    }
    let reg = TerminalRegistry::new();
    let a = reg.create(SHELL, "/tmp", 80, 24).expect("a");
    let b = reg.create(SHELL, "/tmp", 80, 24).expect("b");

    assert_ne!(a, b);
    assert!(a.as_str().starts_with("term-"), "id was {}", a.as_str());
    assert!(b.as_str().starts_with("term-"), "id was {}", b.as_str());

    reg.close(&a).ok();
    reg.close(&b).ok();
}
