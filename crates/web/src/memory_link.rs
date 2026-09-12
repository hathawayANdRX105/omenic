//! Wiring between the agent runs (orbit) and the memory store: injection
//! drainage at fresh user turns, and transcript extraction at trigger
//! boundaries. All thresholds live in [`memory::pipeline`]; this module owns
//! only the session-keyed state and the glue.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use memory::{ExtractionTriggers, MemoryEntry};

fn buffers() -> &'static Mutex<HashMap<String, memory::InjectionBuffer>> {
    static BUFFERS: OnceLock<Mutex<HashMap<String, memory::InjectionBuffer>>> = OnceLock::new();
    BUFFERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn recorders() -> &'static Mutex<HashMap<String, ExtractionTriggers>> {
    static RECORDERS: OnceLock<Mutex<HashMap<String, ExtractionTriggers>>> = OnceLock::new();
    RECORDERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether the memory switch is on (same env contract as
/// [`tools::memory_tool::memory_store`]).
pub fn memory_enabled() -> bool {
    tools::memory_tool::memory_store().enabled()
}

/// Queue a payload for injection into `sid`'s next prompt.
pub fn set_injection(sid: &str, memories: Vec<MemoryEntry>, now: u64) {
    let mut map = buffers().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(sid.to_string())
        .or_default()
        .set_memories(memories, now);
}

/// Drain `sid`'s ready injection into the context as a trailing user
/// message (jcode's system-reminder seam: appended at the end to preserve
/// the KV-cache prefix, consumed only when a fresh user turn begins).
/// Returns whether anything was injected.
pub fn drain_injection(sid: &str, context: &mut adaptor::Context, now: u64) -> bool {
    let payload = {
        let mut map = buffers().lock().unwrap_or_else(|e| e.into_inner());
        map.get_mut(sid).and_then(|b| b.take_ready(now))
    };
    match payload {
        Some(text) if !text.is_empty() => {
            context.messages.push(adaptor::Message::user_text(text));
            true
        }
        _ => false,
    }
}

/// Embedder from the environment: `OMENIC_EMBED_URL` + `OMENIC_EMBED_KEY` +
/// `OMENIC_EMBED_MODEL`. Absent → `None`, and extraction results fall back
/// to plain appends (no cosine dedup).
pub fn embedder_from_env() -> Option<impl memory::Embedder> {
    let url = std::env::var("OMENIC_EMBED_URL").ok()?;
    let key = std::env::var("OMENIC_EMBED_KEY").ok()?;
    let model = std::env::var("OMENIC_EMBED_MODEL").ok()?;
    Some(memory::OpenAIEmbeddings::new(url, key, model))
}

/// Extract-and-remember one finished run. Returns the number of memories
/// stored (0 when the trigger did not fire, the transcript was empty, or
/// the extractor returned nothing).
///
/// - `turns_used` / `session_ending` drive the trigger state machine; the
///   per-session state persists here, so the caller only counts turns.
/// - With an `embedder`, lines go through `Memory::remember` (cosine dedup);
///   without one they fall back to plain appends (documented degradation,
///   not a skipped write).
pub fn extract_and_remember(
    backend: &dyn orbit::LlmBackend,
    model: &adaptor::Model,
    sid: &str,
    turns_used: u32,
    session_ending: bool,
    transcript: &[adaptor::Message],
    now: u64,
) -> usize {
    if !memory_enabled() {
        return 0;
    }
    let embedder = embedder_from_env();
    let should = {
        let mut map = recorders().lock().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(sid) {
            // Known session: the state machine decides.
            Some(triggers) => {
                let fire = triggers.should_extract(turns_used, None, session_ending);
                if fire {
                    triggers.mark_extracted(turns_used, None);
                }
                fire
            }
            // First sight of this session: the caller runs the startup
            // extraction itself (the documented contract — a fresh trigger
            // state never fires on its own, it must be seeded).
            None => {
                let mut seeded = ExtractionTriggers::new();
                seeded.mark_extracted(turns_used, None);
                map.insert(sid.to_string(), seeded);
                true
            }
        }
    };
    if !should {
        return 0;
    }

    // Bounded extraction window: the newest messages, at most ~8000 chars
    // (jcode's window), fed as one user turn under the extraction prompt.
    let mut window: Vec<&adaptor::Message> = transcript.iter().rev().take(12).collect();
    window.reverse();
    let mut chars: usize = window.iter().map(|m| message_len(m)).sum();
    while chars > 8000 && window.len() > 1 {
        chars -= message_len(&window[0]);
        window.remove(0);
    }
    let conversation = window
        .iter()
        .map(|m| format!("{:?}: {}", m.role, block_text(m)))
        .collect::<Vec<_>>()
        .join("\n");
    if conversation.trim().is_empty() {
        return 0;
    }

    let context = adaptor::Context {
        system_prompt: Some(memory::pipeline::EXTRACTION_PROMPT.to_string()),
        messages: vec![adaptor::Message::user_text(conversation)],
    };
    let mut raw = String::new();
    for ev in backend.stream(
        model,
        &context,
        &[],
        &std::sync::atomic::AtomicBool::new(false),
    ) {
        match ev {
            adaptor::StreamEvent::TextDelta(delta) => raw.push_str(&delta),
            adaptor::StreamEvent::Done { .. } => break,
            adaptor::StreamEvent::ToolCall(_) | adaptor::StreamEvent::Error(_) => {}
        }
    }

    let lines = memory::pipeline::extract_lines(&raw);
    if lines.is_empty() {
        return 0;
    }
    let mut mem = tools::memory_tool::memory_store();
    if !mem.enabled() {
        return 0;
    }
    let mut stored = 0usize;
    for line in lines {
        let result = match &embedder {
            Some(embedder) => mem
                .remember(line.content.clone(), line.category, Vec::new(), embedder)
                .map(|_| ()),
            None => {
                let mut entry = MemoryEntry::new(line.content.clone());
                entry.category = line.category;
                entry.trust = line.trust;
                mem.append(entry).map(|_| ())
            }
        };
        if result.is_ok() {
            stored += 1;
        }
    }
    stored
}

fn message_len(m: &adaptor::Message) -> usize {
    match &m.content {
        adaptor::Content::Text(s) => s.len(),
        adaptor::Content::Blocks(_) => 256,
    }
}

fn block_text(m: &adaptor::Message) -> String {
    match &m.content {
        adaptor::Content::Text(s) => s.clone(),
        adaptor::Content::Blocks(bs) => bs
            .iter()
            .filter_map(|b| match b {
                adaptor::Block::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}
