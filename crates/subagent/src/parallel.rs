//! Concurrency primitives for subagent fan-out (port of omp `task/parallel.ts`).
//!
//! [`Semaphore`] tracks permits, not holders: `resize` adjusts the target so
//! in-flight permits are never lost (omp issue #3464), and `acquire` is
//! abortable — a queued waiter gives up as soon as the abort signal fires.
//! [`map_limited`] runs a fallible fn over items with bounded concurrency and
//! keeps results index-stable (all-settled: no fail-fast).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Counting semaphore with abortable acquire and hot resize.
pub struct Semaphore {
    state: Mutex<SemState>,
    cv: Condvar,
}

struct SemState {
    /// Target permit count (`permits` knob).
    permits: usize,
    /// `available = permits - held`; may go negative after a shrink while
    /// holders release, converging back to `permits` as they do.
    available: isize,
    /// FIFO of waiter tickets so wakeups admit the queue head first.
    queue: VecDeque<u64>,
    next_ticket: u64,
}

impl Semaphore {
    pub fn new(permits: usize) -> Self {
        Semaphore {
            state: Mutex::new(SemState {
                permits,
                available: permits as isize,
                queue: VecDeque::new(),
                next_ticket: 0,
            }),
            cv: Condvar::new(),
        }
    }

    /// Change the permit count. Shrink only affects future acquires —
    /// current holders finish and hand their permits back; grow wakes
    /// queued waiters immediately.
    pub fn resize(&self, permits: usize) {
        let mut st = self.lock_state();
        let delta = permits as isize - st.permits as isize;
        st.permits = permits;
        st.available += delta;
        if delta > 0 {
            self.cv.notify_all();
        }
    }

    /// Acquire one permit, or bail with [`Aborted`] once `abort` flips.
    /// Queued waiters are served FIFO.
    ///
    /// The abort flag's writer must store with `SeqCst` (or `Release`) so a
    /// flip is observable to this `SeqCst` load; `Relaxed` writers would
    /// leave the waiter unsynchronized on weak memory models.
    pub fn acquire(&self, abort: &AtomicBool) -> Result<SemaphoreGuard<'_>, Aborted> {
        let mut ticket = None;
        let mut st = self.lock_state();
        loop {
            if abort.load(Ordering::SeqCst) {
                if let Some(t) = ticket {
                    st.queue.retain(|&q| q != t);
                }
                return Err(Aborted);
            }
            // Only the queue head may proceed; everyone else keeps waiting.
            let is_head = match (st.queue.front(), ticket) {
                (None, _) => true,
                (Some(h), Some(t)) => *h == t,
                (Some(_), None) => false,
            };
            if is_head && st.available > 0 {
                st.available -= 1;
                if ticket.is_some() {
                    st.queue.pop_front();
                    // The next waiter is the new head; it must wake up and
                    // re-evaluate, or it sleeps until an unrelated event.
                    self.cv.notify_all();
                }
                return Ok(SemaphoreGuard { sem: self });
            }
            if ticket.is_none() {
                let t = st.next_ticket;
                st.next_ticket += 1;
                st.queue.push_back(t);
                ticket = Some(t);
            }
            // The abort signal is a plain AtomicBool flipped from another
            // thread — it cannot wake this condvar. Poll it on a short
            // wait_timeout so abort latency stays bounded.
            let (next, _) = self
                .cv
                .wait_timeout(st, Duration::from_millis(25))
                .unwrap_or_else(|e| e.into_inner());
            st = next;
        }
    }

    /// Lock that survives a panicking holder: a poisoned mutex still holds
    /// consistent-enough state here (plain counters), and crashing every
    /// subsequent waiter would turn one worker panic into a harness crash.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, SemState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Permits currently available (diagnostics/tests).
    pub fn available(&self) -> isize {
        self.lock_state().available
    }

    fn release(&self) {
        let mut st = self.lock_state();
        st.available += 1;
        self.cv.notify_all();
    }
}

/// Returned by [`Semaphore::acquire`] when the abort signal fired first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aborted;

/// Held permit; dropping it hands the permit back.
pub struct SemaphoreGuard<'a> {
    sem: &'a Semaphore,
}

impl Drop for SemaphoreGuard<'_> {
    fn drop(&mut self) {
        self.sem.release();
    }
}

/// Why one item in a [`map_limited`] run has no successful result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError<E> {
    /// `f` itself failed; carries its error.
    Item(E),
    /// The abort signal fired before the item got a permit.
    Aborted,
}

/// Run `f` over `items` with at most `limit` concurrent executions,
/// collecting `Result`s index-stably. No fail-fast: every item runs (or is
/// skipped as [`MapError::Aborted`] if the abort signal fires while queued).
pub fn map_limited<T, R, E, F>(
    items: &[T],
    limit: usize,
    abort: &AtomicBool,
    f: F,
) -> Vec<Result<R, MapError<E>>>
where
    T: Sync,
    R: Send,
    E: Send,
    F: Fn(&T) -> Result<R, E> + Sync,
{
    let sem = Semaphore::new(limit);
    let results: Vec<Mutex<Option<Result<R, MapError<E>>>>> =
        (0..items.len()).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        let f = &f;
        for (idx, item) in items.iter().enumerate() {
            let sem = &sem;
            let results = &results;
            s.spawn(move || {
                let slot = match sem.acquire(abort) {
                    Ok(guard) => guard,
                    Err(Aborted) => {
                        *results[idx].lock().unwrap() = Some(Err(MapError::Aborted));
                        return;
                    }
                };
                let out = f(item).map_err(MapError::Item);
                *results[idx].lock().unwrap() = Some(out);
                drop(slot);
            });
        }
        // Scope exit joins every worker; all slots are filled by then.
    });
    results
        .into_iter()
        .map(|m| {
            m.into_inner()
                .unwrap()
                .expect("every worker writes its slot")
        })
        .collect()
}
