use omenic_harness_core::{AbortSignal, RunError, RunId, RunStatus};
use omenic_harness_runtime::{LoopEngine, Provider, run_agent_loop};
use omenic_harness_tools::ToolCatalog;
use std::sync::Arc;

#[test]
fn loop_engine_default_values() {
    let engine = LoopEngine::default();
    assert_eq!(
        engine.max_turns, 0,
        "default max_turns should be 0 until set"
    );
}

#[test]
fn run_agent_loop_smoke() {
    // This test only verifies that the function signature compiles
    // and that a stub provider + executor can be passed.
    struct StubProvider;
    impl Provider for StubProvider {
        fn call(
            &self,
            _model: &str,
            _messages: &[omenic_harness_core::Message],
            _tools: &[omenic_harness_core::ToolSpec],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            omenic_harness_core::Message,
                            omenic_harness_core::LlmError,
                        >,
                    > + Send,
            >,
        > {
            Box::pin(async { Err(omenic_harness_core::LlmError::Transport("stub".to_string())) })
        }
    }

    struct StubExecutor;
    impl omenic_harness_tools::ToolExecutor for StubExecutor {
        fn execute(
            &self,
            _spec: &omenic_harness_core::ToolSpec,
            _args: &serde_json::Value,
            _abort: &AbortSignal,
        ) -> Result<omenic_harness_core::ToolResult, omenic_harness_core::ToolError> {
            Ok(omenic_harness_core::ToolResult {
                output: "stub".to_string(),
                is_error: false,
            })
        }
    }

    let engine = LoopEngine::default();
    let provider = StubProvider;
    let executor = StubExecutor;
    let abort = AbortSignal::new();
    let result = run_agent_loop(&engine, &provider, &executor, &abort);
    // We expect the todo! to panic in test, but the type system is satisfied.
    assert!(result.is_err());
}
