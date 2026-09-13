//! Synchronous topic event bus.
//!
//! Reference: `dsh vendor/cordis/src/events.ts` (`EventsService.emit/on`):
//! same-tick sequential dispatch to handlers in subscription order; no
//! async queue, no wildcard topics.
//!
//! Payloads are `&dyn Any`: subscribers downcast to the event type they
//! expect. Non-matching payloads are skipped (a mis-published event never
//! panics a handler).

use std::any::Any;
use std::collections::HashMap;

/// Handler body: receives the erased event payload.
pub type EventHandler = Box<dyn Fn(&dyn Any) + Send + Sync>;

/// Opaque token identifying one subscription; return it to unsubscribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Subscription(usize);

/// Topic-keyed handler map with synchronous sequential dispatch.
///
/// Constraint: `emit` is re-entrant safe only for reads — handlers run
/// before/after each other in subscription order, never concurrently,
/// and cannot mutate the bus mid-dispatch (Rust borrow rules enforce this).
/// Non-goal: no priority ordering, no once/repeat flags, no scoped filters
/// (cordis `Context[filter]`); add when a real host needs them.
#[derive(Default)]
pub struct EventBus {
    topics: HashMap<String, Vec<(Subscription, EventHandler)>>,
    next: usize,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach `handler` to `topic`, returning its unsubscribe token.
    pub fn subscribe(&mut self, topic: impl Into<String>, handler: EventHandler) -> Subscription {
        self.next += 1;
        let sub = Subscription(self.next);
        self.topics
            .entry(topic.into())
            .or_default()
            .push((sub, handler));
        sub
    }

    /// Detach one handler. After this returns, `emit` never calls it again.
    pub fn unsubscribe(&mut self, sub: Subscription) {
        for handlers in self.topics.values_mut() {
            handlers.retain(|(s, _)| *s != sub);
        }
    }

    /// Dispatch `event` to every `topic` handler, subscription order,
    /// synchronously on the calling thread.
    pub fn emit(&self, topic: &str, event: &dyn Any) {
        if let Some(handlers) = self.topics.get(topic) {
            for (_, handler) in handlers {
                handler(event);
            }
        }
    }

    /// Number of live handlers on `topic` (diagnostics/tests).
    pub fn listener_count(&self, topic: &str) -> usize {
        self.topics.get(topic).map_or(0, Vec::len)
    }
}
