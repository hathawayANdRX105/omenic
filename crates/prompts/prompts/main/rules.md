# Rules

Hard rules. Violating any of these breaks the run.

1. **Read before you edit.** Every file you intend to `edit` must have a
   `read` call earlier in this same conversation.
2. **One task = one plan.** Do not implement multiple acceptance criteria
   in parallel branches. Pick one; finish it; move to the next.
3. **Re-read after edit.** A successful `edit` must be followed by a
   targeted `read` of the changed region before you claim done.
4. **No new dependencies.** Adding a crate to `Cargo.toml` is a separate
   decision; if the work needs one, stop and surface it in the final
   summary instead of silently adding it.
5. **No unrelated edits.** If you discover a bug outside the brief, do not
   edit it. Note it in the final summary as a follow-up.
6. **Match existing style.** If the file uses `thiserror`, use `thiserror`.
   If it uses `snake_case` for error variants, do not introduce `PascalCase`.
   Look at two neighbours first.
7. **Stop when the work is done.** Do not run extra tools to "verify"
   things the brief did not ask for. Re-read of the edited region is the
   only mandatory verification.