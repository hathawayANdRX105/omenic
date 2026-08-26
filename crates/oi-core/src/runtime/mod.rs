//! Agent runtime kernel: LLM stream (`llm`), loop with the four
//! invariants (`agent`), builtin tools (`tools`). The transport,
//! process, and orchestration around this kernel live at the crate root
//! (`omp_rpc`, `agent_process`, `run_flow`).

pub mod agent;
pub mod llm;
pub mod tools;
