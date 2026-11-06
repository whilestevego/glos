# Re-anchoring that doesn't jump and survives small edits

**Status:** Draft (2026-11-06). This answers the overview's open question on re-anchoring rules,
in `completed/plan-overview.md`.

When the code under a thread changes, `src/reanchor.rs` `place` searches the whole file for the
exact text the thread was on, and picks the match nearest where it was. That has two problems.

- **A thread can jump.** If a thread is on a common line like `}` or `return Ok(())` and that line
  changes, the search can find another copy of the line far away, and the thread moves there.
- **A small edit outdates it.** If the agent fixes a typo inside the commented lines, the exact
  text is gone, and the thread becomes outdated even though it's about the same code.

## The rules

`place` keeps its first step, which maps the thread through the diff when the change doesn't touch
its lines. The search for moved text changes in three ways.

1. **A limited window.** A match counts only if it's within 200 lines of where the thread was
   mapped to, or if all of its context lines match too. Code that moved far away keeps its
   thread when its surroundings came with it.
2. **Context for short lines.** If the anchored text is short or common, at least one of its
   context lines must match too. Short means under 8 characters of non-whitespace. Common means it
   appears more than 3 times in the new file.
3. **Small edits keep the thread.** If no exact match is found, glos looks at the lines the diff
   maps the thread to. If they're similar enough to the anchored text, the thread stays on them.
   Similar enough means a similarity of 0.7 or more, by the measure `src/diff_words.rs`
   `similarity` already uses for pairing changed lines. The thread's anchor text is then updated
   to the new text.

The numbers are a first guess. The tests below fix them in place, so changing one is a deliberate
choice.

## Steps

1. Make `similarity` in `src/diff_words.rs` usable from `src/reanchor.rs`.
2. Add the three rules to `place`, and remove the `ponytail:` comment there that points at the
   open question.
3. Update the overview's open question in `completed/plan-overview.md` to point here.

## Tests

The overview says re-anchoring gets the most tests of anything in glos. Each rule gets tests in
`src/reanchor.rs` for both sides of its limit.

- A `}` thread whose line changes doesn't jump to a `}` 500 lines away, and becomes outdated.
- A block that moved 1,000 lines with its context keeps its thread.
- A short line with no matching context becomes outdated, and with one context line it moves.
- A typo fix inside a three-line thread keeps the thread, and updates its anchor text.
- A rewrite of the lines, with a similarity under 0.7, outdates the thread.
- The existing tests in `src/reanchor.rs` and `src/ui/tests.rs` still pass unchanged.

## Relevant code

- `src/reanchor.rs` `place` and `Anchored`.
- `src/diff_words.rs` `similarity`.
- `src/ui/mod.rs` `reanchor`, which stores an updated anchor.
