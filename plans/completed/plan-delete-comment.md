# Deleting a comment: `space M`

**Status:** Implemented (2026-11-06) in PR #6.

You can delete a comment today, but only by a route nobody would guess: edit it with `space m`,
clear the text, and press `ret` twice. This adds a command for it, next to the keys that make
and edit comments.

Decided:

- **The key is `space M`.** It's the strongest form of `space m` (edit), following Helix's rule
  that Shift gives a stronger form of the same action. Deleting already works as saving an edit
  empty. `space d` would be the obvious mnemonic, but glos uses it for the thread picker, in
  place of Helix's diagnostics picker. `space A-c` would follow Helix's other rule, that Alt
  gives the opposite action, but Alt after `space` needs Option set as Meta in macOS terminals.
- **Only unsent comments can be deleted.** Once the agent has seen a comment, removing it would
  leave the agent's view of the thread out of step with yours. This is the rule
  `review::delete` already enforces.

## Where we are

- **`review::delete`** (`src/review.rs`) deletes an unsent comment, and its thread if nothing is
  left in it. It refuses one already sent: "that comment was already sent to the agent; edit it
  instead".
- **The only way to it in the UI** is through the comment box. `App::save_box`, given an empty
  `Place::Edit`, asks once ("Saving it empty deletes the comment: ret again deletes it, q keeps
  it") and then deletes.
- **`space m`** (`AppAction::EditComment`) edits your latest comment on the thread under the
  cursor (`Thread::latest_yours`), whether or not it was sent.

## Design

- **A new command,** `delete_comment`, described as "Delete your latest comment on the thread
  under the cursor, if it isn't sent". It's bound to `space M`, with a typed `:delete-comment`.
  The palette finds it by searching "delete".
- **It acts on the same comment `space m` would edit:** your latest on the thread under the
  cursor. If that comment was sent, nothing is deleted, even if an older one wasn't. Reaching
  past it would delete a comment you weren't looking at.
- **You confirm by pressing it again:**
  1. The first `space M` shows "space M again deletes your comment: «first line…»", and
     remembers which comment.
  2. A second `space M` straight after deletes it and says "Comment deleted".
  3. Any other key forgets it.

  This works like quitting with unsent comments, and like saving an edit empty: one keypress
  more, and no dialog.
- **The refusals,** shown in the statusline:
  - No thread under the cursor, or none of yours on it: "No comment of yours here".
  - Your latest was sent: "That comment was already sent to the agent: edit it with space m".
- **What follows:** if the thread has nothing left, it disappears, as `review::delete` already
  does. The unsent count in the statusline drops.
- **While the comment box is open,** `space M` is blocked like the other comment commands
  (`blocked_in_box`). Deleting from the box, by saving it empty, stays.
- **Docs:** add `space M` to the README's key table and to `../KEYMAP.md`'s glos-only keys.

## Steps

One PR on a `delete-comment` branch, as a single commit: the command, the key, the typed
command, the confirmation, the refusals, and the docs.

## Tests

- `space M` twice deletes your latest unsent comment, and a thread left empty disappears.
- `space M` and then any other key deletes nothing, and the next `space M` asks again.
- `:delete-comment` does the same as `space M`.
- If your latest comment is sent, it isn't deleted and the statusline says why, even when an
  older comment of yours is unsent.
- No comment of yours under the cursor: a status message, and nothing deleted.
- With the comment box open, `space M` is blocked.
- The `C-g` palette lists it, and searching "delete" finds it.

## Out of scope

- Withdrawing sent comments.
- Deleting an agent's comments from the UI.

## As built

The work follows the plan. Two details weren't spelled out in it.

"Any other key" means any key that finishes a command other than `space M`. The keys on the way
to a command don't count: `space`, the keys typed on the `:` line, and the palette. Pressing
`esc` to leave the space menu does count, so it forgets the delete.

The quote is the first 40 characters of the comment's first line. It ends in "…" when the
comment is longer than that.

Editing a sent comment with `space m` makes it unsent again, so `space M` can then delete it.
This was already true of `review::delete`, and this change keeps it.
