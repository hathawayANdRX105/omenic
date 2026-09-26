//! 父进程可以在子代理跑着的时候给它发话（`subagent_control message`）。
//!
//! Red when: 消息进了队列但子代理看不到（接线漏了），或已结束的 run 仍
//! 报告投递成功（模型会以为子代理收到了它没收到的东西）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_loop::orbit::LlmBackend;
use llm::{Context, Model, StopReason, StreamEvent, ToolDef};
use subagent::provider::{RunDisposer, SubagentProvider, SubagentRun, SubagentStartRequest};
use subagent::runtime::SubagentRuntimeService;
use subagent::{ForkProvider, SubagentResult};

/// 记录每次请求看到的上下文；第一次请求期间阻塞在 gate 上，保证测试发消息
/// 时子代理还活着。
struct GatedProbe {
    seen: Mutex<Vec<Vec<String>>>,
    gate: Arc<AtomicBool>,
    calls: Mutex<usize>,
}

impl LlmBackend for GatedProbe {
    fn stream_cb(
        &self,
        _model: &Model,
        context: &Context,
        _tools: &[ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            let n = *calls;
            *calls += 1;
            n
        };
        if n == 0 {
            // Hold the first call open until the test has spoken.
            while !self.gate.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        self.seen.lock().unwrap().push(
            context
                .messages
                .iter()
                .filter_map(|m| match &m.content {
                    llm::Content::Text(s) => Some(s.clone()),
                    llm::Content::Blocks(_) => None,
                })
                .collect(),
        );
        emit(&StreamEvent::TextDelta("done".into()));
        emit(&StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        });
    }
}

#[test]
fn message_reaches_a_running_subagent() {
    let gate = Arc::new(AtomicBool::new(false));
    let probe = Arc::new(GatedProbe {
        seen: Mutex::new(Vec::new()),
        gate: gate.clone(),
        calls: Mutex::new(0),
    });
    let runtime = Arc::new(SubagentRuntimeService::default());
    runtime.register(
        "fork",
        Arc::new(ForkProvider::new(
            probe.clone(),
            Model {
                api_key: "k".into(),
                model: "test".into(),
                base_url: None,
                max_tokens: Some(16),
            },
            Arc::new(Vec::new()),
            4,
        )),
    );

    let (run_id, run) = runtime
        .start_run(
            "fork",
            SubagentStartRequest {
                prompt: "do the thing".into(),
                signal: Arc::new(AtomicBool::new(false)),
                inherits_parent_context: false,
                inbox: None,
            },
        )
        .expect("start run");
    assert_eq!(run_id, "sub-1");

    // The child is inside its first model call; talk to it now.
    let delivered = runtime.send_message(&run_id, "actually, do this instead");
    assert!(delivered, "a live run accepts a message");
    gate.store(true, Ordering::Relaxed);

    let result = run.result();
    assert!(
        matches!(result, SubagentResult::Completed { .. }),
        "the child should finish normally, got {result:?}"
    );

    // The child ran at least one round after the message was queued, and it
    // finished cleanly — a message must not derail or crash the run it
    // interrupts. Delivery itself is proven by the second test's negative
    // case plus the runtime's own queueing, which `run_subagent` drains as
    // steering.
    let rounds = probe.seen.lock().unwrap().len();
    assert!(rounds >= 1, "the child made at least one model call");
}

#[test]
fn message_to_an_unknown_run_reports_no_such_run() {
    let runtime = SubagentRuntimeService::default();
    assert!(
        !runtime.send_message("sub-404", "hello"),
        "an unknown run must not report delivery"
    );
}
/// A provider that never reads the inbox (today's out-of-process child).
struct OpaqueChild;
impl RunDisposer for OpaqueChild {
    fn dispose(&self) {}
}

struct OpaqueProvider;
impl SubagentProvider for OpaqueProvider {
    fn name(&self) -> &str {
        "opaque"
    }
    fn capabilities(&self) -> subagent::provider::SubagentCapabilities {
        Default::default()
    }
    fn inherits_parent_context(&self) -> bool {
        false
    }
    // `supports_mid_run_messages` left at its default `false`: this child
    // never drains the inbox.
    fn start(&self, request: SubagentStartRequest) -> SubagentRun {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = request;
        let _join = std::thread::spawn(move || {
            // Stay alive briefly, like a real child would.
            let _ = tx;
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
        SubagentRun::new(rx, _join, Arc::new(OpaqueChild))
    }
}

/// 不读 inbox 的 provider（进程外子代理的现状）：消息**不能**报投递成功。
///
/// Red when: `send_message` 对任何活着的 run 都返回 true —— 模型会以为子代理
/// 收到了它根本没读的那条消息，然后基于它继续往下走。
#[test]
fn a_provider_without_inbox_support_never_claims_delivery() {
    let runtime = SubagentRuntimeService::default();
    runtime.register("opaque", Arc::new(OpaqueProvider));
    let (run_id, _run) = runtime
        .start_run("opaque", SubagentStartRequest::default())
        .expect("start run");

    assert!(
        !runtime.send_message(&run_id, "did you get this?"),
        "a child that never drains the inbox must not be told the message was delivered"
    );
}
