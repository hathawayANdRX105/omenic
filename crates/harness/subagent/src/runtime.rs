use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::provider::{RunDisposer, SubagentProvider, SubagentRun, SubagentStartRequest};

/// Thread-safe registry of named subagent providers, plus the table of runs
/// started through [`Self::start_run`] that are still in flight.
///
/// Stored in the fiber under the key `"harness.subagents"`. Callers resolve
/// the service and query it by provider name.
#[derive(Default)]
pub struct SubagentRuntimeService {
    providers: Mutex<HashMap<String, Arc<dyn SubagentProvider>>>,
    /// Disposers of runs that have not yet settled, keyed by run id.
    runs: Mutex<HashMap<String, Arc<dyn RunDisposer>>>,
    run_seq: AtomicU64,
}

impl SubagentRuntimeService {
    /// Register `provider` under `name`. Overwrites an existing entry with the
    /// same name (composition controls load order).
    pub fn register(&self, name: impl Into<String>, provider: Arc<dyn SubagentProvider>) {
        self.providers.lock().unwrap().insert(name.into(), provider);
    }

    /// Look up a provider by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn SubagentProvider>> {
        self.providers.lock().unwrap().get(name).cloned()
    }

    /// All registered provider names, in insertion order.
    pub fn providers(&self) -> Vec<String> {
        self.providers.lock().unwrap().keys().cloned().collect()
    }

    /// Start a run under `provider_name`, remember its disposer so the run can
    /// be interrupted later, and return the fresh run id alongside the handle.
    pub fn start_run(
        &self,
        provider_name: &str,
        request: SubagentStartRequest,
    ) -> Result<(String, SubagentRun), String> {
        let provider = self
            .get(provider_name)
            .ok_or_else(|| format!("unknown subagent provider: {provider_name}"))?;
        let run = provider.start(request);
        let id = format!("sub-{}", self.run_seq.fetch_add(1, Ordering::Relaxed) + 1);
        self.runs.lock().unwrap().insert(id.clone(), run.disposer());
        Ok((id, run))
    }

    /// Dispose a run that is still in flight, if its id is live. Returns
    /// whether an active run was found and torn down.
    pub fn interrupt(&self, run_id: &str) -> bool {
        match self.runs.lock().unwrap().remove(run_id) {
            Some(disposer) => {
                disposer.dispose();
                true
            }
            None => false,
        }
    }

    /// Retire a run that has settled on its own — a finished run is no longer
    /// interruptable, so it leaves the table.
    pub fn finish_run(&self, run_id: &str) {
        self.runs.lock().unwrap().remove(run_id);
    }

    /// Ids of runs that are still in flight.
    pub fn active_runs(&self) -> Vec<String> {
        self.runs.lock().unwrap().keys().cloned().collect()
    }
}

/// Plugin that wires `SubagentRuntimeService` into the fiber.
///
/// Register this plugin in the composition root (Task C) so downstream
/// plugins and tools can resolve `"harness.subagents"` and register or
/// query subagent providers.
#[derive(Default)]
pub struct SubagentRuntime;

impl omenic_harness_plugin::DshPlugin for SubagentRuntime {
    fn name(&self) -> &str {
        "harness.subagents"
    }

    fn register(&self, ctx: &mut omenic_harness_plugin::PluginContext<'_>) {
        ctx.provide("harness.subagents", SubagentRuntimeService::default());
    }
}
