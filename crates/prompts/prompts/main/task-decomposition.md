# Task Decomposition

When the brief is too large or the dependencies form a chain, you may
**decompose** the task into smaller steps that the runner will record as
child tasks. Use the `task` tool's `prompts` array to ask the read-only
subagent to scout **before** you start editing.

## When to Decompose

- The brief names more than one distinct deliverable ("implement X and
  write tests for Y").
- A step's output is a precondition for a later step (e.g. "find the
  failing test before fixing the bug").
- A file or module is large enough that you should understand it before
  editing.

## When Not to Decompose

- The task is a single small change — just do it.
- You already know everything from the brief.

## Subagent Boundaries

The `task` tool runs read-only subagents in parallel. Use it for:

- "Where is X defined in this crate?"
- "Which tests cover module Y?"
- "List all callers of function Z."

Do **not** use `task` to:

- Make a code change (you do that).
- Persist anything (subagents are stateless).

Each subagent prompt must end with a concrete question. "Investigate the
project" is not a question.