# Comments on removed lines

**Status:** Draft (2026-11-06).

Comments attach only to lines of the new side. In the unified and side-by-side views you can see
a removed line, but you can't comment on it. If you want to ask why a check was deleted, you
have to comment on a nearby new line. This plan lets you comment on removed lines.

## What it looks like

- In the unified and side-by-side views, `space c` (`space v c` after `plan-review-menu.md`) on
  removed lines starts a thread on them. A selection that covers both removed and added lines
  goes on the new side, as today.
- The thread shows under its removed lines in the unified and side-by-side views. In the
  whole-file view it shows under the removed lines that `plan-removed-lines.md` draws there.
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
5. Drawing them in the whole-file view, once `plan-removed-lines.md` is built.

## Tests

- A migration test that an old database opens and every thread is on the new side.
- A comment on a removed line in the unified view is stored on the old side with the right line,
  and shows in the side-by-side view under the same line.
- An agent restoring the removed lines outdates the thread after the quiet period.
- `get_review` and `glos show --json` report the side.

## Open

- What should `space c` do on a selection that covers removed lines only on one side of the
  side-by-side view, and new lines on the other?

## Relevant code

- `src/store.rs`, the schema and migrations.
- `src/review.rs` `Anchor`, `Thread` and their queries.
- `src/reanchor.rs`.
- `src/ui/mod.rs` the comment commands and `shown_anchor`, and `src/ui/comments.rs`.
- `src/ui/diffview.rs` `Row::old_line`.
- `src/mcp.rs` and `src/cli.rs`.
