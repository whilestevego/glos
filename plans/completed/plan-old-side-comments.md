# Comments on removed lines

**Status:** Implemented (2026-11-09) in PR #17.

Comments attach only to lines of the new side. In the unified and side-by-side views you can see
a removed line, but you can't comment on it. If you want to ask why a check was deleted, you
have to comment on a nearby new line. This plan lets you comment on removed lines.

## What it looks like

- In the unified and side-by-side views, `' c` (`space c` before `completed/plan-review-menu.md`) on
  removed lines starts a thread on them. A selection that covers both removed and added lines
  goes on the new side, as today.
- The thread shows under its removed lines in the unified and side-by-side views.
- `glos show`, the MCP tools and the pickers write an old-side anchor as `path:-12` or
  `path:-12-14`, and say "removed lines" in words where there's room.

## How it works

- **The anchor gets a side.** `threads` in `src/store.rs` gets a column,
  `side TEXT NOT NULL DEFAULT 'new'`, in a new migration. Every existing thread stays on the new
  side. `src/review.rs` `Anchor` gets `side`, and its line numbers count lines of the old side
  when it's `old`.
- **It anchors to the old file's content.** The thread stores the old side's file hash, its
  lines and their context, as a new-side thread does.
- **It doesn't move.** The old side is the base, and it doesn't change while the agent works.
  The base can change when you switch scope or the base branch moves. In that case the thread is
  placed onto the new old side with `src/reanchor.rs` `place`, as a new-side thread is.
- **It goes outdated when the change is gone.** If its lines are no longer removed, because the
  agent put them back or the base changed, the thread becomes outdated after the quiet period, as
  a new-side thread does.
- **Agents see it.** `get_review` and `wait_for_review` in `src/mcp.rs` include the side in each
  thread's location, and `annotate` and `suggest` take an optional `side`.

## Steps

1. The migration, `Anchor::side`, and storing and loading it.
2. Commenting on removed lines in the unified and side-by-side views, and drawing the thread.
3. Placing and outdating old-side threads.
4. The MCP tools, the CLI and the pickers.

## Tests

- A migration test that an old database opens and every thread is on the new side.
- A comment on a removed line in the unified view is stored on the old side with the right line,
  and shows in the side-by-side view under the same line.
- An agent restoring the removed lines outdates the thread after the quiet period.
- `get_review` and `glos show --json` report the side.

## Open

- Settled: what should `space c` do on a selection that covers removed lines only on one side of
  the side-by-side view, and new lines on the other? Any new-side line puts the thread on the new
  side. A selection of filler rows only, beside removed lines, goes on the old side.

## As built

- `src/store.rs` migration v6 adds `threads.side`. `src/review.rs` has `Side`, `Anchor::side`
  and `Anchor::described`, which adds "(removed lines)" for `glos show` and the MCP tools.
- `src/ui/mod.rs` `can_comment` picks the side, and `old_line_at` reads the old side's line of a
  row, from the left column in side by side. `save_box` stores the old side's text with an
  old-side thread, read by `old_text`.
- `src/ui/comments.rs` `layout` lays out one side at a time. `threads_in` puts old-side threads
  under the row of their last removed line. `ThreadRow::old` and `Threads::old_marks` let
  `side_threads` draw them in the left column side by side, and leave blank rows on the right.
  This replaced `blank_threads`.
- Thread jumps (`]d`, `[d`) in `goto_thread_in` now work in the view's rows, so a thread on
  removed lines takes its place among the others. `select_anchor` and `anchor_rows` select a
  thread's lines on either side.
- `review::reanchor_side` places old-side threads against the base's text, and its `still`
  check treats a thread as gone when its lines are no longer removed. `App::reanchor` does this
  only in the base scope, because the other scopes have another old side.
- A deleted file's lines can now take comments. They're all removed lines, so the thread goes on
  the old side. The old "this file was deleted" refusal is gone.
- The MCP `now` note is left out for old-side threads, since removed lines aren't in the agent's
  files. An outdated one says the lines aren't removed any more.
- Not built: `annotate` and `suggest` don't take a side. `plans/DECISIONS.md` says why under
  "Comments on removed lines".
- A fold you make doesn't count the old-side threads it hides. Removed lines are changes, so
  only a fold you make can hide them.

## Relevant code

- `src/store.rs`, the schema and migrations.
- `src/review.rs` `Anchor`, `Thread` and their queries.
- `src/reanchor.rs`.
- `src/ui/mod.rs` the comment commands and `shown_anchor`, and `src/ui/comments.rs`.
- `src/ui/diffview.rs` `Row::old_line`.
- `src/mcp.rs` and `src/cli.rs`.
