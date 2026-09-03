//! System prompt fragments, copied verbatim from
//! `oh-my-pi`'s `packages/coding-agent/src/prompts/system/`.
//!
//! Each constant is the entire contents of the corresponding `.md`
//! file. Fragments are injected by the calling agent loop in a
//! specific order; omenic currently has no per-fragment conditional
//! gating (no `prep` step like omp `system-prompt.ts:666`), so the
//! exposed constants are raw `&str` references for callers to
//! compose at the role layer. See `agents.rs` for the role-level
//! prompts; this module is the fragment library.

/// System prompt fragment: `active-repo-context.md` (verbatim from omp).
pub const ACTIVE_REPO_CONTEXT: &str = include_str!("../prompts/system/active-repo-context.md");

/// System prompt fragment: `agent-creation-architect.md` (verbatim from omp).
pub const AGENT_CREATION_ARCHITECT: &str =
    include_str!("../prompts/system/agent-creation-architect.md");

/// System prompt fragment: `agent-creation-user.md` (verbatim from omp).
pub const AGENT_CREATION_USER: &str = include_str!("../prompts/system/agent-creation-user.md");

/// System prompt fragment: `auto-continue.md` (verbatim from omp).
pub const AUTO_CONTINUE: &str = include_str!("../prompts/system/auto-continue.md");

/// System prompt fragment: `auto-thinking-difficulty-local.md` (verbatim from omp).
pub const AUTO_THINKING_DIFFICULTY_LOCAL: &str =
    include_str!("../prompts/system/auto-thinking-difficulty-local.md");

/// System prompt fragment: `auto-thinking-difficulty.md` (verbatim from omp).
pub const AUTO_THINKING_DIFFICULTY: &str =
    include_str!("../prompts/system/auto-thinking-difficulty.md");

/// System prompt fragment: `autolearn-guidance-learn.md` (verbatim from omp).
pub const AUTOLEARN_GUIDANCE_LEARN: &str =
    include_str!("../prompts/system/autolearn-guidance-learn.md");

/// System prompt fragment: `autolearn-guidance.md` (verbatim from omp).
pub const AUTOLEARN_GUIDANCE: &str = include_str!("../prompts/system/autolearn-guidance.md");

/// System prompt fragment: `autolearn-nudge-autocontinue.md` (verbatim from omp).
pub const AUTOLEARN_NUDGE_AUTOCONTINUE: &str =
    include_str!("../prompts/system/autolearn-nudge-autocontinue.md");

/// System prompt fragment: `background-tan-dispatch.md` (verbatim from omp).
pub const BACKGROUND_TAN_DISPATCH: &str =
    include_str!("../prompts/system/background-tan-dispatch.md");

/// System prompt fragment: `btw-user.md` (verbatim from omp).
pub const BTW_USER: &str = include_str!("../prompts/system/btw-user.md");

/// System prompt fragment: `checkpoint-active-notice.md` (verbatim from omp).
pub const CHECKPOINT_ACTIVE_NOTICE: &str =
    include_str!("../prompts/system/checkpoint-active-notice.md");

/// System prompt fragment: `commit-message-system.md` (verbatim from omp).
pub const COMMIT_MESSAGE_SYSTEM: &str = include_str!("../prompts/system/commit-message-system.md");

/// System prompt fragment: `computer-safety.md` (verbatim from omp).
pub const COMPUTER_SAFETY: &str = include_str!("../prompts/system/computer-safety.md");

/// System prompt fragment: `custom-system-prompt.md` (verbatim from omp).
pub const CUSTOM_SYSTEM_PROMPT: &str = include_str!("../prompts/system/custom-system-prompt.md");

/// System prompt fragment: `date-cwd-reminder.md` (verbatim from omp).
pub const DATE_CWD_REMINDER: &str = include_str!("../prompts/system/date-cwd-reminder.md");

/// System prompt fragment: `eager-task.md` (verbatim from omp).
pub const EAGER_TASK: &str = include_str!("../prompts/system/eager-task.md");

/// System prompt fragment: `eager-todo.md` (verbatim from omp).
pub const EAGER_TODO: &str = include_str!("../prompts/system/eager-todo.md");

/// System prompt fragment: `empty-stop-retry.md` (verbatim from omp).
pub const EMPTY_STOP_RETRY: &str = include_str!("../prompts/system/empty-stop-retry.md");

/// System prompt fragment: `gemini-tool-call-reminder.md` (verbatim from omp).
pub const GEMINI_TOOL_CALL_REMINDER: &str =
    include_str!("../prompts/system/gemini-tool-call-reminder.md");

/// System prompt fragment: `interrupted-thinking.md` (verbatim from omp).
pub const INTERRUPTED_THINKING: &str = include_str!("../prompts/system/interrupted-thinking.md");

/// System prompt fragment: `irc-autoreply.md` (verbatim from omp).
pub const IRC_AUTOREPLY: &str = include_str!("../prompts/system/irc-autoreply.md");

/// System prompt fragment: `irc-incoming.md` (verbatim from omp).
pub const IRC_INCOMING: &str = include_str!("../prompts/system/irc-incoming.md");

/// System prompt fragment: `manual-continue.md` (verbatim from omp).
pub const MANUAL_CONTINUE: &str = include_str!("../prompts/system/manual-continue.md");

/// System prompt fragment: `mcp-xdev-guidance.md` (verbatim from omp).
pub const MCP_XDEV_GUIDANCE: &str = include_str!("../prompts/system/mcp-xdev-guidance.md");

/// System prompt fragment: `memory-consolidation-system.md` (verbatim from omp).
pub const MEMORY_CONSOLIDATION_SYSTEM: &str =
    include_str!("../prompts/system/memory-consolidation-system.md");

/// System prompt fragment: `memory-extraction-system.md` (verbatim from omp).
pub const MEMORY_EXTRACTION_SYSTEM: &str =
    include_str!("../prompts/system/memory-extraction-system.md");

/// System prompt fragment: `mid-run-todo-nudge.md` (verbatim from omp).
pub const MID_RUN_TODO_NUDGE: &str = include_str!("../prompts/system/mid-run-todo-nudge.md");

/// System prompt fragment: `omfg-user.md` (verbatim from omp).
pub const OMFG_USER: &str = include_str!("../prompts/system/omfg-user.md");

/// System prompt fragment: `orchestrate-notice.md` (verbatim from omp).
pub const ORCHESTRATE_NOTICE: &str = include_str!("../prompts/system/orchestrate-notice.md");

/// System prompt fragment: `plan-filename.md` (verbatim from omp).
pub const PLAN_FILENAME: &str = include_str!("../prompts/system/plan-filename.md");

/// System prompt fragment: `plan-mode-active.md` (verbatim from omp).
pub const PLAN_MODE_ACTIVE: &str = include_str!("../prompts/system/plan-mode-active.md");

/// System prompt fragment: `plan-mode-approved.md` (verbatim from omp).
pub const PLAN_MODE_APPROVED: &str = include_str!("../prompts/system/plan-mode-approved.md");

/// System prompt fragment: `plan-mode-compact-instructions.md` (verbatim from omp).
pub const PLAN_MODE_COMPACT_INSTRUCTIONS: &str =
    include_str!("../prompts/system/plan-mode-compact-instructions.md");

/// System prompt fragment: `plan-mode-reference.md` (verbatim from omp).
pub const PLAN_MODE_REFERENCE: &str = include_str!("../prompts/system/plan-mode-reference.md");

/// System prompt fragment: `plan-mode-subagent.md` (verbatim from omp).
pub const PLAN_MODE_SUBAGENT: &str = include_str!("../prompts/system/plan-mode-subagent.md");

/// System prompt fragment: `plan-mode-tool-decision-reminder.md` (verbatim from omp).
pub const PLAN_MODE_TOOL_DECISION_REMINDER: &str =
    include_str!("../prompts/system/plan-mode-tool-decision-reminder.md");

/// System prompt fragment: `plan-yolo-handoff.md` (verbatim from omp).
pub const PLAN_YOLO_HANDOFF: &str = include_str!("../prompts/system/plan-yolo-handoff.md");

/// System prompt fragment: `prewalk-checklist.md` (verbatim from omp).
pub const PREWALK_CHECKLIST: &str = include_str!("../prompts/system/prewalk-checklist.md");

/// System prompt fragment: `prewalk-continue.md` (verbatim from omp).
pub const PREWALK_CONTINUE: &str = include_str!("../prompts/system/prewalk-continue.md");

/// System prompt fragment: `prewalk-plan.md` (verbatim from omp).
pub const PREWALK_PLAN: &str = include_str!("../prompts/system/prewalk-plan.md");

/// System prompt fragment: `project-prompt.md` (verbatim from omp).
pub const PROJECT_PROMPT: &str = include_str!("../prompts/system/project-prompt.md");

/// System prompt fragment: `recap-user.md` (verbatim from omp).
pub const RECAP_USER: &str = include_str!("../prompts/system/recap-user.md");

/// System prompt fragment: `resolve-device-reminder.md` (verbatim from omp).
pub const RESOLVE_DEVICE_REMINDER: &str =
    include_str!("../prompts/system/resolve-device-reminder.md");

/// System prompt fragment: `rewind-report.md` (verbatim from omp).
pub const REWIND_REPORT: &str = include_str!("../prompts/system/rewind-report.md");

/// System prompt fragment: `side-channel-no-tools.md` (verbatim from omp).
pub const SIDE_CHANNEL_NO_TOOLS: &str = include_str!("../prompts/system/side-channel-no-tools.md");

/// System prompt fragment: `snapcompact-context-frames-note.md` (verbatim from omp).
pub const SNAPCOMPACT_CONTEXT_FRAMES_NOTE: &str =
    include_str!("../prompts/system/snapcompact-context-frames-note.md");

/// System prompt fragment: `snapcompact-context-stub.md` (verbatim from omp).
pub const SNAPCOMPACT_CONTEXT_STUB: &str =
    include_str!("../prompts/system/snapcompact-context-stub.md");

/// System prompt fragment: `snapcompact-system-frames-note.md` (verbatim from omp).
pub const SNAPCOMPACT_SYSTEM_FRAMES_NOTE: &str =
    include_str!("../prompts/system/snapcompact-system-frames-note.md");

/// System prompt fragment: `snapcompact-system-stub.md` (verbatim from omp).
pub const SNAPCOMPACT_SYSTEM_STUB: &str =
    include_str!("../prompts/system/snapcompact-system-stub.md");

/// System prompt fragment: `snapcompact-toolresult-note.md` (verbatim from omp).
pub const SNAPCOMPACT_TOOLRESULT_NOTE: &str =
    include_str!("../prompts/system/snapcompact-toolresult-note.md");

/// System prompt fragment: `speech-rewrite.md` (verbatim from omp).
pub const SPEECH_REWRITE: &str = include_str!("../prompts/system/speech-rewrite.md");

/// System prompt fragment: `subagent-async-pending.md` (verbatim from omp).
pub const SUBAGENT_ASYNC_PENDING: &str =
    include_str!("../prompts/system/subagent-async-pending.md");

/// System prompt fragment: `subagent-system-prompt.md` (verbatim from omp).
pub const SUBAGENT_SYSTEM_PROMPT: &str =
    include_str!("../prompts/system/subagent-system-prompt.md");

/// System prompt fragment: `subagent-user-prompt.md` (verbatim from omp).
pub const SUBAGENT_USER_PROMPT: &str = include_str!("../prompts/system/subagent-user-prompt.md");

/// System prompt fragment: `subagent-yield-reminder.md` (verbatim from omp).
pub const SUBAGENT_YIELD_REMINDER: &str =
    include_str!("../prompts/system/subagent-yield-reminder.md");

/// System prompt fragment: `system-prompt.md` (verbatim from omp).
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system/system-prompt.md");

/// System prompt fragment: `tan-context-switch.md` (verbatim from omp).
pub const TAN_CONTEXT_SWITCH: &str = include_str!("../prompts/system/tan-context-switch.md");

/// System prompt fragment: `task-label.md` (verbatim from omp).
pub const TASK_LABEL: &str = include_str!("../prompts/system/task-label.md");

/// System prompt fragment: `thinking-loop-redirect.md` (verbatim from omp).
pub const THINKING_LOOP_REDIRECT: &str =
    include_str!("../prompts/system/thinking-loop-redirect.md");

/// System prompt fragment: `title-marker-instruction.md` (verbatim from omp).
pub const TITLE_MARKER_INSTRUCTION: &str =
    include_str!("../prompts/system/title-marker-instruction.md");

/// System prompt fragment: `title-system.md` (verbatim from omp).
pub const TITLE_SYSTEM: &str = include_str!("../prompts/system/title-system.md");

/// System prompt fragment: `tool-call-loop-redirect.md` (verbatim from omp).
pub const TOOL_CALL_LOOP_REDIRECT: &str =
    include_str!("../prompts/system/tool-call-loop-redirect.md");

/// System prompt fragment: `ttsr-interrupt.md` (verbatim from omp).
pub const TTSR_INTERRUPT: &str = include_str!("../prompts/system/ttsr-interrupt.md");

/// System prompt fragment: `ttsr-tool-reminder.md` (verbatim from omp).
pub const TTSR_TOOL_REMINDER: &str = include_str!("../prompts/system/ttsr-tool-reminder.md");

/// System prompt fragment: `ultrathink-notice.md` (verbatim from omp).
pub const ULTRATHINK_NOTICE: &str = include_str!("../prompts/system/ultrathink-notice.md");

/// System prompt fragment: `unexpected-stop-classifier.md` (verbatim from omp).
pub const UNEXPECTED_STOP_CLASSIFIER: &str =
    include_str!("../prompts/system/unexpected-stop-classifier.md");

/// System prompt fragment: `unexpected-stop-retry.md` (verbatim from omp).
pub const UNEXPECTED_STOP_RETRY: &str = include_str!("../prompts/system/unexpected-stop-retry.md");

/// System prompt fragment: `vibe-mode-active.md` (verbatim from omp).
pub const VIBE_MODE_ACTIVE: &str = include_str!("../prompts/system/vibe-mode-active.md");

/// System prompt fragment: `web-search.md` (verbatim from omp).
pub const WEB_SEARCH: &str = include_str!("../prompts/system/web-search.md");

/// System prompt fragment: `workflow-notice.md` (verbatim from omp).
pub const WORKFLOW_NOTICE: &str = include_str!("../prompts/system/workflow-notice.md");

/// System prompt fragment: `xdev-mount-notice.md` (verbatim from omp).
pub const XDEV_MOUNT_NOTICE: &str = include_str!("../prompts/system/xdev-mount-notice.md");
