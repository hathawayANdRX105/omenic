//! C6 plugin surface: registry typing, bus ordering, LIFO unload,
//! duplicate-name rejection.

use std::any::Any;
use std::sync::Arc;

use parking_lot::Mutex;

use plugin::{
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

/// Schema validation failure prevents `register` and plugin is not pushed.
#[test]
fn invalid_config_rejected_before_register() {
    struct StrictPlugin {
        registered: Arc<Mutex<Vec<&'static str>>>,
    }
    impl DshPlugin for StrictPlugin {
        fn name(&self) -> &str {
            "strict"
        }
        fn register(&self, _ctx: &mut PluginContext<'_>) {
            self.registered.lock().push("should-not-fire");
            panic!("register must not be called when validate_config fails");
        }
        fn validate_config(&self, _config: &serde_json::Value) -> Result<(), PluginError> {
            Err(PluginError::InvalidConfig(
                "missing required field 'threshold'".into(),
            ))
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut fiber = Fiber::default();
    let mut registry = PluginRegistry::default();

    let plugin = Arc::new(StrictPlugin {
        registered: Arc::clone(&log),
    });
    let err = registry
        .register(plugin, &mut fiber.context())
        .expect_err("schema validation failure must reject");
    assert!(
        matches!(&err, PluginError::InvalidConfig(msg) if msg == "missing required field 'threshold'"),
        "{err}"
    );
    assert!(log.lock().is_empty(), "register was not called");
    assert!(registry.is_empty(), "rejected plugin not pushed");
}

/// Default validate_config returns Ok, so registration proceeds normally.
#[test]
fn valid_config_allows_register() {
    struct LenientPlugin {
        registered: Arc<Mutex<Vec<&'static str>>>,
    }
    impl DshPlugin for LenientPlugin {
        fn name(&self) -> &str {
            "lenient"
        }
        fn register(&self, _ctx: &mut PluginContext<'_>) {
            self.registered.lock().push("ok");
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut fiber = Fiber::default();
    let mut registry = PluginRegistry::default();

    let plugin = Arc::new(LenientPlugin {
        registered: Arc::clone(&log),
    });
    assert!(registry.register(plugin, &mut fiber.context()).is_ok());
    assert_eq!(registry.plugins(), vec!["lenient"]);
    assert_eq!(*log.lock(), vec!["ok"], "register was called");
}

/// Unregister removes a single plugin definition by name; the remaining
/// plugins keep their order and are still fully functional.
#[test]
fn unregister_removes_single_plugin() {
    struct DualPlugin {
        name: &'static str,
        registered: Arc<Mutex<Vec<&'static str>>>,
    }

    impl DshPlugin for DualPlugin {
        fn name(&self) -> &str {
            self.name
        }
        fn register(&self, _ctx: &mut PluginContext<'_>) {
            self.registered.lock().push(self.name);
        }
    }

    impl PluginLifecycle for DualPlugin {
        fn name(&self) -> &str {
            self.name
        }
        fn on_unload(&mut self, _ctx: &mut PluginContext<'_>) {
            self.registered.lock().push(self.name);
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut fiber = Fiber::default();
    let mut registry = PluginRegistry::default();
    let ctx = &mut fiber.context();

    let a = Arc::new(DualPlugin {
        name: "a",
        registered: Arc::clone(&log),
    });
    let b = Arc::new(DualPlugin {
        name: "b",
        registered: Arc::clone(&log),
    });

    registry.register(a, ctx).unwrap();
    registry.register(b, ctx).unwrap();
    fiber.load(Box::new(DualPlugin {
        name: "a",
        registered: Arc::clone(&log),
    }));
    fiber.load(Box::new(DualPlugin {
        name: "b",
        registered: Arc::clone(&log),
    }));

    let removed = registry.unregister("a").unwrap();
    assert!(removed.is_some(), "registry must return the removed plugin");
    assert_eq!(registry.plugins(), vec!["b"], "b stays registered");

    let unloaded = fiber.unload_named("a").unwrap();
    assert!(unloaded.is_some(), "fiber must return the unloaded plugin");
    assert_eq!(fiber.len(), 1, "only b remains loaded");
    assert_eq!(
        *log.lock(),
        vec!["a", "b", "a"],
        "register a/b, then unload a's on_unload"
    );
}

/// Unregistering a name that was never registered is idempotent and leaves
/// the registry empty.
#[test]
fn unregister_nonexistent_returns_none() {
    let mut fiber = Fiber::default();
    let mut registry = PluginRegistry::default();

    assert!(registry.unregister("ghost").unwrap().is_none());
    assert!(fiber.unload_named("ghost").unwrap().is_none());
}

/// After a plugin is unloaded from both the registry and the fiber, the same
/// name can be registered again without hitting the duplicate-name guard.
#[test]
fn unload_then_register_same_name() {
    struct Once {
        registered: Arc<Mutex<Vec<&'static str>>>,
    }
    impl DshPlugin for Once {
        fn name(&self) -> &str {
            "once"
        }
        fn register(&self, _ctx: &mut PluginContext<'_>) {
            self.registered.lock().push("registered");
        }
    }
    impl PluginLifecycle for Once {
        fn name(&self) -> &str {
            "once"
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut fiber = Fiber::default();
    let mut registry = PluginRegistry::default();

    {
        let ctx = &mut fiber.context();
        registry
            .register(
                Arc::new(Once {
                    registered: Arc::clone(&log),
                }),
                ctx,
            )
            .unwrap();
        fiber.load(Box::new(Once {
            registered: Arc::clone(&log),
        }));
    }

    registry.unregister("once").unwrap();
    fiber.unload_named("once").unwrap();

    assert!(registry.is_empty());
    assert!(fiber.is_empty());

    // Re-register under the same name must succeed now.
    {
        let ctx = &mut fiber.context();
        registry
            .register(
                Arc::new(Once {
                    registered: Arc::clone(&log),
                }),
                ctx,
            )
            .unwrap();
    }
    assert_eq!(registry.plugins(), vec!["once"]);
}

/// Full LIFO unload is preserved after adding named unload.
#[test]
fn full_unload_is_still_reverse_order() {
    struct Order {
        tag: &'static str,
        order: Arc<Mutex<Vec<&'static str>>>,
    }
    impl PluginLifecycle for Order {
        fn name(&self) -> &str {
            self.tag
        }
        fn on_unload(&mut self, _ctx: &mut PluginContext<'_>) {
            self.order.lock().push(self.tag);
        }
    }

    let order = Arc::new(Mutex::new(Vec::new()));
    let mut fiber = Fiber::default();
    fiber.load(Box::new(Order {
        tag: "first",
        order: Arc::clone(&order),
    }));
    fiber.load(Box::new(Order {
        tag: "second",
        order: Arc::clone(&order),
    }));
    fiber.load(Box::new(Order {
        tag: "third",
        order: Arc::clone(&order),
    }));

    fiber.unload();

    assert_eq!(
        *order.lock(),
        vec!["third", "second", "first"],
        "full unload remains LIFO"
    );
}
