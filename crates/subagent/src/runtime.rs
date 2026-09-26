use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::provider::{
    RunDisposer, SubagentProvider, SubagentResult, SubagentRun, SubagentStartRequest,
};

/// One run that has not settled yet: how to tear it down, and where its
/// mid-run messages go.
struct LiveRun {
    disposer: Arc<dyn RunDisposer>,
    inbox: Arc<Mutex<VecDeque<String>>>,
    /// Whether this run's provider actually reads the inbox. A run whose
    /// provider does not must not be reported as delivered.
    steerable: bool,
}

/// Thread-safe registry of named subagent providers, plus the table of runs
/// started through [`Self::start_run`] that are still in flight.
///
/// Stored in the fiber under the key `"harness.subagents"`. Callers resolve
/// the service and query it by provider name.
#[derive(Default)]
pub struct SubagentRuntimeService {
    providers: Mutex<HashMap<String, Arc<dyn SubagentProvider>>>,
    /// Live runs keyed by run id: the teardown handle plus the inbox a
    /// `subagent_control message` pushes into.
    runs: Mutex<HashMap<String, LiveRun>>,
    /// Settled background runs, bounded so a long-lived daemon does not grow
    /// this table without limit. Oldest first; [`Self::result`] looks up here.
    settled: Mutex<VecDeque<(String, SubagentResult)>>,
    /// Fired once when a background run settles. The daemon wires it to push
    /// a completion notice into the model's aside channel; a sync run that
    /// returned its result inline does not fire it.
    on_settled: Mutex<Option<Arc<dyn Fn(&str, &SubagentResult) + Send + Sync>>>,
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
        // The inbox is created here so the caller can push into it while the
        // child is still running, and handed to the provider through the
        // request so the loop drains it as steering.
        let inbox: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let mut request = request;
        request.inbox = Some(Arc::clone(&inbox));
        let run = provider.start(request);
        let id = format!("sub-{}", self.run_seq.fetch_add(1, Ordering::Relaxed) + 1);
        let steerable = provider.supports_mid_run_messages();
        self.runs.lock().unwrap().insert(
            id.clone(),
            LiveRun {
                disposer: run.disposer(),
                inbox,
                steerable,
            },
        );
        Ok((id, run))
    }

    /// Dispose a run that is still in flight, if its id is live. Returns
    /// whether an active run was found and torn down.
    pub fn interrupt(&self, run_id: &str) -> bool {
        match self.runs.lock().unwrap().remove(run_id) {
            Some(run) => {
                run.disposer.dispose();
                true
            }
            None => false,
        }
    }

    /// Deliver `text` to a running child. The child sees it at its next
    /// step boundary, as if the parent had spoken mid-turn. Returns whether
    /// the message was queued.
    ///
    /// `false` covers two cases the caller must not confuse: no such run, and
    /// a run whose provider never drains an inbox (an out-of-process child).
    /// Both are "not delivered"; the caller that needs to tell them apart
    /// checks [`Self::active_runs`] first.
    pub fn send_message(&self, run_id: &str, text: &str) -> bool {
        let runs = self.runs.lock().unwrap();
        match runs.get(run_id).filter(|run| run.steerable) {
            Some(run) => {
                run.inbox
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_back(text.to_string());
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

    /// Register the hook fired when a background run settles. The daemon uses
    /// it to push a completion notice into the model's aside channel.
    pub fn set_on_settled(&self, hook: Arc<dyn Fn(&str, &SubagentResult) + Send + Sync>) {
        *self.on_settled.lock().unwrap() = Some(hook);
    }

    /// Retire a settled background run: drop it from the live table, keep its
    /// result for [`Self::result`], and fire the completion hook.
    pub fn settle(&self, run_id: &str, result: &SubagentResult) {
        self.runs.lock().unwrap().remove(run_id);
        let mut settled = self.settled.lock().unwrap();
        settled.push_back((run_id.to_string(), result.clone()));
        if settled.len() > 64 {
            settled.pop_front();
        }
        drop(settled);
        if let Some(hook) = self.on_settled.lock().unwrap().as_ref() {
            hook(run_id, result);
        }
    }

    /// The result of a settled background run, or `None` if it has not
    /// settled (or was evicted past the 64-run bound).
    pub fn result(&self, run_id: &str) -> Option<SubagentResult> {
        self.settled
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(id, _)| id == run_id)
            .map(|(_, r)| r.clone())
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
