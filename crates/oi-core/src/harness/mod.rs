//! Agent harness core: unified LLM stream, agent loop with four
//! invariants, builtin tools, context JSONL persistence.
//!
//! Rust port of pi-from-scratch (llm.ts / agent.ts / tools.ts).

pub mod agent;
pub mod llm;
pub mod tools;
