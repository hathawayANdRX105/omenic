//! Semaphore / map_limited unit tests: concurrency bounds, abortable
//! acquire, resize semantics, and index-stable all-settled collection.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use subagent::parallel::{Aborted, MapError, Semaphore, map_limited};

#[test]
fn acquire_bounds_concurrency() {
    let sem = Arc::new(Semaphore::new(2));
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|s| {
        for _ in 0..6 {
            let sem = &sem;
            let live = &live;
            let peak = &peak;
            s.spawn(move || {
                let g = sem.acquire(&AtomicBool::new(false)).unwrap();
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                live.fetch_sub(1, Ordering::SeqCst);
                drop(g);
            });
        }
    });
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "never exceed 2 concurrent holders"
    );
}

#[test]
fn acquire_aborts_while_queued() {
    let sem = Semaphore::new(1);
    let g = sem.acquire(&AtomicBool::new(false)).unwrap();
    let abort = Arc::new(AtomicBool::new(false));
    let abort2 = Arc::clone(&abort);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        abort2.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    assert!(matches!(sem.acquire(&abort), Err(Aborted)));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "must not block forever"
    );
    drop(g);
}

#[test]
fn resize_up_wakes_waiters_and_shrink_never_oversubscribes() {
    let sem = Arc::new(Semaphore::new(0));
    let acquired = Arc::new(AtomicUsize::new(0));
    let sem2 = Arc::clone(&sem);
    let acquired2 = Arc::clone(&acquired);
    let handle = std::thread::spawn(move || {
        let _g = sem2.acquire(&AtomicBool::new(false)).unwrap();
        acquired2.store(1, Ordering::SeqCst);
    });
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(acquired.load(Ordering::SeqCst), 0, "no permits yet");
    sem.resize(1);
    handle.join().unwrap();
    assert_eq!(
        acquired.load(Ordering::SeqCst),
        1,
        "resize up admits the waiter"
    );

    // Shrink while 2 held: available goes negative; a queued acquire
    // must wait until BOTH holders release, because the target is now
    // 1 permit and g2 still holds it.
    let sem = Arc::new(Semaphore::new(2));
    let g1 = sem.acquire(&AtomicBool::new(false)).unwrap();
    let g2 = sem.acquire(&AtomicBool::new(false)).unwrap();
    sem.resize(1);
    assert_eq!(sem.available(), -1);
    let queued = Arc::new(AtomicUsize::new(0));
    let queued2 = Arc::clone(&queued);
    let sem2 = Arc::clone(&sem);
    let h = std::thread::spawn(move || {
        let _g = sem2.acquire(&AtomicBool::new(false)).unwrap();
        queued2.store(1, Ordering::SeqCst);
    });
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        queued.load(Ordering::SeqCst),
        0,
        "shrunk semaphore stays drained"
    );
    drop(g1); // available 0 — still no free permit while g2 is held
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        queued.load(Ordering::SeqCst),
        0,
        "g2 still holds the only permit"
    );
    drop(g2); // now the queued waiter may proceed
    h.join().unwrap();
    assert_eq!(queued.load(Ordering::SeqCst), 1);
}

#[test]
fn map_limited_keeps_order_and_collects_all_settled() {
    let items: Vec<usize> = (0..5).collect();
    let abort = AtomicBool::new(false);
    let out = map_limited(&items, 2, &abort, |&i| {
        if i == 3 {
            Err("boom-3")
        } else {
            // Reverse sleep so completion order differs from index order.
            std::thread::sleep(Duration::from_millis(10 * (5 - i as u64)));
            Ok(i * 10)
        }
    });
    let expected: Vec<Result<usize, MapError<&str>>> = (0..5)
        .map(|i| {
            if i == 3 {
                Err(MapError::Item("boom-3"))
            } else {
                Ok(i * 10)
            }
        })
        .collect();
    assert_eq!(
        out, expected,
        "slots must be index-stable despite completion order"
    );
}

#[test]
fn map_limited_aborts_queued_items() {
    // limit=1, f sleeps 80ms per item; the abort flips as soon as a second
    // item has STARTED. Thread spawn order does not guarantee which items
    // run first, so the assertion is order-agnostic: the items that got a
    // permit before the flip succeed, everything queued after is skipped.
    let items: Vec<usize> = (0..4).collect();
    let abort = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicUsize::new(0));
    let abort2 = Arc::clone(&abort);
    let started2 = Arc::clone(&started);
    std::thread::spawn(move || {
        while started2.load(Ordering::SeqCst) < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        abort2.store(true, Ordering::SeqCst);
    });
    let out: Vec<Result<usize, MapError<()>>> = map_limited(&items, 1, &abort, |&i| {
        started.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(80));
        Ok(i)
    });
    let ran = started.load(Ordering::SeqCst);
    assert_eq!(ran, 2, "exactly two items started before the abort");
    assert_eq!(out.iter().filter(|r| r.is_ok()).count(), 2);
    assert_eq!(
        out.iter()
            .filter(|r| matches!(r, Err(MapError::Aborted)))
            .count(),
        2
    );
}
