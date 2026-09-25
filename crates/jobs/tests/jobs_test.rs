//! State-machine and concurrency tests for `LocalJobRegistry`.
//!
//! These lean on `wait` with an explicit timeout rather than `sleep`-then-
//! assert, so a slow CI machine cannot make them flaky in the "job finished
//! too fast" direction, and a broken implementation fails fast instead of
//! hanging.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use jobs::{JobError, JobId, JobOutput, JobRegistry, JobState, LocalJobRegistry};

/// A task that completes immediately with `stdout`.
fn completes(stdout: &'static str) -> jobs::JobTask {
    Box::new(move |_ctl| {
        Ok(JobOutput {
            stdout: stdout.into(),
            ..JobOutput::default()
        })
    })
}

#[test]
fn completed_job_reports_its_output() {
    let reg = LocalJobRegistry::new();
    let id = reg.start("echo hi", completes("hi\n")).unwrap();

    let out = reg.wait(&id, Some(5_000)).unwrap();
    assert_eq!(out.stdout, "hi\n");

    let summary = reg.status(&id).unwrap();
    assert_eq!(summary.state, JobState::Completed);
    assert_eq!(summary.label, "echo hi");
}

#[test]
fn failing_task_lands_in_failed_with_stderr() {
    let reg = LocalJobRegistry::new();
    let id = reg
        .start("boom", Box::new(|_ctl| Err("exit status 3".to_string())))
        .unwrap();

    let out = reg.wait(&id, Some(5_000)).unwrap();
    assert_eq!(out.stderr, "exit status 3");

    assert_eq!(reg.status(&id).unwrap().state, JobState::Failed);
}

#[test]
fn wait_without_timeout_returns_once_the_job_finishes() {
    let reg = LocalJobRegistry::new();
    let id = reg
        .start(
            "delayed",
            Box::new(|_ctl| {
                std::thread::sleep(Duration::from_millis(120));
                Ok(JobOutput {
                    stdout: "late".into(),
                    ..JobOutput::default()
                })
            }),
        )
        .unwrap();

    // No timeout: this must block until the sleep above finishes, not return
    // early with an empty result.
    let out = reg.wait(&id, None).unwrap();
    assert_eq!(out.stdout, "late");
}

#[test]
fn wait_with_zero_timeout_reports_still_running() {
    let reg = LocalJobRegistry::new();
    let release = Arc::new(std::sync::Barrier::new(2));

    let gate = Arc::clone(&release);
    let id = reg
        .start(
            "gated",
            Box::new(move |_ctl| {
                gate.wait();
                Ok(JobOutput::default())
            }),
        )
        .unwrap();

    let err = reg.wait(&id, Some(0)).unwrap_err();
    assert!(
        matches!(err, JobError::StillRunning(ref got) if got == &id),
        "expected StillRunning, got {err:?}"
    );

    release.wait();
    // The job is untouched by the timeout — still waitable, and it finishes.
    reg.wait(&id, Some(5_000)).unwrap();
    assert_eq!(reg.status(&id).unwrap().state, JobState::Completed);
}

#[test]
fn wait_with_short_timeout_reports_still_running_and_leaves_the_job_alone() {
    let reg = LocalJobRegistry::new();
    let id = reg
        .start(
            "slow",
            Box::new(|_ctl| {
                std::thread::sleep(Duration::from_millis(400));
                Ok(JobOutput {
                    stdout: "done".into(),
                    ..JobOutput::default()
                })
            }),
        )
        .unwrap();

    let err = reg.wait(&id, Some(50)).unwrap_err();
    assert!(matches!(err, JobError::StillRunning(_)), "got {err:?}");

    // The short timeout did not cancel it: the real output still arrives.
    assert_eq!(reg.wait(&id, None).unwrap().stdout, "done");
}

#[test]
fn kill_transitions_to_killed_and_is_idempotent() {
    let reg = LocalJobRegistry::new();
    let release = Arc::new(std::sync::Barrier::new(2));

    let gate = Arc::clone(&release);
    let id = reg
        .start(
            "long",
            Box::new(move |ctl| {
                gate.wait();
                // Report honestly: the flag was set before the task returned.
                let _ = ctl.is_cancelled();
                Ok(JobOutput::default())
            }),
        )
        .unwrap();

    assert_eq!(reg.kill(&id).unwrap(), JobState::Running);
    // Killing a terminal job is a no-op that reports the existing state.
    assert_eq!(reg.kill(&id).unwrap(), JobState::Killed);

    assert_eq!(reg.status(&id).unwrap().state, JobState::Killed);

    release.wait();
    // Still killed after the task body returns — `finish_locked` must not
    // upgrade a killed record back to completed.
    reg.wait(&id, Some(5_000)).unwrap();
    assert_eq!(reg.status(&id).unwrap().state, JobState::Killed);
}

#[test]
fn kill_on_a_finished_job_reports_the_terminal_state() {
    let reg = LocalJobRegistry::new();
    let id = reg.start("quick", completes("ok")).unwrap();
    reg.wait(&id, Some(5_000)).unwrap();

    assert_eq!(reg.kill(&id).unwrap(), JobState::Completed);
}

#[test]
fn unknown_id_is_an_error_on_every_lookup() {
    let reg = LocalJobRegistry::new();
    let ghost = JobId::new("job-does-not-exist");

    assert!(matches!(reg.status(&ghost), Err(JobError::Unknown(_))));
    assert!(matches!(reg.kill(&ghost), Err(JobError::Unknown(_))));
    assert!(matches!(
        reg.wait(&ghost, Some(0)),
        Err(JobError::Unknown(_))
    ));
    assert!(!reg.remove(&ghost));
}

#[test]
fn list_is_newest_first_and_includes_terminal_jobs() {
    let reg = LocalJobRegistry::new();
    let first = reg.start("first", completes("1")).unwrap();
    reg.wait(&first, Some(5_000)).unwrap();
    let second = reg.start("second", completes("2")).unwrap();
    reg.wait(&second, Some(5_000)).unwrap();

    let rows = reg.list();
    assert_eq!(rows.len(), 2);
    // Newest first: `second` was started after `first`.
    assert_eq!(rows[0].id, second);
    assert_eq!(rows[1].id, first);
    assert_eq!(rows[0].label, "second");
}

#[test]
fn terminal_jobs_can_be_removed_but_running_ones_cannot() {
    let reg = LocalJobRegistry::new();
    let release = Arc::new(std::sync::Barrier::new(2));

    let gate = Arc::clone(&release);
    let running = reg
        .start(
            "held",
            Box::new(move |_ctl| {
                gate.wait();
                Ok(JobOutput::default())
            }),
        )
        .unwrap();
    let done = reg.start("done", completes("x")).unwrap();
    reg.wait(&done, Some(5_000)).unwrap();

    // Removing a running job would leak its join handle and make it
    // unkillable, so it is refused.
    assert!(!reg.remove(&running));
    assert!(reg.remove(&done));
    assert!(matches!(reg.status(&done), Err(JobError::Unknown(_))));

    release.wait();
    reg.wait(&running, Some(5_000)).unwrap();
}

#[test]
fn tasks_actually_run_concurrently_not_serially() {
    // Eight jobs each holding a barrier: if `start` ran the task inline this
    // would deadlock instead of completing.
    const N: usize = 8;
    let reg = LocalJobRegistry::new();
    let barrier = Arc::new(std::sync::Barrier::new(N));

    let mut ids = Vec::new();
    for i in 0..N {
        let gate = Arc::clone(&barrier);
        ids.push(
            reg.start(
                format!("concurrent-{i}"),
                Box::new(move |_ctl| {
                    gate.wait();
                    Ok(JobOutput::default())
                }),
            )
            .unwrap(),
        );
    }

    for id in ids {
        reg.wait(&id, Some(10_000)).unwrap();
        assert_eq!(reg.status(&id).unwrap().state, JobState::Completed);
    }
}

#[test]
fn running_job_ceiling_refuses_further_starts() {
    let reg = LocalJobRegistry::new();
    reg.set_max_running(2);
    let release = Arc::new(std::sync::Barrier::new(3));

    let mut ids = Vec::new();
    for i in 0..2 {
        let gate = Arc::clone(&release);
        ids.push(
            reg.start(
                format!("held-{i}"),
                Box::new(move |_ctl| {
                    gate.wait();
                    Ok(JobOutput::default())
                }),
            )
            .unwrap(),
        );
    }

    let err = reg.start("one-too-many", completes("no")).unwrap_err();
    assert!(matches!(err, JobError::Refused(_)), "got {err:?}");

    // Finish the held jobs; now there is room again.
    release.wait();
    for id in &ids {
        reg.wait(id, Some(5_000)).unwrap();
    }
    // Terminal jobs do not count toward the ceiling.
    reg.start("fits-now", completes("yes")).unwrap();
}

#[test]
fn shutdown_cancels_running_jobs_and_refuses_new_work() {
    let reg = LocalJobRegistry::new();
    let release = Arc::new(std::sync::Barrier::new(2));
    let observed = Arc::new(AtomicUsize::new(0));

    let gate = Arc::clone(&release);
    let seen = Arc::clone(&observed);
    let id = reg
        .start(
            "held",
            Box::new(move |ctl| {
                gate.wait();
                // The registry set the flag during shutdown; record that the
                // task can see it before returning.
                seen.store(ctl.is_cancelled() as usize, Ordering::SeqCst);
                Ok(JobOutput::default())
            }),
        )
        .unwrap();

    reg.shutdown();

    // Unknown after shutdown: the record was dropped with the registry.
    assert!(matches!(reg.status(&id), Err(JobError::Unknown(_))));
    assert!(matches!(
        reg.start("nope", completes("x")),
        Err(JobError::Refused(_))
    ));

    release.wait();
    // The task body still saw the cancellation flag.
    // Spin briefly: the task sets it after `gate.wait()` returns, which races
    // this assertion only by nanoseconds, but a bounded wait keeps it honest.
    for _ in 0..200 {
        if observed.load(Ordering::SeqCst) == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(observed.load(Ordering::SeqCst), 1, "task never saw cancel");
}

#[test]
fn ids_are_unique_and_prefixed() {
    let reg = LocalJobRegistry::new();
    let a = reg.start("a", completes("a")).unwrap();
    let b = reg.start("b", completes("b")).unwrap();

    assert_ne!(a, b);
    assert!(a.as_str().starts_with("job-"), "id was {}", a.as_str());
    assert!(b.as_str().starts_with("job-"), "id was {}", b.as_str());
}
