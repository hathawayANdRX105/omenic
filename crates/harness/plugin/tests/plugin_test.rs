//! C6 plugin surface: registry typing, bus ordering, LIFO unload,
//! duplicate-name rejection.

use std::any::Any;
use std::sync::Arc;

use parking_lot::Mutex;

use omenic_harness_plugin::{
    DshPlugin, EventHandler, Fiber, PluginContext, PluginError, PluginLifecycle, PluginRegistry,
    ServiceRegistry, Subscription,
};

#[derive(Debug)]
struct Tick(u32);

/// Wrong-type (or wrong-key) takes return `None`, never panic.
#[test]
fn registry_wrong_type_resolve_returns_none() {
    let mut reg = ServiceRegistry::default();
    reg.provide("counter", 7u32);
    reg.provide("label", String::from("harness"));

    assert_eq!(reg.resolve::<u32>("counter").map(|c| *c), Some(7));
    assert!(
        reg.resolve::<String>("counter").is_none(),
        "no String under counter"
    );
    assert!(
        reg.resolve::<u64>("counter").is_none(),
        "no u64 under counter"
    );
    assert!(
        reg.resolve::<u32>("label").is_none(),
        "wrong key must miss, not steal the u32"
    );
    assert!(reg.resolve::<Tick>("counter").is_none());
    assert_eq!(
        reg.resolve::<String>("label")
            .map(|s| s.as_str().to_owned()),
        Some("harness".into())
    );
}

/// `emit` dispatches synchronously, to every handler, in subscription order.
#[test]
fn emit_dispatches_in_subscription_order() {
    let mut fiber = Fiber::default();
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let make = |tag: &'static str, log: &Arc<Mutex<Vec<String>>>| -> EventHandler {
        let log = Arc::clone(log);
        Box::new(move |ev: &dyn Any| {
            if let Some(t) = ev.downcast_ref::<Tick>() {
                log.lock().push(format!("{tag}:{}", t.0));
            }
        })
    };
    {
        let ctx = &mut fiber.context();
        ctx.subscribe("tick", make("a", &log));
        ctx.subscribe("tick", make("b", &log));
        ctx.subscribe("other", make("should-not-fire", &log));
    }

    fiber.context().emit("tick", &Tick(1));
    fiber.context().emit("tick", &Tick(2));

    assert_eq!(
        log.lock().clone(),
        vec!["a:1", "b:1", "a:2", "b:2"],
        "handlers run synchronously in subscription order, per emit"
    );
}

/// Unloading tears plugins down in reverse load order, and their handlers
/// stop firing on the still-alive bus; `Drop` performs the same teardown.
#[test]
fn handlers_dead_after_unload_and_teardown_is_reverse() {
    struct Watcher {
        tag: &'static str,
        sub: Option<Subscription>,
        hits: Arc<Mutex<Vec<&'static str>>>,
        order: Arc<Mutex<Vec<&'static str>>>,
    }
    impl PluginLifecycle for Watcher {
        fn on_load(&mut self, ctx: &mut PluginContext<'_>) {
            let hits = Arc::clone(&self.hits);
            let tag = self.tag;
            self.sub = Some(ctx.subscribe(
                "work",
                Box::new(move |ev: &dyn Any| {
                    if ev.downcast_ref::<Tick>().is_some() {
                        hits.lock().push(tag);
                    }
                }),
            ));
        }
        fn on_unload(&mut self, ctx: &mut PluginContext<'_>) {
            self.order.lock().push(self.tag);
            if let Some(sub) = self.sub.take() {
                ctx.unsubscribe(sub);
            }
        }
    }

    let hits = Arc::new(Mutex::new(Vec::new()));
    let order = Arc::new(Mutex::new(Vec::new()));
    let make = |tag: &'static str| {
        Box::new(Watcher {
            tag,
            sub: None,
            hits: Arc::clone(&hits),
            order: Arc::clone(&order),
        }) as Box<dyn PluginLifecycle>
    };

    let mut fiber = Fiber::default();
    assert!(fiber.is_empty());
    fiber.load(make("first"));
    fiber.load(make("second"));
    assert_eq!(fiber.len(), 2);
    fiber.context().emit("work", &Tick(1));
    assert_eq!(*hits.lock(), vec!["first", "second"]);

    fiber.unload();

    assert_eq!(
        *order.lock(),
        vec!["second", "first"],
        "teardown runs in reverse registration order"
    );
    assert!(fiber.is_empty());
    fiber.context().emit("work", &Tick(2));
    assert_eq!(
        *hits.lock(),
        vec!["first", "second"],
        "no handler survives unload on the still-alive bus"
    );

    let mut fiber = Fiber::default();
    fiber.load(make("solo"));
    drop(fiber);
    assert_eq!(
        *order.lock(),
        vec!["second", "first", "solo"],
        "Drop unloads exactly like explicit teardown"
    );
}

/// Duplicate plugin names are rejected before `register` runs.
#[test]
fn duplicate_plugin_name_rejected() {
    struct Dummy {
        name: &'static str,
        registered: Arc<Mutex<Vec<&'static str>>>,
    }
    impl DshPlugin for Dummy {
        fn name(&self) -> &str {
            self.name
        }
        fn register(&self, _ctx: &mut PluginContext<'_>) {
            self.registered.lock().push(self.name);
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut fiber = Fiber::default();
    let mut registry = PluginRegistry::default();

    let first = Arc::new(Dummy {
        name: "memory",
        registered: Arc::clone(&log),
    });
    let twin = Arc::new(Dummy {
        name: "memory",
        registered: Arc::clone(&log),
    });

    assert!(registry.register(first, &mut fiber.context()).is_ok());
    let err = registry
        .register(twin, &mut fiber.context())
        .expect_err("duplicate name must be refused");
    assert!(
        matches!(&err, PluginError::Duplicate(n) if n.as_str() == "memory"),
        "{err}"
    );
    assert_eq!(registry.plugins(), vec!["memory"]);
    assert_eq!(
        *log.lock(),
        vec!["memory"],
        "the rejected twin never touched the context"
    );
}
