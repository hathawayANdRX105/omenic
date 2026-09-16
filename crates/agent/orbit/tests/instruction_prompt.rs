//! WP-A: AGENTS.md instruction injection into orbit's system prompt.
//!
//! The loop defaults `Context.system_prompt` to `prompts::agents::TASK`
//! prefixed with any `AGENTS.md` fragments discovered up the ancestor chain
//! of `LoopConfig::instruction_cwd`. These tests drive real runs through a
//! capturing backend so the prompt actually handed to the model is asserted,
//! and every tempdir carries a `.git` marker so the upward walk stays inside
//! the tempdir and never sees the real repo's `AGENTS.md`.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use adaptor::{Context, Message, Model, StopReason, StreamEvent, ToolDef};
use orbit::{LlmBackend, LoopConfig, build_system_prompt, run_agent_streaming};
use prompts::agents::TASK;

/// Unique content marker so assertions pin the injected text, not just any
/// "Instructions from:" heading that happens to appear.
const MARKER: &str = "WP_A_INJECTION_MARKER_8675309";

fn model() -> Model {
    Model {
        api_key: "k".into(),
        model: "test".into(),
        base_url: None,
        max_tokens: None,
    }
}

/// A tempdir workspace bounded by a `.git` root marker (a worktree's `.git`
/// is a file, and `exists()` covers both, so a directory works too) — the
/// ancestor chain never escapes into the real filesystem.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    dir
}

/// Backend that captures every system prompt it was handed and ends the turn
/// immediately: one round-trip per run is all these tests need.
struct Capture {
    prompts: RefCell<Vec<String>>,
}

impl LlmBackend for Capture {
    fn stream_cb(
        &self,
        _model: &Model,
        context: &Context,
        _tools: &[ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        self.prompts
            .borrow_mut()
            .push(context.system_prompt.clone().unwrap_or_default());
        emit(&StreamEvent::TextDelta("hi".into()));
        emit(&StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        });
    }
}

/// Run one turn with `instruction_cwd` set and return the prompt the backend
/// actually received.
fn prompt_sent_with(cwd: &Path) -> String {
    let backend = Capture {
        prompts: RefCell::new(vec![]),
    };
    let mut ctx = Context {
        system_prompt: None,
        messages: vec![Message::user_text("hi")],
    };
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &[],
        &AtomicBool::new(false),
        LoopConfig {
            instruction_cwd: Some(cwd),
            ..LoopConfig::default()
        },
        &mut |_| {},
    );
    let prompts = backend.prompts.borrow();
    assert_eq!(prompts.len(), 1, "exactly one LLM round-trip");
    prompts[0].clone()
}

/// Test 1: a run started under a workspace with an `AGENTS.md` sends the
/// rendered fragment before the main-agent profile.
#[test]
fn agents_md_fragment_is_prepended_to_the_system_prompt() {
    let root = workspace();
    let agents_md = root.path().join("AGENTS.md");
    fs::write(&agents_md, format!("{MARKER}\nbe excellent to each other")).unwrap();

    let prompt = prompt_sent_with(root.path());

    // The instruction section leads, the role profile follows.
    let heading = format!("Instructions from: {}", agents_md.display());
    assert!(
        prompt.starts_with(heading.as_str()),
        "prompt must lead with the rendered AGENTS.md section, got: {:?}",
        prompt.chars().take(160).collect::<String>()
    );
    assert!(
        prompt.contains(MARKER),
        "injected content must reach the model"
    );
    // The TASK profile survives the prefix intact, after the fragment.
    let marker_at = prompt.find(MARKER).unwrap();
    assert!(prompt[marker_at..].contains("Worker agent:"));
    assert!(prompt.contains("<directives>"));
    // An explicit caller prompt is never overwritten.
    let backend = Capture {
        prompts: RefCell::new(vec![]),
    };
    let mut ctx = Context {
        system_prompt: Some("caller wins".into()),
        messages: vec![Message::user_text("hi")],
    };
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &[],
        &AtomicBool::new(false),
        LoopConfig {
            instruction_cwd: Some(root.path()),
            ..LoopConfig::default()
        },
        &mut |_| {},
    );
    assert_eq!(backend.prompts.borrow()[0], "caller wins");
}

/// Test 2: no `AGENTS.md` anywhere on the chain — the prompt is exactly the
/// bare TASK profile, byte for byte: behavior unchanged from pre-WP-A.
#[test]
fn no_agents_md_keeps_the_pure_task_prompt() {
    let root = workspace();
    let nested = root.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();

    assert_eq!(prompt_sent_with(&nested), TASK);
    // The opt-out knob holds too: no cwd handed over, no discovery.
    assert_eq!(build_system_prompt(None), TASK);
}

/// Test 3: two `AGENTS.md` files with identical (whitespace-trimmed) content
/// collapse to one section — the digest dedup keeps the prompt free of
/// repeated instructions.
#[test]
fn identical_agents_md_files_digest_dedup() {
    let root = workspace();
    fs::write(root.path().join("AGENTS.md"), format!("{MARKER}\n")).unwrap();
    let nested = root.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();
    // Same content modulo surrounding whitespace → same trimmed digest.
    fs::write(nested.join("AGENTS.md"), format!("  {MARKER}  \n\n")).unwrap();

    let prompt = prompt_sent_with(&nested);

    assert_eq!(
        prompt.matches("Instructions from:").count(),
        1,
        "identical sections must not be injected twice"
    );
    assert_eq!(prompt.matches(MARKER).count(), 1);
    assert!(prompt.ends_with(TASK), "profile must round out the prompt");
    // Genuinely different content still renders as its own section.
    fs::write(nested.join("AGENTS.md"), format!("{MARKER} but different")).unwrap();
    let prompt = prompt_sent_with(&nested);
    assert_eq!(prompt.matches("Instructions from:").count(), 2);
    assert_eq!(prompt.matches(MARKER).count(), 2);
}

/// Test 4: failure modes degrade silently to the TASK profile — discovery and
/// loading treat a missing/unreadable/non-file candidate as a no-op, and the
/// run never panics on a workspace without instructions.
#[test]
fn instruction_failures_degrade_silently_to_task() {
    // A directory named AGENTS.md is not a candidate (`is_file()` gate).
    let root = workspace();
    fs::create_dir(root.path().join("AGENTS.md")).unwrap();
    assert_eq!(prompt_sent_with(root.path()), TASK);

    // A cwd that does not exist: the walk stays lexical and bounded by the
    // workspace's `.git` marker, finds nothing, returns TASK.
    let root = workspace();
    assert_eq!(
        build_system_prompt(Some(root.path().join("gone/deeper").as_path())),
        TASK
    );

    // An AGENTS.md alongside the nested run dir still resolves relative to
    // the host-supplied cwd, not the process cwd.
    let root = workspace();
    fs::write(root.path().join("AGENTS.md"), MARKER.to_string()).unwrap();
    let nested = root.path().join("sub");
    fs::create_dir(&nested).unwrap();
    assert!(prompt_sent_with(&nested).contains(MARKER));
}
