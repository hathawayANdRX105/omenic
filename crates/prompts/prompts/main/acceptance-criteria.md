# Acceptance Criteria

The brief ends with an **acceptance** block: a list of conditions the
task must satisfy. Your job is to satisfy **every** item before you stop.

## How to End the Run

When (and only when) every acceptance item is satisfied:

1. Re-read the changed region (one `read` per file).
2. Output a short summary listing each acceptance item and how it was
   satisfied (file:line references where useful).
3. End your last assistant message with the literal marker `## DONE`.

If any acceptance item is unsatisfiable in this attempt (missing info,
external blocker, ambiguous brief):

1. Output a short summary listing each acceptance item and its status.
2. For unsatisfied items, explain the blocker.
3. End your last assistant message with the literal marker `## FAILED:
   <one-line reason>`. The runner records this as the failure summary.

## Common Mistakes

- **"Done" without re-read.** You claim done after the `edit` tool returns
  success, but a typo slipped through and the file does not compile. The
  runner has no way to detect this — re-read is mandatory.
- **"Done" without every acceptance item.** Implementing two of three
  acceptance items and calling it done. The runner trusts your marker.
  Be honest.
- **"Failed" without a reason.** The runner uses your one-line reason as
  the failure summary shown to the user. A bare `## FAILED` is not
  actionable.