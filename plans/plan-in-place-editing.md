# In-place editing

**Status:** Draft (2026-11-06).

Today glos is read-only. If you spot a typo while reviewing, you have to leave a comment about it
or switch to your editor. This plan lets you edit files in glos with Helix's keys and modes, and
save them with `:w`. It's the "Editing (after the MVP)" item in `completed/plan-overview.md`,
under "Built on Helix".

Edits in glos are for quick fixes. Bigger work stays in your editor, as the overview's goals say.

## What you can edit

- **Any file on disk.** That includes the changed files of the review and files opened for
  context with `space f`.
- **Only the new side, and only when it's the file on disk.** That's the case for a branch
  review, for "since your last review", and for `glos diff old new` (the `new` file or folder).
  When the new side is a commit (`glos diff A B`, or the per-commit scope), the text isn't on
  disk, and the editing keys say so in the status line and do nothing.
- **Only in the whole-file view.** The unified and side-by-side views are made of lines from both
  sides (`src/ui/diffview.rs`), so they stay read-only. Editing in the all-files mode of
  `completed/plan-all-files.md` is a later step, below.

## How it works

The text model is Helix's, from `helix-core`: a `Rope`, a `Selection` and `Transaction`s, with
`History` for undo. glos already holds a file as `src/ui/view.rs` `File` with a `Rope` and a
`Selection`, and its motions come from `crates/glos-helix/src/commands.rs`, ported from
`helix-term`. Editing adds three things.

1. **Editing commands, ported from `helix-term`.** Insert mode (`i`, `a`, `I`, `A`, `o`, `O`),
   `c`, `d`, `r`, `R`, `p`, `P`, `>`, `<`, `J`, `~`, `` ` ``, `` A-` ``, `C-a` and `C-x`, and
   undo and redo (`u`, `U`, `A-u`, `A-U`). They go into `glos-helix`, which stays the only crate
   that names Helix, as functions that return a `Transaction` rather than changing the text
   themselves. The comment box (`src/ui/comments.rs`) has its own small set of editing keys
   today. It should move onto the same commands, so the box and the file behave the same.
2. **A modified state on `File`.** The file keeps the text as last read from disk, a `History`,
   and a flag for unsaved changes. The status line shows `[+]` while there are unsaved changes,
   as Helix does. `:w` writes the file, `:w!` writes over a conflict (below), and `:wa` writes
   every modified file. `:q` refuses while any file is modified, and `:q!` quits anyway, as in
   Helix. Edits keep the gutter's diff marks current by diffing the old side against the edited
   text after each change, which `src/diff.rs` `hunks` already does fast enough.
3. **A new keymap context.** `Context::Insert` in `src/ui/keymap.rs`, with Helix's insert-mode
   keys. Ctrl-G shows them under the name `insert`. Typed characters go to `text()`, as they do
   in the comment box.

Threads move with your edits as they do with the agent's. `src/ui/mod.rs` `files_changed`
already maps them through each change with `reanchor::place`, and an edit in glos goes through
the same path when it's saved.

## When the agent changes a file you're editing

glos watches files (`Watch` in `src/ui/mod.rs`) and reads them again when they change. Today it
replaces the text and keeps your place with `view::keep_view`. A file with unsaved edits needs a
merge instead.

glos merges in three ways. The base is the text as last read from disk, yours is the edited
text, and theirs is the new text on disk. glos works out the hunks from the base to each with
`src/diff.rs` `hunks`. Their hunks that don't overlap yours apply to your text, and your file
stays modified. If any of their hunks overlaps one of yours, glos applies none of them, keeps
your text, and says in the status line that the file changed on disk. `:w` then refuses, and
`:w!` writes your text over theirs. `:reload` throws your edits away and reads the file again,
as in Helix.

Helix itself doesn't merge. It only warns when you write a file that changed on disk. glos
merges because an agent may be writing the same files while you review.

Your own save also triggers the watcher. `files_changed` must see that the text on disk is the
text you wrote, and do nothing.

## Telling the agent

The agent needs to know you edited its files, or it may write over your fix. When you submit, the
answer to `wait_for_review` (`src/mcp.rs`) lists the files you saved since the last round, with
their hunks. `glos show` lists them too.

## Keys

Helix wins any clash (see `DECISIONS.md`). Editing takes back `space c` and `space C` for
toggling comments in code, so the review commands move into the Review menu first, as step 3 of
`plan-review-menu.md` describes. `space r` is rename, which needs a language server, so it moves
with `plan-language-servers.md`.

## Steps

1. Port the editing commands into `glos-helix`, with Helix's unit tests for them.
2. Add the modified state, `:w`, `:w!`, `:wa`, `:q!`, `:reload` and `[+]`, with insert mode and
   the `insert` keymap context. Turn editing off where the new side isn't on disk.
3. Merge an agent's change into a modified file.
4. List your saved edits in `wait_for_review` and `glos show`.
5. Move the comment box onto the same editing commands.
6. Edit in the whole-file view of the all-files mode, now that `completed/plan-all-files.md` is built. An edit
   there goes to the file of the section the cursor is in.

## Tests

- Each ported command has Helix's own tests, run in `glos-helix`.
- Screen tests in `src/ui/tests.rs` for insert mode, undo and redo, `[+]`, and `:q` refusing.
- `:w` writes the file, and the watcher's event for it changes nothing on screen.
- The merge: an agent's change away from your edit applies, an overlapping one leaves your text
  and blocks `:w`, `:w!` writes yours, and `:reload` takes theirs.
- A thread on a line you edit moves with it, or becomes outdated after the quiet period.
- Editing keys do nothing in the unified and side-by-side views, or when the new side is a commit.
- `wait_for_review`'s answer lists the files you saved.
- The perf run in `src/ui/tests/perf.rs` gets a phase that types into a 4,000-line file.

## Open

- Should glos save on its own, for example when you leave insert mode, the way comment drafts
  are saved as you type? Helix doesn't by default. Saving on its own avoids losing work, but
  races with the agent more often.
- When the new side is a commit, should the editing keys open the file as it is on disk instead
  of doing nothing?

## Relevant code

- `crates/glos-helix/src/commands.rs` and `crates/glos-helix/src/lib.rs` `text`.
- `src/ui/view.rs` `File`, `keep_view` and `draw`.
- `src/ui/mod.rs` `files_changed`, `Watch`, the typable commands and `handle_key`.
- `src/ui/keymap.rs` `Context` and `normal_defs`.
- `src/ui/comments.rs` `CommentBox` and `BoxCmd`.
- `src/mcp.rs` `wait_for_review`, and `src/cli.rs` for `glos show`.
